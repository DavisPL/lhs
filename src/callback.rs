extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

extern crate rustc_data_structures;
extern crate rustc_metadata;

use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::LocalDefId;
use rustc_interface::interface::Compiler;
use rustc_middle::mir::{Body, TerminatorKind};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::Span;

use crate::operand::get_operand_def_id;
use crate::parser::{
    ClosureInput, ClosureSink, ClosureSinks, MIRParser, ParamSink, ParamSinks, ReturnSources,
};
use crate::symexec::SymExecBool as SymExec;
use rustc_span::source_map::SourceMap;

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::{BufWriter, Write as _};
use std::path::Path;

/// Findings collected from one function: (source, sink function, forbidden
/// value/pattern) -> the spans where it is reachable. `source` is the taint
/// provenance, so a finding reads "source -> sink : value".
type DangerMap = HashMap<(String, String, String), Vec<(Span, String, Option<String>)>>;

/// Upper bound on per-parameter summary analyses, so a pathological crate with
/// thousands of sink-reachable functions cannot make the summary pass unbounded.
const MAX_SUMMARY_ANALYSES: u32 = 4000;

pub struct LCallback {}

impl LCallback {
    pub fn new() -> Self {
        LCallback {}
    }
}

impl Callbacks for LCallback {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        // Silence the default panic hook for the duration of our analysis: an
        // unsupported construct that panics deep inside is caught per-function
        // below and reported as "skipped", so we don't want rustc's backtrace
        // spewed into the build output. The real hook is restored afterwards.
        let prev_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));

        // Every analyzable body with MIR: free functions, methods, and closures.
        // Closures matter because archive extractors routinely do the write inside
        // a closure passed to an iterator adaptor, and a closure body is a distinct
        // MIR body never inlined into its parent here.
        let fns: Vec<(LocalDefId, DefKind, String)> = tcx
            .hir_body_owners()
            .filter(|d| {
                matches!(
                    tcx.def_kind(*d),
                    DefKind::Fn | DefKind::AssocFn | DefKind::Closure
                ) && tcx.is_mir_available(d.to_def_id())
            })
            .map(|d| (d, tcx.def_kind(d), tcx.def_path_str(d.to_def_id())))
            .collect();

        // Interprocedural taint summaries. Closure summaries first (which of a
        // closure's captured upvars / iterated element args route to a sink), so the
        // param-summary pass can see, at a closure's construction / driving adaptor,
        // that a tainted arg flows into the closure's inner sink — restoring the
        // transitive chain (e.g. uncbv extract_block -> block -> extract_file) that a
        // sink-in-a-closure would otherwise hide.
        let closure_sinks = compute_closure_sinks(tcx, &fns, &HashMap::new());
        // Return-origin summaries: which functions hand back data they read internally
        // (so `x = read(..)` taints `x` from the real read). Computed before the
        // param-summary and reporting passes, which both consult it.
        let return_sources = compute_return_sources(tcx, &fns, &closure_sinks);
        let param_sinks = compute_param_sinks(tcx, &fns, &closure_sinks, &return_sources);

        // Reporting pass: analyze every function, isolated so one panic can't
        // abort the whole compilation. Aggregate all findings, then write once.
        let mut all: DangerMap = HashMap::new();
        let mut functions: u64 = 0;
        let mut skipped: u64 = 0;
        for (local_def_id, dk, name) in &fns {
            functions += 1;
            let taint = seeds_for(*dk, tcx, *local_def_id);
            let ps = param_sinks.clone();
            let cs = closure_sinks.clone();
            let rs = return_sources.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mir_body = tcx.optimized_mir(*local_def_id);
                run_body(tcx, mir_body, ps, cs, rs, taint)
            }));
            match result {
                Ok(dm) => {
                    // Emit the enclosing (analyzed) function for any finding so the
                    // RustSec recall harness can match an advisory's named affected
                    // function; the CSV carries source/sink, not the containing fn.
                    if !dm.is_empty() {
                        println!("LHS_FINDING_FN {}", name);
                    }
                    merge_danger(&mut all, dm);
                }
                Err(_) => {
                    skipped += 1;
                    eprintln!("LHS: skipped `{}` (unsupported construct)", name);
                }
            }
        }

        std::panic::set_hook(prev_hook);

        let findings: usize = all.values().map(|v| v.len()).sum();
        if findings > 0 {
            write_danger_csv(tcx.sess.source_map(), &all, "dangerous_spans.csv");
        }
        println!(
            "LHS_STATS {{\"functions\":{},\"skipped\":{},\"findings\":{}}}",
            functions, skipped, findings
        );

        Compilation::Continue
    }
}

/// How to seed taint on entry when analyzing a function.
#[derive(Clone, Copy, PartialEq)]
enum TaintInit {
    /// No parameter tainted. Sources inside the body (env::args, archive-entry
    /// names, clap values, reader/parse reads, …) still taint as usual.
    None,
    /// Taint only parameter `i` — used to compute interprocedural summaries.
    Param(usize),
    /// Taint every path-carrying parameter: a public function's parameters are a
    /// library attack surface (an external caller passes untrusted data).
    PublicParams,
}

/// Decide how to seed a function based on its kind and visibility.
///
/// Closures get [`TaintInit::None`] — NO synthetic capture/arg seed. Their real
/// flow is recovered by closure summaries ([`compute_closure_sinks`]) consulted at
/// the construction site (captured upvar) and the driving iterator adaptor / Fn::call
/// (element arg), so a finding only fires when a REAL source reaches the closure.
fn seeds_for<'tcx>(dk: DefKind, tcx: TyCtxt<'tcx>, def_id: LocalDefId) -> TaintInit {
    match dk {
        DefKind::Closure => TaintInit::None,
        _ if crate::settings::PUBLIC_PARAMS_ENABLED
            && tcx.visibility(def_id.to_def_id()).is_public() =>
        {
            TaintInit::PublicParams
        }
        _ => TaintInit::None,
    }
}

/// Whether parameter `i` (local `_(i+1)`) can carry a path/command string. Scalars
/// and fieldless enums cannot, so seeding them as sources would be pure noise.
fn param_can_carry_taint<'tcx>(mir_body: &Body<'tcx>, i: usize) -> bool {
    let local = rustc_middle::mir::Local::from_usize(i + 1);
    if local.as_usize() >= mir_body.local_decls.len() {
        return false;
    }
    let ty = mir_body.local_decls[local].ty.peel_refs();
    if ty.is_integral() || ty.is_bool() || ty.is_char() || ty.is_floating_point() {
        return false;
    }
    if let TyKind::Adt(adt, _) = ty.kind() {
        if adt.is_enum() && adt.variants().iter().all(|v| v.fields.is_empty()) {
            return false;
        }
    }
    true
}

/// Best-effort source-level name of parameter `i` (local `_(i+1)`), from MIR
/// debug info — so a finding can say `param[1] (dest)` instead of just `param[1]`.
fn param_name<'tcx>(mir_body: &Body<'tcx>, i: usize) -> Option<String> {
    let target = rustc_middle::mir::Local::from_usize(i + 1);
    for dbg in &mir_body.var_debug_info {
        if let rustc_middle::mir::VarDebugInfoContents::Place(p) = &dbg.value {
            if p.local == target && p.projection.is_empty() {
                return Some(dbg.name.to_string());
            }
        }
    }
    None
}

/// Best-effort "file:line" of parameter `i`'s declaration (from its MIR debug
/// info span) — the first hop's location in a taint path.
fn param_loc<'tcx>(tcx: TyCtxt<'tcx>, mir_body: &Body<'tcx>, i: usize) -> Option<String> {
    let target = rustc_middle::mir::Local::from_usize(i + 1);
    for dbg in &mir_body.var_debug_info {
        if let rustc_middle::mir::VarDebugInfoContents::Place(p) = &dbg.value {
            if p.local == target && p.projection.is_empty() {
                let s = tcx
                    .sess
                    .source_map()
                    .span_to_diagnostic_string(dbg.source_info.span);
                let mut it = s.splitn(3, ':');
                return match (it.next(), it.next()) {
                    (Some(f), Some(l)) => Some(format!("{f}:{l}")),
                    _ => Some(s),
                };
            }
        }
    }
    None
}

/// Declare a Z3 variable for each local of a modeled type. Parameters are locals
/// `_1..=arg_count`. String-like std wrappers (`PathBuf`/`String`/`OsString`, and
/// references to them or to `str`) become uninterpreted strings so path/command
/// sinks can reason about them.
fn setup_ev<'ctx, 'tcx>(
    tcx: TyCtxt<'tcx>,
    mir_body: &Body<'tcx>,
    ctx: &'ctx z3::Context,
) -> SymExec<'ctx> {
    let mut ev = SymExec::new(ctx);
    for (local, local_decl) in mir_body.local_decls.iter_enumerated() {
        let name = local.as_usize().to_string();
        match local_decl.ty.kind() {
            TyKind::Int(_) => ev.create_int(&name),
            TyKind::Uint(_) => ev.create_int(&name),
            TyKind::Str => ev.create_uninterpreted_string(&name),
            TyKind::Char => ev.create_uninterpreted_string(&name),
            TyKind::Bool => ev.create_uninterpreted_bool(&name),
            TyKind::Ref(_, inner, _) => {
                // Model `&str` and references to string-like wrappers (`&PathBuf`,
                // `&String`, `&OsString`) as strings — otherwise a `&PathBuf`
                // parameter (a very common `join` receiver) carries no value.
                if is_string_like(tcx, *inner) {
                    ev.create_uninterpreted_string(&name);
                }
            }
            TyKind::Adt(adt_def, _) => {
                let p = tcx.def_path_str(adt_def.did());
                if p.ends_with("path::PathBuf")
                    || p.ends_with("string::String")
                    || p.ends_with("ffi::os_str::OsString")
                {
                    ev.create_uninterpreted_string(&name);
                }
            }
            _ => {}
        }
    }
    ev
}

/// Whether a type is a string/path we model as a Z3 string (peeling references).
fn is_string_like<'tcx>(tcx: TyCtxt<'tcx>, ty: rustc_middle::ty::Ty<'tcx>) -> bool {
    let ty = ty.peel_refs();
    if ty.is_str() {
        return true;
    }
    if let TyKind::Adt(adt_def, _) = ty.kind() {
        let p = tcx.def_path_str(adt_def.did());
        return p.ends_with("path::PathBuf")
            || p.ends_with("path::Path")
            || p.ends_with("string::String")
            || p.ends_with("ffi::os_str::OsString")
            || p.ends_with("ffi::os_str::OsStr");
    }
    false
}

/// Run the analysis over one function body with the given taint seeding and
/// interprocedural summaries, returning its findings.
fn run_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    mir_body: &'tcx Body<'tcx>,
    param_sinks: ParamSinks,
    closure_sinks: ClosureSinks,
    return_sources: ReturnSources,
    taint: TaintInit,
) -> DangerMap {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let mut ev = setup_ev(tcx, mir_body, &ctx);

    // Seed taint and record each seeded value's provenance so findings can name
    // where the value came from (e.g. `param[1] (dest) @ lib.rs:98`) — the `@ line`
    // is the parameter's declaration site, the first hop of the taint path.
    let seed = |ev: &mut SymExec, i: usize, kind: &str| {
        let key = (i + 1).to_string();
        ev.set_taint(&key, true);
        let label = match param_name(mir_body, i) {
            Some(n) => format!("{}[{}] ({})", kind, i, n),
            None => format!("{}[{}]", kind, i),
        };
        let origin = match param_loc(tcx, mir_body, i) {
            Some(loc) => format!("{label} @ {loc}"),
            None => label,
        };
        ev.set_taint_origin(&key, &origin);
    };
    match taint {
        TaintInit::None => {}
        TaintInit::Param(i) => {
            if i < mir_body.arg_count && param_can_carry_taint(mir_body, i) {
                seed(&mut ev, i, "param");
            }
        }
        TaintInit::PublicParams => {
            for i in 0..mir_body.arg_count {
                if param_can_carry_taint(mir_body, i) {
                    seed(&mut ev, i, "param");
                }
            }
        }
    }

    let mut parser = MIRParser::new(tcx, mir_body, ev)
        .with_param_sinks(param_sinks)
        .with_closure_sinks(closure_sinks)
        .with_return_sources(return_sources);
    // In LIBRARY mode a public function's parameter IS an untrusted source (a caller
    // can pass anything, incl. `..`), so a `pub param -> fs sink` flow is a real
    // finding even when the function never reads a file — e.g. a `write(path, data)` /
    // `remove(path)` helper. So there is no reader-provenance gate: every public-param
    // flow into a sink is reported. (The trade-off is that plain path-taking libraries
    // — fs_extra, tempfile, … — surface many findings; those are valid "a caller
    // passing an untrusted path here causes traversal" warnings under this threat
    // model. Turn PUBLIC_PARAMS_ENABLED off to analyze a binary, where params are not
    // an attack surface.)
    parser.parse()
}

/// Does this function body directly call a dangerous sink? (Cheap syntactic scan
/// used to scope the summary pass to functions that could possibly matter.)
fn calls_sink<'tcx>(tcx: TyCtxt<'tcx>, mir_body: &Body<'tcx>) -> bool {
    for bb in mir_body.basic_blocks.iter() {
        if let TerminatorKind::Call { func, .. } = &bb.terminator().kind {
            if let Some(did) = get_operand_def_id(func) {
                if crate::matching::is_sink_func(&tcx.def_path_str(did)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Does this function body directly call an untrusted-input SOURCE? (Cheap syntactic
/// scan used to scope the return-origin summary pass to functions that actually read.)
fn calls_source<'tcx>(tcx: TyCtxt<'tcx>, mir_body: &Body<'tcx>) -> bool {
    for bb in mir_body.basic_blocks.iter() {
        if let TerminatorKind::Call { func, .. } = &bb.terminator().kind {
            if let Some(did) = get_operand_def_id(func) {
                if crate::matching::is_source_func(&tcx.def_path_str(did)) {
                    return true;
                }
            }
        }
    }
    false
}

/// Compute return-origin summaries (see [`ReturnSources`]): for each non-closure function
/// that directly calls a SOURCE, analyze it with NO parameter tainted (baseline — only
/// in-body reads fire) and, if its RETURN value carries a real read/source origin, record
/// that origin. Consulted at call sites so a caller's `let x = f(..)` is tainted from the
/// real read INSIDE `f`, letting a sink in a different function trace back to it.
fn compute_return_sources<'tcx>(
    tcx: TyCtxt<'tcx>,
    fns: &[(LocalDefId, DefKind, String)],
    closure_sinks: &ClosureSinks,
) -> ReturnSources {
    let mut out: ReturnSources = HashMap::new();
    let mut analyses: u32 = 0;
    for (def_id, dk, path) in fns {
        if matches!(dk, DefKind::Closure) {
            continue;
        }
        if analyses >= MAX_SUMMARY_ANALYSES {
            break;
        }
        let scan = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            calls_source(tcx, tcx.optimized_mir(*def_id))
        }));
        if !matches!(scan, Ok(true)) {
            continue;
        }
        analyses += 1;
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            return_origin_of_body(tcx, tcx.optimized_mir(*def_id), closure_sinks.clone())
        }));
        if let Ok(Some(origin)) = res {
            out.insert(path.clone(), origin);
        }
    }
    out
}

/// Analyze a body with no parameter tainted and return the real read/source origin that
/// reached its return value, if any.
fn return_origin_of_body<'tcx>(
    tcx: TyCtxt<'tcx>,
    mir_body: &'tcx Body<'tcx>,
    closure_sinks: ClosureSinks,
) -> Option<String> {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let ev = setup_ev(tcx, mir_body, &ctx);
    let mut parser = MIRParser::new(tcx, mir_body, ev).with_closure_sinks(closure_sinks);
    let _ = parser.parse();
    parser.return_origin.clone()
}

/// Run a body with an explicit set of tainted place keys (each with an origin
/// label), returning the raw findings (no reader-provenance gate — this is a probe,
/// like a `Param` seed). Used to summarize a CLOSURE by seeding one input at a time:
/// an argument local (`"2"`) or a captured upvar (`"1.f0"` and `"1*.f0"`, covering
/// by-value and by-ref environments).
fn run_body_keyseed<'tcx>(
    tcx: TyCtxt<'tcx>,
    mir_body: &'tcx Body<'tcx>,
    param_sinks: ParamSinks,
    closure_sinks: ClosureSinks,
    keys: &[(String, String)],
) -> DangerMap {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let mut ev = setup_ev(tcx, mir_body, &ctx);
    for (k, origin) in keys {
        ev.set_taint(k, true);
        ev.set_taint_origin(k, origin);
    }
    let mut parser = MIRParser::new(tcx, mir_body, ev)
        .with_param_sinks(param_sinks)
        .with_closure_sinks(closure_sinks)
        // Data-flow only: a closure's own captured-reader / branch ambient taint must
        // not pollute the baseline, or it masks which INPUT routes to the sink.
        .with_suppress_path_taint(true);
    parser.parse()
}

/// Max captured upvars to probe per closure (fields of the environment `_1`).
const MAX_UPVAR_PROBE: usize = 12;

/// Compute CLOSURE summaries: for each closure that directly calls a sink, determine
/// which of its INPUTS — a captured upvar (`Upvar(k)`) or an argument local such as
/// an iterated element (`Arg(l)`) — routes a tainted value into that sink. Consulted
/// at the closure's construction site / driving iterator-adaptor so a real source
/// reaching a captured value or an iterated element fires the inner sink WITHOUT the
/// synthetic capture/arg seed. Mirrors [`compute_param_sinks`]'s baseline-diff.
fn compute_closure_sinks<'tcx>(
    tcx: TyCtxt<'tcx>,
    fns: &[(LocalDefId, DefKind, String)],
    param_sinks: &ParamSinks,
) -> ClosureSinks {
    let mut out: ClosureSinks = HashMap::new();
    let mut analyses: u32 = 0;
    for (def_id, dk, _path) in fns {
        if !matches!(dk, DefKind::Closure) {
            continue;
        }
        if analyses >= MAX_SUMMARY_ANALYSES {
            break;
        }
        let scan = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            calls_sink(tcx, tcx.optimized_mir(*def_id))
        }));
        if !matches!(scan, Ok(true)) {
            continue;
        }
        let baseline_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_body_keyseed(tcx, tcx.optimized_mir(*def_id), param_sinks.clone(), HashMap::new(), &[])
        }));
        let Ok(baseline) = baseline_res else { continue };
        let baseline_keys: HashSet<(String, String, Span)> = danger_keys(&baseline);

        let arg_count = tcx.optimized_mir(*def_id).arg_count;
        let record = |input: ClosureInput, dm: &DangerMap, out: &mut ClosureSinks| {
            for ((_s, func, val), spans) in dm {
                for (sp, _c, ex) in spans {
                    if baseline_keys.contains(&(func.clone(), val.clone(), *sp)) {
                        continue;
                    }
                    out.entry(def_id.to_def_id()).or_default().push(ClosureSink {
                        input,
                        sink_func: func.clone(),
                        forbidden_val: val.clone(),
                        span: *sp,
                        example: ex.clone(),
                    });
                }
            }
        };

        // Argument locals `_2..=_arg_count` (e.g. an iterator element). Local `_1` is
        // the captured environment, probed as upvars below, not here.
        for l in 2..=arg_count {
            if analyses >= MAX_SUMMARY_ANALYSES {
                break;
            }
            analyses += 1;
            let keys = [(l.to_string(), "closure element".to_string())];
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_body_keyseed(tcx, tcx.optimized_mir(*def_id), param_sinks.clone(), HashMap::new(), &keys)
            }));
            if let Ok(dm) = res {
                record(ClosureInput::Arg(l), &dm, &mut out);
            }
        }

        // Captured upvars `(*_1).k` / `_1.k` — probe each field of the environment.
        for k in 0..MAX_UPVAR_PROBE {
            if analyses >= MAX_SUMMARY_ANALYSES {
                break;
            }
            analyses += 1;
            let keys = [
                (format!("1.f{k}"), "captured value".to_string()),
                (format!("1*.f{k}"), "captured value".to_string()),
            ];
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_body_keyseed(tcx, tcx.optimized_mir(*def_id), param_sinks.clone(), HashMap::new(), &keys)
            }));
            if let Ok(dm) = res {
                record(ClosureInput::Upvar(k), &dm, &mut out);
            }
        }
    }
    out
}

/// Compute interprocedural summaries: for each non-closure function that calls a
/// sink directly, determine which of its parameters route a tainted value into
/// that sink (by comparing a per-parameter tainted run against an untainted
/// baseline, so only param-attributable sinks are recorded).
fn compute_param_sinks<'tcx>(
    tcx: TyCtxt<'tcx>,
    fns: &[(LocalDefId, DefKind, String)],
    closure_sinks: &ClosureSinks,
    return_sources: &ReturnSources,
) -> ParamSinks {
    let mut summaries: ParamSinks = HashMap::new();
    let mut analyses: u32 = 0;

    for (def_id, dk, path) in fns {
        // Closures are invoked by std adaptors, not by call sites we analyze, so
        // a closure summary would never be consulted — skip them here (their
        // inputs are seeded directly in the reporting pass).
        if matches!(dk, DefKind::Closure) {
            continue;
        }
        if analyses >= MAX_SUMMARY_ANALYSES {
            break;
        }

        let scan = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mir = tcx.optimized_mir(*def_id);
            calls_sink(tcx, mir)
        }));
        if !matches!(scan, Ok(true)) {
            continue;
        }

        // Baseline: findings with no parameter tainted. Anything here is caused
        // by an in-body source, not a parameter, so it must not be attributed to
        // one (that would make every caller passing any tainted arg fire).
        let baseline_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_body(tcx, tcx.optimized_mir(*def_id), HashMap::new(), closure_sinks.clone(), return_sources.clone(), TaintInit::None)
        }));
        let Ok(baseline) = baseline_res else { continue };
        let baseline_keys: HashSet<(String, String, Span)> = danger_keys(&baseline);

        let arg_count = tcx.optimized_mir(*def_id).arg_count;
        for i in 0..arg_count {
            if analyses >= MAX_SUMMARY_ANALYSES {
                break;
            }
            let mir = tcx.optimized_mir(*def_id);
            if !param_can_carry_taint(mir, i) {
                continue;
            }
            analyses += 1;
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                run_body(tcx, tcx.optimized_mir(*def_id), HashMap::new(), closure_sinks.clone(), return_sources.clone(), TaintInit::Param(i))
            }));
            let Ok(dm) = res else { continue };
            for ((_source, func, val), spans) in &dm {
                for (sp, _chain, ex) in spans {
                    let key = (func.clone(), val.clone(), *sp);
                    if baseline_keys.contains(&key) {
                        continue; // caused by an in-body source, not this param
                    }
                    summaries.entry(path.clone()).or_default().push(ParamSink {
                        param_idx: i,
                        sink_func: func.clone(),
                        forbidden_val: val.clone(),
                        span: *sp,
                        example: ex.clone(),
                    });
                }
            }
        }
    }
    summaries
}

/// Flatten a DangerMap into a set of (sink, value, span) identity keys, ignoring
/// the source label — a sink that fires in the untainted baseline must not be
/// attributed to any parameter, regardless of how its source is labeled.
fn danger_keys(map: &DangerMap) -> HashSet<(String, String, Span)> {
    let mut out = HashSet::new();
    for ((_source, func, val), spans) in map {
        for (sp, _chain, _ex) in spans {
            out.insert((func.clone(), val.clone(), *sp));
        }
    }
    out
}

/// Merge one function's findings into the crate-wide map.
fn merge_danger(all: &mut DangerMap, dm: DangerMap) {
    for (k, spans) in dm {
        all.entry(k).or_default().extend(spans);
    }
}

/// Quote a CSV field if it contains a comma, quote, or newline (turbofish paths
/// and generic sink names can contain commas), escaping embedded quotes.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write all findings to CSV once, de-duplicating identical (source, sink, value,
/// span) rows the summary/reporting passes can produce.
/// Schema: `source,sink,value,example,attack,dup,span,chain`:
///  - `value`  = the forbidden PATTERN that was checked (e.g. `*..*` = "could contain
///               a `..` path component").
///  - `example`= a CONCRETE value the tainted argument could take at the sink, from
///               Z3's model — "what value can it actually be here" (blank for
///               summary-based findings, which don't re-run the solver).
///  - `attack` = the attack class this finding represents (e.g. `path-traversal`,
///               `command-injection`), derived from the (sink, value) pair.
///  - `dup`   = `unique` if this (source, sink) pair occurs once, else `repeat(N)`
///              where N is how many rows share the same source→sink (so you can
///              spot the same flow reported at several sites/values).
///  - `chain` is the hop-by-hop taint path source→sink.
fn write_danger_csv(sm: &SourceMap, map: &DangerMap, path: &str) {
    // First gather the de-duplicated rows so we can tally repeats across groups.
    // (source, sink, value, example, span, chain)
    let mut rows: Vec<(String, String, String, String, String, String)> = Vec::new();
    for ((source, func, val), spans) in map {
        let mut seen: HashSet<String> = HashSet::new();
        for (sp, chain, example) in spans {
            let span_str = sm.span_to_diagnostic_string(*sp);
            if !seen.insert(span_str.clone()) {
                continue; // duplicate location for this (source, sink, value)
            }
            rows.push((
                source.clone(),
                func.clone(),
                val.clone(),
                example.clone().unwrap_or_default(),
                span_str,
                chain.clone(),
            ));
        }
    }

    // How many rows share each (source, sink) pair — the repeat label.
    let mut ss_count: HashMap<(&str, &str), usize> = HashMap::new();
    for (source, sink, ..) in &rows {
        *ss_count.entry((source, sink)).or_insert(0) += 1;
    }

    let path = Path::new(path);
    let file_exists = path.exists();
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(path) {
        let mut writer = BufWriter::new(file);
        if !file_exists {
            let _ = writeln!(writer, "source,sink,value,example,attack,dup,span,chain");
        }
        for (source, sink, val, example, span_str, chain) in &rows {
            let n = ss_count[&(source.as_str(), sink.as_str())];
            let dup = if n > 1 {
                format!("repeat({n})")
            } else {
                "unique".to_string()
            };
            let attack = crate::matching::attack_class(sink, val);
            let _ = writeln!(
                writer,
                "{},{},{},{},{},{},{},{}",
                csv_field(source),
                csv_field(sink),
                csv_field(val),
                csv_field(example),
                csv_field(attack),
                dup,
                csv_field(span_str),
                csv_field(chain)
            );
        }
    }
}
