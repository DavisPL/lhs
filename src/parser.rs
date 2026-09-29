use rustc_middle::mir::{
    AggregateKind, BasicBlock, BinOp, Body, CallSource, Operand, Place, ProjectionElem, Rvalue,
    StatementKind, SwitchTargets, TerminatorKind, UnOp, UnwindAction,
};

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{data_structures::IndexMap, IntTy, Ty, TyCtxt, TyKind, UintTy};

use rustc_span::Span;

use z3::ast::Ast;
use z3::SatResult;

use crate::operand::{
    get_operand_const_string, get_operand_def_id, get_operand_local, get_operand_span,
};
// TODO: update to use SOURCE_FUNCTIONS and SINK_FUNCTION_ARGS
use crate::settings::{ENV_VARS_TO_TRACK, MAX_LOOP_ITER, SINK_FUNCTION_ARGS, SOURCE_FUNCTIONS};
use crate::symexec::SymExecBool as SymExec;

use std::collections::{HashMap, HashSet};

use crate::handlers::{
    generic_string_handler, handle_deref_generic, handle_deref_mut,
    handle_fmt_arg_new_display,
    handle_fmt_arguments_new_v1, handle_fmt_format, handle_from_trait, handle_generic_source,
    handle_path_join, handle_path_new, handle_path_to_path_buf, handle_pathbuf_from,
    handle_pathbuf_push, handle_read_into_buf, handle_result_unwrap_or_default, handle_str_contains,
    handle_str_ends_with, handle_str_starts_with, handle_string_from, handle_string_from_utf8,
    handle_string_from_utf8_lossy,
};

#[derive(Clone, Copy, Debug)]
pub struct SinkInformation {
    pub arg_idx: usize,
    pub forbidden_val: &'static str,
}

/// The bundle of value-provenance facts LHS carries along every dataflow edge: the
/// taint flag, the taint ORIGIN (where the value came from), and the entry-vs-base
/// `composed` bit (a base with a path component joined/pushed on). Propagating them as
/// one unit (via `MIRParser::value_facts_of` / `apply_value_facts` /
/// `propagate_value_facts`) is what keeps a fact from being moved at one site but
/// forgotten at another; a future fact (e.g. a `confined` no-`..` marker) is added here
/// and in those few methods, not at every call site.
#[derive(Default, Clone)]
struct ValueFacts {
    tainted: bool,
    origin: Option<String>,
    composed: bool,
}

/// One interprocedural summary entry: "if parameter `param_idx` of this function
/// is tainted on entry, a tainted value reaches sink `sink_func` (checked against
/// `forbidden_val`) at `span`". Computed by a summary pass and consulted at call
/// sites so a tainted argument flowing into a callee's sink is reported.
#[derive(Clone)]
pub struct ParamSink {
    pub param_idx: usize,
    pub sink_func: String,
    pub forbidden_val: String,
    pub span: Span,
    /// A concrete example value (Z3 witness) the tainted value could take at the sink,
    /// captured when the callee's own sink check fired — carried to the call site so a
    /// summary-based finding also shows "what value it can be."
    pub example: Option<String>,
}

/// Interprocedural summaries keyed by the callee's fully-qualified path.
pub type ParamSinks = HashMap<String, Vec<ParamSink>>;

/// Return-origin summaries: callee path -> the origin label to attribute to its RETURN
/// value, for functions that INTERNALLY read untrusted input (a registered source /
/// reader) and hand back data derived from it (e.g. `PakFile::read` -> reads the file,
/// returns the parsed archive). The mirror image of [`ParamSinks`] ("dirty flows IN"):
/// here "dirty comes OUT". Consulted at a call `x = callee(..)` so `x` is tainted with
/// the real read origin, letting a later sink in a DIFFERENT function be traced back to
/// the actual archive/file read rather than a rough public-param guess.
pub type ReturnSources = HashMap<String, String>;

/// Which INPUT of a closure a summary entry is about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClosureInput {
    /// Field `k` of the captured environment (`(*_1).k` / `_1.k`) — a captured value.
    Upvar(usize),
    /// Argument local `l` (`_2..=_arg_count`) — e.g. an iterator element the closure
    /// is applied to by `map`/`for_each`/`try_for_each`.
    Arg(usize),
}

/// A sink reachable from a closure INPUT — the closure analogue of [`ParamSink`].
/// Consulted at the closure's CONSTRUCTION site (for `Upvar`, when the captured
/// operand is tainted) and at the iterator-adaptor / `Fn::call` that drives it (for
/// `Arg`, when the iterator receiver is tainted). This is what lets a REAL source's
/// taint reach a sink hidden inside a closure body WITHOUT the synthetic capture seed.
#[derive(Clone)]
pub struct ClosureSink {
    pub input: ClosureInput,
    pub sink_func: String,
    pub forbidden_val: String,
    pub span: Span,
    /// A concrete example value (Z3 witness), as in [`ParamSink::example`].
    pub example: Option<String>,
}

/// Closure summaries keyed by the closure's `DefId` (recovered from a closure-typed
/// operand at the driving call / construction site).
pub type ClosureSinks = HashMap<DefId, Vec<ClosureSink>>;

pub struct MIRParser<'tcx, 'mir, 'ctx>
where
    'mir: 'tcx,
{
    pub(crate) mir_body: &'mir Body<'tcx>,
    pub curr: SymExec<'ctx>,

    // Stack for iterative basic block processing
    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,

    // Loop handling: track how many times we've visited each basic block
    visit_counts: HashMap<BasicBlock, u32>,

    // Collection of all dangerous write locations found during analysis, keyed by
    // (source, sink function, forbidden value/pattern) -> (span, taint-path chain).
    // `source` is the first hop of the chain (where the value came from); `chain`
    // is the full hop-by-hop taint path from source to (just before) the sink.
    // (source, sink fn, forbidden pattern) -> [(sink span, taint-path chain, example
    // value)]. `example` is a concrete string the tainted arg could take at the sink,
    // from Z3's model (None for summary-based findings, which don't re-run the solver).
    dangerous_spans: HashMap<(String, String, String), Vec<(Span, String, Option<String>)>>,
    pub(crate) aliases: HashMap<String, String>, // Hashmap for aliases check

    // registry of “interesting” callees → handler
    handlers: IndexMap<String, (CallHandler<'tcx, 'mir, 'ctx>, Vec<SinkInformation>)>,
    pub(crate) tcx: TyCtxt<'tcx>,

    // Interprocedural summaries: callee path -> which params route into a sink.
    // Consulted at call sites so a tainted arg flowing into a callee sink fires.
    pub(crate) param_sinks: ParamSinks,

    // Closure summaries: closure DefId -> which of its inputs (captured upvar / arg
    // element) route into a sink. Consulted at the closure's construction site and at
    // the iterator-adaptor / Fn::call that drives it, so a real source reaching a
    // captured value or an iterated element fires the closure's inner sink.
    pub(crate) closure_sinks: ClosureSinks,

    // When true, ignore ambient `path_taint` in `operand_tainted` — only real
    // data-flow taint decides. Used by the closure input-probe pass so a closure's
    // own in-body reader/branch ambient taint doesn't pollute the baseline and mask
    // which INPUT (upvar/element) actually routes to the sink by data-flow.
    pub(crate) suppress_path_taint: bool,

    // Set true once this function's body touches an untrusted archive/stream
    // boundary: an archive-entry-name accessor, a file/socket read, or a
    // reader/deserialize consumption. Gates the public-parameter finding model —
    // a public fn that never reads such data is a plain path-taking API, so its
    // `param -> fs-sink` flows are the benign FP floor (dropped in `run_body`).
    pub(crate) touched_reader: bool,

    // Entry-vs-base bookkeeping (populated during the pass, resolved in
    // `filter_base_sinks` after `parse`):
    //  - `bare_target_origins`: origin heads that reach a WHOLE-TARGET sink
    //    (`File::create`/`write`/`open`/`remove`/…) with a BARE (non-composed) arg —
    //    proof that the value is a per-entry TARGET path, not just an output dir.
    //  - `pending_base`: base-position (`create_dir`/`copy`) findings on a bare,
    //    non-real-source value — DEFERRED, then promoted iff their origin turns out
    //    to be a bare-target origin (else dropped as the trusted-output-dir class).
    pub(crate) bare_target_origins: std::collections::HashSet<String>,
    pending_base: Vec<(String, String, String, Span, String, Option<String>)>,

    // Return-origin summaries (see [`ReturnSources`]): callee path -> origin to give its
    // return value, so `x = read_archive(..)` marks `x` dirty from the real read.
    pub(crate) return_sources: ReturnSources,
    // Set while analyzing a function whose RETURN local (`_0`) ends up tainted with a
    // real read/source origin — captured at `Return` and read back by
    // `compute_return_sources` to build this function's return-origin summary.
    pub(crate) return_origin: Option<String>,
}

impl<'tcx, 'mir, 'ctx> MIRParser<'tcx, 'mir, 'ctx>
where
    'mir: 'tcx, // this means tcx outlives the mir
{
    pub fn new(tcx: TyCtxt<'tcx>, body: &'mir Body<'tcx>, z3: SymExec<'ctx>) -> Self {
        let mut p = Self {
            tcx,
            mir_body: body,
            curr: z3,
            handlers: IndexMap::default(),
            stack: Vec::new(),
            path_count: 0,
            visit_counts: HashMap::new(),
            aliases: HashMap::new(),
            dangerous_spans: HashMap::default(),
            param_sinks: HashMap::new(),
            closure_sinks: HashMap::new(),
            suppress_path_taint: false,
            touched_reader: false,
            bare_target_origins: std::collections::HashSet::new(),
            pending_base: Vec::new(),
            return_sources: HashMap::new(),
            return_origin: None,
        };

        // built-ins we always want
        p.add_builtin_handlers();
        p
    }

    /// Attach interprocedural summaries (see [`ParamSinks`]) for the reporting pass.
    pub fn with_param_sinks(mut self, param_sinks: ParamSinks) -> Self {
        self.param_sinks = param_sinks;
        self
    }

    /// Attach closure summaries (see [`ClosureSinks`]) for the reporting pass.
    pub fn with_closure_sinks(mut self, closure_sinks: ClosureSinks) -> Self {
        self.closure_sinks = closure_sinks;
        self
    }

    /// Suppress ambient `path_taint` (data-flow-only). Used for closure input-probes.
    pub fn with_suppress_path_taint(mut self, v: bool) -> Self {
        self.suppress_path_taint = v;
        self
    }

    /// Attach return-origin summaries (see [`ReturnSources`]) so a call to a function
    /// that internally reads untrusted data taints its result with the real read origin.
    pub fn with_return_sources(mut self, return_sources: ReturnSources) -> Self {
        self.return_sources = return_sources;
        self
    }

    /// Record a finding: a tainted value whose taint path is `chain` reaches sink
    /// `func_path`, checked against `value`, at `span`. The `source` column is the
    /// first hop of the chain; the full `chain` is stored alongside the span.
    pub(crate) fn record_sink_hit(&mut self, chain: &str, func_path: &str, value: &str, span: Span) {
        self.record_sink_hit_ex(chain, func_path, value, span, None);
    }

    /// Like [`record_sink_hit`], plus a concrete example value (Z3 witness) the tainted
    /// argument could take at the sink — "what value can it be here." Assumes the arg is
    /// not known-composed (the generic path); the entry-vs-base routing below then
    /// treats it as a bare value.
    pub(crate) fn record_sink_hit_ex(
        &mut self,
        chain: &str,
        func_path: &str,
        value: &str,
        span: Span,
        example: Option<String>,
    ) {
        self.record_sink_hit_prov(chain, func_path, value, span, example, false);
    }

    /// Full sink-hit recorder with entry-vs-base provenance. `composed` = the tainted
    /// arg at this sink is a COMPOSED path (a base with a component join/push-ed on).
    ///
    /// Routing (unless the `LHS_NO_ENTRY_BASE` A/B escape hatch is set, which records
    /// everything verbatim):
    ///  - WHOLE-TARGET sinks (`File::create`/`write`/`open`/`remove`/…) always record;
    ///    a BARE one also marks its origin as a "bare-target origin" — evidence the
    ///    value is a per-entry target path, not merely an output directory.
    ///  - BASE-position sinks (`create_dir`/`create_dir_all`/`copy`) record immediately
    ///    when COMPOSED or from a real archive/file read; otherwise they are DEFERRED
    ///    to `pending_base` and later promoted by [`Self::filter_base_sinks`] iff their
    ///    origin is a bare-target origin (else dropped as the trusted-output-dir class).
    pub(crate) fn record_sink_hit_prov(
        &mut self,
        chain: &str,
        func_path: &str,
        value: &str,
        span: Span,
        example: Option<String>,
        composed: bool,
    ) {
        let source = chain.split(" -> ").next().unwrap_or(chain).to_string();
        // Append the sink itself as the final hop so the stored chain is the
        // complete taint path: source @ line -> … -> sink @ line.
        let sink_node = format!("{} @ {}", Self::short_callee(func_path), self.loc_short(span));
        let full = if chain.is_empty() {
            sink_node
        } else {
            format!("{chain} -> {sink_node}")
        };

        let ab_off = std::env::var("LHS_NO_ENTRY_BASE").as_deref() == Ok("1");
        let is_base = crate::matching::is_base_position_sink(func_path);
        let real_src = crate::matching::origin_is_real_source(&source)
            && crate::matching::is_reader_provenance_source(&source);

        if !ab_off && !is_base && !composed {
            // A whole-target sink fed a BARE value → this origin names a real target path.
            self.bare_target_origins.insert(source.clone());
        }

        if !ab_off && is_base && !composed && !real_src {
            // Defer: keep only if the origin proves to be a per-entry target elsewhere.
            self.pending_base
                .push((source, func_path.to_string(), value.to_string(), span, full, example));
            return;
        }

        self.dangerous_spans
            .entry((source, func_path.to_string(), value.to_string()))
            .or_default()
            .push((span, full, example));
    }

    /// Resolve deferred base-position findings after the full pass: promote a pending
    /// `create_dir`/`copy` finding iff its origin also reached a whole-target sink as a
    /// BARE value (so the value is a real target path, e.g. sevenz-rust's
    /// `default_entry_extract_fn(dest)` which does both `create_dir_all(dest)` and
    /// `File::create(dest)`); otherwise drop it as the trusted-output-dir FP class
    /// (e.g. decompress's `create_dir_all(to)`, whose `to` only reaches `File::create`
    /// via a COMPOSED `to.join(name)`, never bare).
    fn filter_base_sinks(&mut self) {
        let pending = std::mem::take(&mut self.pending_base);
        for (source, func, value, span, full, example) in pending {
            if self.bare_target_origins.contains(&source) {
                self.dangerous_spans
                    .entry((source, func, value))
                    .or_default()
                    .push((span, full, example));
            }
        }
    }

    pub fn register_handler<S: Into<String>>(
        &mut self,
        path: S,
        handler: CallHandler<'tcx, 'mir, 'ctx>,
    ) {
        let path = path.into();
        self.handlers
            .entry(path)
            .and_modify(|e| e.0 = handler)
            .or_insert((handler, Vec::new()));
    }

    pub fn register_forbid<S: Into<String>>(
        &mut self,
        path: S,
        handler: CallHandler<'tcx, 'mir, 'ctx>,
        arg_idx: usize,
        forbidden_val: &'static str,
    ) {
        let path = path.into();
        let entry = self.handlers.entry(path).or_insert((handler, Vec::new()));
        entry.0 = handler; // ensure correct handler is set
        entry.1.push(SinkInformation {
            arg_idx,
            forbidden_val,
        });
    }

    fn add_builtin_handlers(&mut self) {
        // register sinks from the settings
        for (path, arg_idx, forbidden, _attack) in SINK_FUNCTION_ARGS {
            self.register_forbid(*path, generic_string_handler, *arg_idx, forbidden);
        }

        // register env's we want to check for update
        for &name in ENV_VARS_TO_TRACK {
            self.register_forbid("std::env::set_var", generic_string_handler, 0, name);
        }

        //register sources
        for &name in SOURCE_FUNCTIONS {
            self.register_handler(name, handle_generic_source);
        }

        // all other handlers we added for processing
        self.register_handler("std::path::PathBuf::from", handle_pathbuf_from);
        self.register_handler("std::path::PathBuf::deref", handle_deref_generic);
        self.register_handler("std::path::Path::new", handle_path_new);
        self.register_handler("std::path::Path::to_path_buf", handle_path_to_path_buf);
        self.register_handler("std::path::Path::join", handle_path_join);
        self.register_handler("std::path::PathBuf::push", handle_pathbuf_push);
        // (String building — push_str/insert_str/`+=`/`join`/… — needs no per-method
        // handler: taint propagation carries the untrusted flag, and two automatic
        // binding rules make the VALUE checkable — a tainted sink arg with no modeled
        // string is treated as unconstrained, and a tainted in-place mutation of a
        // concrete-literal receiver invalidates the stale literal. See
        // `generic_string_handler` and the `&mut`-receiver step in `handle_function_call`.)
        // (Container/receiver taint — `Vec::push`, `HashMap::insert`, `String::push_str`,
        // a `Writer`, any user `add(&mut self, x)`, … — is handled generically in
        // `handle_function_call` for ANY `&mut self` call, not via a method list.)

        // String predicates modeled as Z3 string constraints, so a guard branch
        // (e.g. `if !p.contains("..")`) constrains the string content and the sink
        // query is resolved by Z3. Suffix-matched, so they resolve on the printed
        // `core::str::<impl str>::contains` etc.
        self.register_handler("str::contains", handle_str_contains);
        self.register_handler("str::starts_with", handle_str_starts_with);
        self.register_handler("str::ends_with", handle_str_ends_with);

        //some traits that are used implicitly
        self.register_handler("core::convert::From::from", handle_from_trait);
        self.register_handler("std::convert::From::from", handle_from_trait);

        self.register_handler("alloc::string::String::from", handle_string_from);
        self.register_handler("std::string::String::from", handle_string_from);
        self.register_handler("std::ffi::OsString::from", handle_string_from);

        // Sync IO reads. `read`/`read_exact`/`read_to_end`/`read_to_string` all deposit
        // the untrusted bytes into their BUFFER argument (arg 1) and return only a count
        // (or ()), so the taint must land on the buffer, not the return value. These
        // override the SOURCE_FUNCTIONS default (which taints the result) so e.g.
        // `reader.read_to_end(&mut bytes)` taints `bytes` — the archive buffer a parser
        // then reads entry names out of (hunpak's `PakFile::read`).
        self.register_handler("std::io::Read::read", handle_read_into_buf);
        self.register_handler("std::io::Read::read_exact", handle_read_into_buf);
        self.register_handler("std::io::Read::read_to_end", handle_read_into_buf);
        self.register_handler("std::io::Read::read_to_string", handle_read_into_buf);

        // --- UTF-8 lossy, also need to add to_string modeling
        self.register_handler(
            "std::string::String::from_utf8_lossy",
            handle_string_from_utf8_lossy,
        );
        self.register_handler(
            "alloc::string::String::from_utf8_lossy",
            handle_string_from_utf8_lossy,
        );
        self.register_handler("std::string::String::from_utf8", handle_string_from_utf8);
        self.register_handler("alloc::string::String::from_utf8", handle_string_from_utf8);
        self.register_handler(
            "std::result::Result::unwrap_or_default",
            handle_result_unwrap_or_default,
        );
        self.register_handler(
            "std::result::Result::<T, E>::unwrap_or_default",
            handle_result_unwrap_or_default,
        );
        self.register_handler(
            "core::result::Result::unwrap_or_default",
            handle_result_unwrap_or_default,
        );

        self.register_handler("std::ops::DerefMut::deref_mut", handle_deref_mut);
        self.register_handler("core::ops::deref::DerefMut::deref_mut", handle_deref_mut);
        self.register_handler("std::ops::Deref::deref", handle_deref_generic);
        self.register_handler("core::ops::deref::Deref::deref", handle_deref_generic);

        //format
        self.register_handler(
            "core::fmt::rt::Argument::new_display",
            handle_fmt_arg_new_display,
        );
        self.register_handler("std::fmt::Arguments::new_v1", handle_fmt_arguments_new_v1);
        self.register_handler("std::fmt::format", handle_fmt_format);
    }

    pub(crate) fn operand_tainted(&self, op: &Operand<'tcx>) -> bool {
        // If the path is tainted, everything is considered tainted — unless we are in
        // a data-flow-only probe (closure input summaries), where ambient taint would
        // pollute the which-input-routes-to-the-sink diff.
        if self.curr.path_taint && !self.suppress_path_taint {
            return true;
        }
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.taint_of_key(&self.place_key(p)),
            Operand::Constant(_) | Operand::RuntimeChecks(_) => false,
        }
    }

    /// Field-sensitive taint check: a place is tainted if its own key is tainted
    /// OR its base local is (so reading `s.field` off a tainted struct `s` yields
    /// a tainted value). This is what lets a tainted aggregate parameter (a CLI
    /// `Args` struct, an archive-entry record) taint the specific field that is
    /// later joined into a path — without it, tainting the struct is inert.
    pub(crate) fn taint_of_key(&self, key: &str) -> bool {
        if self.curr.is_tainted(key) {
            return true;
        }
        // Base local = the leading run of digits in the place key (e.g. the
        // base of "2.f0" or "2*.f3" is "2").
        let base = Self::base_local(key);
        !base.is_empty() && base != key && self.curr.is_tainted(&base)
    }

    /// The base local of a place key: its leading run of digits ("2.f0" -> "2").
    fn base_local(key: &str) -> String {
        key.chars().take_while(|c| c.is_ascii_digit()).collect()
    }

    /// Provenance of a place key (where its taint came from), field-sensitively:
    /// its own recorded origin, else its base local's.
    pub(crate) fn origin_of_key(&self, key: &str) -> Option<String> {
        if let Some(o) = self.curr.taint_origin_of(key) {
            return Some(o);
        }
        let base = Self::base_local(key);
        if !base.is_empty() && base != key {
            return self.curr.taint_origin_of(&base);
        }
        None
    }

    /// Provenance of an operand (for reporting a finding's source / taint path).
    pub(crate) fn operand_origin(&self, op: &Operand<'tcx>) -> Option<String> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.origin_of_key(&self.place_key(p)),
            _ => None,
        }
    }

    /// Is this place a COMPOSED path (a base with a component joined/pushed on),
    /// field-sensitively: its own mark, else its base local's. See
    /// [`crate::symexec::SymExecBool::composed`].
    fn composed_of_key(&self, key: &str) -> bool {
        if self.curr.is_composed(key) {
            return true;
        }
        let base = Self::base_local(key);
        !base.is_empty() && base != key && self.curr.is_composed(&base)
    }

    /// Is this operand a composed path?
    pub(crate) fn operand_composed(&self, op: &Operand<'tcx>) -> bool {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.composed_of_key(&self.place_key(p)),
            _ => false,
        }
    }

    /// Rank a candidate return origin: a GENUINE registered content-read source (e.g.
    /// `read_to_end`, `fs::read`, `Mmap::as_slice`, an archive-entry accessor) beats the
    /// `reads bytes @ …` handle-carrier pseudo-source (which fires broadly, e.g. on
    /// `File::metadata()`/`File::open()` and is often not a content read). Used so a
    /// function's return-origin summary names the real data read, not an incidental one.
    fn retsrc_rank(origin: &str) -> u8 {
        let head = origin
            .split(" -> ")
            .next()
            .unwrap_or(origin)
            .split(" @ ")
            .next()
            .unwrap_or(origin)
            .trim();
        if crate::matching::is_source_func(head) {
            3
        } else if origin.starts_with("reads bytes") {
            2
        } else {
            1
        }
    }

    /// At a `Return`, note if the return value (`_0`, or a field/deref of it) carries a
    /// REAL read/source origin — evidence this function hands back data it read from an
    /// untrusted input. `compute_return_sources` reads `self.return_origin` afterward to
    /// build the function's return-origin summary. Accumulates the BEST origin across all
    /// return paths (a genuine content read outranks an incidental `reads bytes`), so an
    /// early `?`-error path can't shadow the real data read on the success path.
    fn capture_return_origin(&mut self) {
        let mut best_rank = self
            .return_origin
            .as_deref()
            .map(Self::retsrc_rank)
            .unwrap_or(0);
        let mut best = self.return_origin.clone();
        for (k, o) in self.curr.taint_origin.iter() {
            let is_ret = k == "0"
                || k.starts_with("0.")
                || k.starts_with("0*")
                || k.starts_with("0[");
            if !is_ret {
                continue;
            }
            // Only a GENUINE registered content-read (read_to_end, fs::read, Mmap, an
            // archive-entry accessor) qualifies — not the broad `reads bytes @ …`
            // handle-carrier pseudo-source (which fires incidentally on things like
            // `File::metadata()`/`File::open()` and would mis-summarize the return).
            if Self::retsrc_rank(o) < 3 {
                continue;
            }
            if !self.taint_of_key(k) {
                continue;
            }
            if best.is_none() || 3 > best_rank {
                best = Some(o.clone());
                best_rank = 3;
            }
        }
        self.return_origin = best;
    }

    // ---- Value-provenance propagation choke point -----------------------------------
    //
    // Every dataflow edge that moves a value (assignment, aggregate construction, a
    // call's arg->result, a &mut-receiver store, a store-through-pointer) carries the
    // SAME set of provenance facts — the taint flag, the taint ORIGIN, and the
    // entry-vs-base `composed` bit — bundled in [`ValueFacts`]. Routing every edge
    // through `value_facts_of` (read) + `apply_value_facts`/`propagate_value_facts`
    // (write) means a value-fact can never be propagated at one site but forgotten at
    // another (the bug that dropped `composed` across `.parent()`). Adding a future bit
    // (e.g. a `confined` no-`..` marker) is then a change to `ValueFacts` +
    // `value_facts_of` + the two writers + `fold_operand_facts` — not to every call site.

    /// Read a place's propagation-carried facts, field-sensitively (own key, else its
    /// base local — matching [`Self::origin_of_key`] / [`Self::composed_of_key`]).
    fn value_facts_of(&self, key: &str) -> ValueFacts {
        ValueFacts {
            tainted: self.taint_of_key(key),
            origin: self.origin_of_key(key),
            composed: self.composed_of_key(key),
        }
    }

    /// Facts of an operand (constants / runtime-checks carry none).
    fn operand_facts(&self, op: &Operand<'tcx>) -> ValueFacts {
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.value_facts_of(&self.place_key(p)),
            _ => ValueFacts::default(),
        }
    }

    /// Union the boolean facts (taint / composed) of several operands; origin is left
    /// to the caller (it uses ranked attribution). A newly added boolean fact is
    /// unioned here once and every combine site inherits it.
    fn fold_operand_facts(
        &self,
        ops: &[rustc_span::source_map::Spanned<Operand<'tcx>>],
    ) -> ValueFacts {
        let mut f = ValueFacts::default();
        for sp in ops {
            let g = self.operand_facts(&sp.node);
            f.tainted |= g.tainted;
            f.composed |= g.composed;
        }
        f
    }

    /// ADDITIVELY apply facts to a place (never clears): set taint if tainted, set
    /// origin if present, mark composed if composed. Used at combine / aggregate /
    /// &mut-receiver / store-through-pointer edges, which only ever ADD provenance.
    fn apply_value_facts(&mut self, dest: &str, f: &ValueFacts) {
        if f.tainted {
            self.curr.set_taint(dest, true);
        }
        if let Some(o) = &f.origin {
            self.curr.set_taint_origin(dest, o);
        }
        if f.composed {
            self.curr.set_composed(dest);
        }
    }

    /// COPY facts from one source place to a destination — the single-source
    /// propagation choke point (a plain assignment / deref / cast). Taint uses replace
    /// semantics (it reflects the source, matching a fresh assignment); origin and the
    /// monotone `composed` bit are additive.
    fn propagate_value_facts(&mut self, src: &str, dest: &str) {
        let f = self.value_facts_of(src);
        self.curr.set_taint(dest, f.tainted);
        if let Some(o) = &f.origin {
            self.curr.set_taint_origin(dest, o);
        }
        if f.composed {
            self.curr.set_composed(dest);
        }
    }

    /// If `op` has a closure type (peeling refs), return its `DefId`, so a summary
    /// keyed by closure DefId can be consulted at the site that drives it.
    fn closure_def_of_operand(&self, op: &Operand<'tcx>) -> Option<DefId> {
        let ty = op.ty(&self.mir_body.local_decls, self.tcx);
        match ty.peel_refs().kind() {
            TyKind::Closure(did, _) => Some(*did),
            _ => None,
        }
    }

    /// Rank a taint origin for ATTRIBUTION: a concrete real-source origin (one that
    /// traces to a registered source or a reader/parse read) outranks a synthetic
    /// seed (`param[...]` / `closure arg[...]` / a capture label), which outranks
    /// no origin at all. Used so a value-combining call like `outdir.join(name)`
    /// credits the finding to the archive-entry source (`name`), not the trusted
    /// output-dir param (`outdir`) that merely happens to be tainted by the
    /// public-parameter seed. Attribution only — never changes fire/no-fire.
    pub(crate) fn origin_rank(o: Option<&str>) -> u8 {
        match o {
            None => 0,
            Some(s) if s.is_empty() => 0,
            Some(s) => {
                // A chain whose HEAD is a real registered source outranks everything
                // (this is the attribution we want). A synthetic seed outranks a bare
                // intermediate hop (`param[0] (self)` is more informative than
                // `exchange_malloc`/`into_vec`/`ok_or_else`). A non-source hop is last.
                let head = s.split(" -> ").next().unwrap_or(s);
                if crate::matching::origin_is_real_source(head) {
                    3
                } else if s.starts_with("param[")
                    || s.starts_with("closure arg[")
                    || s.starts_with("untrusted input (capt")
                    || s.starts_with("untrusted input (closure capture")
                {
                    2
                } else {
                    1
                }
            }
        }
    }

    /// Among the tainted arguments of a call, return the origin that ranks highest
    /// (real source over synthetic seed); ties resolve to the FIRST tainted arg
    /// (matching the previous first-tainted behavior). This is what makes a finding
    /// attribute to the real untrusted-input source rather than a co-tainted param.
    fn best_tainted_origin(
        &self,
        args: &[rustc_span::source_map::Spanned<Operand<'tcx>>],
    ) -> Option<String> {
        let mut best: Option<String> = None;
        let mut best_rank = 0u8;
        let mut seen = false;
        for sp in args.iter() {
            if !self.operand_tainted(&sp.node) {
                continue;
            }
            let o = self.operand_origin(&sp.node);
            let r = Self::origin_rank(o.as_deref());
            if !seen || r > best_rank {
                best = o;
                best_rank = r;
                seen = true;
            }
        }
        best
    }

    /// Short "file:line" for a span (drops the column and the end position).
    fn loc_short(&self, sp: Span) -> String {
        let s = self.tcx.sess.source_map().span_to_diagnostic_string(sp);
        let mut it = s.splitn(3, ':');
        match (it.next(), it.next()) {
            (Some(f), Some(l)) => format!("{f}:{l}"),
            _ => s,
        }
    }

    /// Compact label for a callee path: last two `::` segments, generics stripped
    /// (e.g. `std::fs::File::create` -> `File::create`, `zip::…::ZipFile::name` -> `ZipFile::name`).
    fn short_callee(path: &str) -> String {
        let stripped = crate::matching::strip_generics(path);
        let segs: Vec<&str> = stripped.split("::").filter(|s| !s.is_empty()).collect();
        let n = segs.len();
        if n >= 2 {
            format!("{}::{}", segs[n - 2], segs[n - 1])
        } else {
            stripped
        }
    }

    /// Append one hop to a taint-path chain: `prev -> callee @ file:line`. Pure
    /// plumbing calls (deref/clone/as_ref/…) are skipped so the chain shows only
    /// meaningful hops, and consecutive duplicate hops are collapsed.
    fn append_chain(&self, prev: Option<String>, callee_path: &str, span: Option<Span>) -> String {
        let label = Self::short_callee(callee_path);
        let leaf = label.rsplit("::").next().unwrap_or(&label);
        let prev = prev.unwrap_or_default();
        if crate::settings::CHAIN_SKIP_FNS.contains(&leaf) {
            return prev; // don't record plumbing hops; just carry the chain
        }
        let node = match span {
            Some(sp) => format!("{label} @ {}", self.loc_short(sp)),
            None => label,
        };
        if prev.is_empty() {
            node
        } else if prev.rsplit(" -> ").next() == Some(node.as_str()) {
            prev // collapse a repeated hop
        } else {
            format!("{prev} -> {node}")
        }
    }

    // Main entry point: analyze the MIR and return all dangerous write locations
    pub fn parse(&mut self) -> HashMap<(String, String, String), Vec<(Span, String, Option<String>)>> {
        self.stack
            .push((self.curr.clone(), BasicBlock::from_usize(0)));

        while let Some((state, bb)) = self.stack.pop() {
            self.curr = state;
            if let Some(is_terminal) = self.parse_bb_iterative(bb) {
                if is_terminal {
                    self.path_count += 1;
                }
            }
        }

        // Resolve deferred base-position (create_dir/copy) findings now that the whole
        // body has been seen and every bare-target origin is known.
        self.filter_base_sinks();
        self.dangerous_spans.clone()
    }

    pub(crate) fn def_path_str(&self, def_id: DefId) -> String {
        self.tcx.def_path_str(def_id)
    }

    // Convert a Place (memory location + projections) into a stable string key
    // Example: _1.field[2] becomes "1.f0[2]"
    pub(crate) fn place_key(&self, place: &Place<'tcx>) -> String {
        let mut key = place.local.as_usize().to_string();
        for elem in place.projection {
            use ProjectionElem::*;
            match elem {
                Deref => key.push('*'),
                Field(f, _) => key.push_str(&format!(".f{}", f.as_usize())),
                Index(l) => key.push_str(&format!("[{}]", l.as_usize())),
                ConstantIndex { offset, .. } => key.push_str(&format!("[{}]", offset)),
                Subslice { from, to, .. } => key.push_str(&format!("[{}..{}]", from, to)),
                Downcast(_, v) => key.push_str(&format!("::variant{}", v.as_usize())),
                OpaqueCast(_) => key.push_str("::opaque"),
                // `ProjectionElem::Subtype` was removed in rustc 1.94.
                ProjectionElem::UnwrapUnsafeBinder(_) => key.push_str("::unwrap_binder"),
            }
        }
        key
    }

    /// True if `ty` is a genuine file/reader HANDLE carrier ([`HANDLE_CARRIER_TYPES`],
    /// e.g. `File`/`BufReader`), peeling references. Consuming one IS a read (it happens
    /// inside the callee), so its result is a taint source even when the handle value
    /// wasn't already tainted — e.g. `parse_pack(File) -> Pack`. `File::open` is NOT a
    /// handle read: its argument is a `&Path`, so opening a handle isn't a data read.
    /// (Byte buffers `&[u8]`/`Vec<u8>` are NOT handled here: they only matter when the
    /// buffer is already tainted, which ordinary taint propagation already carries.)
    fn is_handle_carrier_ty(&self, ty: Ty<'tcx>) -> bool {
        match ty.kind() {
            TyKind::Ref(_, inner, _) => self.is_handle_carrier_ty(*inner),
            TyKind::Adt(adt, _) => {
                let p = self.tcx.def_path_str(adt.did());
                crate::settings::HANDLE_CARRIER_TYPES.iter().any(|t| p.ends_with(t))
            }
            _ => false,
        }
    }

    /// Whether a call's result type can carry path-relevant data (worth tainting).
    /// Excludes `()`, integers, bools, chars, floats — results that cannot hold a
    /// string/path — so the read-source rule does not taint the byte counts that
    /// `read`/`write` return, only real parsed data.
    fn ty_carries_data(&self, ty: Ty<'tcx>) -> bool {
        !ty.is_unit()
            && !matches!(
                ty.kind(),
                TyKind::Bool
                    | TyKind::Char
                    | TyKind::Int(_)
                    | TyKind::Uint(_)
                    | TyKind::Float(_)
                    | TyKind::Never
            )
    }

    // Collect all variables written in a basic block (for widening)
    fn collect_written_vars(&self, bb: BasicBlock) -> HashSet<String> {
        let mut vars = HashSet::new();
        for stmt in &self.mir_body.basic_blocks[bb].statements {
            if let StatementKind::Assign(assignment) = &stmt.kind {
                vars.insert(self.place_key(&assignment.0));
            }
        }
        vars
    }

    // Check if a constraint mentions any of the given variable names
    fn constraint_mentions(names: &HashSet<String>, constraint: &z3::ast::Bool<'ctx>) -> bool {
        let constraint_text = constraint.to_string();
        names.iter().any(|name| constraint_text.contains(name))
    }

    // Process a single basic block iteratively
    fn parse_bb_iterative(&mut self, bb: BasicBlock) -> Option<bool> {
        // Handle loops: track visit counts and apply widening
        let counter = self.visit_counts.entry(bb).or_insert(0);
        *counter += 1;

        if *counter > MAX_LOOP_ITER {
            return None; // Stop processing this path
        }

        if *counter == MAX_LOOP_ITER {
            // println!(
            //     "bb{} exceeded limit {} — applying widening",
            //     bb.as_u32(),
            //     MAX_LOOP_ITER
            // );

            // Widening: remove constraints on variables modified in this block
            let written_vars = self.collect_written_vars(bb);
            self.curr
                .constraints
                .retain(|c| !Self::constraint_mentions(&written_vars, c));
            return None;
        }

        let data = &self.mir_body.basic_blocks[bb];

        // Process all statements in this basic block
        for stmt in &data.statements {
            if let StatementKind::Assign(assignment) = &stmt.kind {
                self.parse_assignment(assignment.clone());
            }
        }

        // Determine if this is a terminal block (function exit)
        let is_terminal = matches!(
            &data.terminator().kind,
            TerminatorKind::Return
                | TerminatorKind::Unreachable
                | TerminatorKind::CoroutineDrop
                | TerminatorKind::UnwindResume
                | TerminatorKind::UnwindTerminate { .. }
                | TerminatorKind::TailCall { .. }
        );

        // Handle control flow based on terminator type
        self.handle_terminator(&data.terminator().kind);

        Some(is_terminal)
    }

    // Handle different types of control flow terminators
    fn handle_terminator(&mut self, terminator: &TerminatorKind<'tcx>) {
        match terminator {
            // Simple jump to another block
            TerminatorKind::Goto { target } => {
                self.stack.push((self.curr.clone(), *target));
            }

            // Terminal blocks - execution ends here
            TerminatorKind::Return | TerminatorKind::TailCall { .. } => {
                // Note if this function returns data it read from an untrusted input, so
                // callers can attribute the result to that real read (return-origin
                // summary).
                self.capture_return_origin();
            }
            TerminatorKind::Unreachable
            | TerminatorKind::CoroutineDrop
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate { .. } => {
                // Nothing to do - this path ends
            }

            // Conditional branches (if/match statements)
            TerminatorKind::SwitchInt { discr, targets } => {
                self.handle_switch_int(discr.clone(), targets.clone());
            }

            // Function calls - the most important case for our analysis
            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                ..
            } => {
                self.handle_function_call(
                    func.clone(),
                    args.clone(),
                    destination.clone(),
                    *target,
                    (*unwind).clone(),
                );
            }

            // Runtime assertions
            TerminatorKind::Assert {
                cond,
                expected,
                target,
                unwind,
                ..
            } => {
                self.handle_assert(cond.clone(), *expected, *target, (*unwind).clone());
            }

            // Other control flow constructs
            TerminatorKind::Yield { resume, drop, .. } => {
                self.stack.push((self.curr.clone(), *resume));
                if let Some(d) = drop {
                    self.stack.push((self.curr.clone(), *d));
                }
            }

            TerminatorKind::Drop { target, unwind, .. } => {
                self.stack.push((self.curr.clone(), *target));
                if let UnwindAction::Cleanup(clean) = unwind {
                    self.stack.push((self.curr.clone(), *clean));
                }
            }

            TerminatorKind::InlineAsm {
                targets, unwind, ..
            } => {
                for &t in targets {
                    self.stack.push((self.curr.clone(), t));
                }
                if let UnwindAction::Cleanup(clean) = unwind {
                    self.stack.push((self.curr.clone(), *clean));
                }
            }

            TerminatorKind::FalseEdge { real_target, .. } => {
                self.stack.push((self.curr.clone(), *real_target));
            }

            TerminatorKind::FalseUnwind { real_target, .. } => {
                self.stack.push((self.curr.clone(), *real_target));
            }
        }
    }

    // Parse assignment statements: `destination = rvalue`
    // This is expanded to handle more assignment types beyond just Use and BinaryOp
    fn parse_assignment(&mut self, assignment: Box<(Place<'tcx>, Rvalue<'tcx>)>) {
        let (destination, rvalue) = *assignment;
        let dest_key = self.place_key(&destination);

        match rvalue {
            // Simple copy/move operations: `x = y`
            Rvalue::Use(operand) => {
                self.handle_use_operation(&dest_key, &operand);
            }

            // Binary operations: `x = y + z`, `x = y == z`, etc.
            Rvalue::BinaryOp(op, operands) => {
                self.handle_binary_operation(&dest_key, op, &operands);
            }

            // Reference creation: `x = &y` - needed for tracking references to PathBuf/Path
            Rvalue::Ref(_, _, place) => {
                self.handle_reference_operation(&dest_key, &place);
            }

            // Type casts: `x = y as T` - needed for various type conversions
            Rvalue::Cast(_, operand, _) => {
                self.handle_cast_operation(&dest_key, &operand);
            }

            // Struct/tuple/array construction: `x = SomeStruct { field: value }`
            // This is crucial for tracking PathBuf construction
            Rvalue::Aggregate(kind, operands) => {
                // Closure construction `_dest = {closure}(upvars...)`: if a captured
                // upvar operand is tainted (a real source reached it) and the closure
                // routes THAT upvar into a sink, the sink is reachable — report it
                // attributed to the upvar's real origin. This is the real replacement
                // for the synthetic capture seed (e.g. uncbv's extract_block closure
                // capturing `file` and writing `output.join(file.filename)`).
                if let AggregateKind::Closure(did, _) = kind.as_ref() {
                    if let Some(csinks) = self.closure_sinks.get(did).cloned() {
                        for cs in csinks.iter() {
                            let ClosureInput::Upvar(k) = cs.input else {
                                continue;
                            };
                            let Some((_, op)) =
                                operands.iter_enumerated().find(|(fi, _)| fi.as_usize() == k)
                            else {
                                continue;
                            };
                            if self.operand_tainted(op) {
                                let chain = self
                                    .operand_origin(op)
                                    .unwrap_or_else(|| "captured value".to_string());
                                let composed = self.operand_composed(op);
                                self.record_sink_hit_prov(
                                    &chain,
                                    &cs.sink_func,
                                    &cs.forbidden_val,
                                    cs.span,
                                    cs.example.clone(),
                                    composed,
                                );
                            }
                        }
                    }
                }
                // For single-operand aggregates (like PathBuf wrapping a string), copy the value
                if operands.len() == 1 {
                    if let Some((_, operand)) = operands.iter_enumerated().next() {
                        if let Operand::Copy(place) | Operand::Move(place) = operand {
                            let src_key = self.place_key(&place);
                            self.copy_variable_value(&src_key, &dest_key);
                        }
                    }
                }
                // Field-sensitive provenance: taint each constructed FIELD key with
                // its own operand's taint+origin, so a later read of a specific field
                // (e.g. `metadata.filename` from `ZipFile::name`) reports THAT field's
                // real source rather than a sibling field's (`comment`) or a merged
                // base origin. Field index = operand index for struct/tuple aggregates
                // (matches place_key's `.f{idx}`); enum-variant aggregates don't line
                // up with `N.fK`, so they just fall through to the base taint below.
                for (fidx, op) in operands.iter_enumerated() {
                    let f = self.operand_facts(op);
                    if f.tainted {
                        let fkey = format!("{}.f{}", dest_key, fidx.as_usize());
                        self.apply_value_facts(&fkey, &f);
                    }
                }
                // An aggregate is tainted if any of its constituents is tainted
                // (covers tuples/structs/arrays and the enum case); carry the
                // provenance (incl. composed) from the first tainted constituent as the
                // base catch-all.
                if let Some(op) = operands.iter().find(|op| self.operand_tainted(op)) {
                    let f = self.operand_facts(op);
                    self.apply_value_facts(&dest_key, &f);
                }
            }

            // Copy for dereference: used in some compiler optimizations
            Rvalue::CopyForDeref(place) => {
                self.handle_copy_for_deref(&dest_key, &place);
            }

            // Unary operations: `x = !y`, `x = -y`, pointer metadata, ...
            Rvalue::UnaryOp(op, operand) => {
                self.handle_unary_operation(&dest_key, op, &operand);
            }

            // Reading an enum's discriminant — e.g. the scrutinee of a `match`.
            // Modeled as an unknown integer with taint carried from the enum
            // value. (Exact variant indices would need layout reasoning.)
            Rvalue::Discriminant(place) => {
                let src = self.place_key(&place);
                self.curr.create_int(&dest_key);
                let tainted = self.curr.is_tainted(&src);
                self.curr.set_taint(&dest_key, tainted);
            }

            // NOTE: `Rvalue::Len` was removed from MIR; array/slice length now
            // lowers to `Rvalue::UnaryOp(UnOp::PtrMetadata, ..)`, handled above.

            // Array initializer `[elem; N]`: forward value/taint from the element.
            Rvalue::Repeat(operand, _) => {
                self.handle_use_operation(&dest_key, &operand);
            }

            // `Box::new`-style shallow initialization: forward the inner operand.
            Rvalue::ShallowInitBox(operand, _) => {
                self.handle_use_operation(&dest_key, &operand);
            }

            // Raw pointer creation `x = &raw const/mut y`: like a reference, the
            // pointer aliases `y`, so forward its value and taint.
            Rvalue::RawPtr(_, place) => {
                let src_key = self.place_key(&place);
                self.copy_variable_value(&src_key, &dest_key);
                self.curr.propagate_taint(&src_key, &dest_key);
            }

            // Other operations we don't currently handle
            // (`Rvalue::NullaryOp` was removed/restructured in rustc 1.94; its
            // size_of/align_of results fall through here as unmodeled.)
            _ => {
                // println!("Unsupported Rvalue in assignment: {:?}", rvalue);
            }
        }

        // Store-through-pointer: `(*P) = <tainted>` writes tainted data into whatever
        // `P` points to, so the OWNER of that memory is now tainted. We model locals,
        // not the heap, so without this the taint is lost when a value is written
        // through a raw/box pointer and read back via the owner — e.g. the `vec![..]`
        // macro allocates a `Box`, takes a raw pointer into it, writes the (tainted)
        // elements through that pointer, then `into_vec`s the box into a `Vec`. Taint
        // the base local of the pointer's alias so it flows to the owning `Box`/`Vec`.
        if destination
            .projection
            .iter()
            .any(|e| matches!(e, ProjectionElem::Deref))
            && self.taint_of_key(&dest_key)
        {
            let ptr_key = destination.local.as_usize().to_string();
            let owner = Self::base_local(&self.resolve_alias(&ptr_key));
            if !owner.is_empty() && owner != ptr_key {
                // The tainted value written through the pointer carries all its facts
                // (incl. composed) into the owning Box/Vec.
                let f = self.value_facts_of(&dest_key);
                self.apply_value_facts(&owner, &f);
            }
        }
    }

    // Handle simple copy/move operations
    fn handle_use_operation(&mut self, dest_key: &str, operand: &Operand<'tcx>) {
        match operand {
            // Copy from another variable: `x = y`
            Operand::Copy(place) | Operand::Move(place) => {
                let src_key = self.place_key(place);
                self.copy_variable_value(&src_key, dest_key);
            }

            // Assign constant: `x = 42` or `x = "hello"`
            Operand::Constant(constant) => {
                self.assign_constant_value(dest_key, constant);
            }

            // Runtime-check operands carry no value we model.
            Operand::RuntimeChecks(_) => {}
        }
    }

    // Handle binary operations like addition, comparison, etc.
    fn handle_binary_operation(
        &mut self,
        dest_key: &str,
        op: BinOp,
        operands: &(Operand<'tcx>, Operand<'tcx>),
    ) {
        let (lhs, rhs) = operands;

        // Try to get integer operands
        if let (Some(lhs_int), Some(rhs_int)) = (
            self.get_int_from_operand(lhs),
            self.get_int_from_operand(rhs),
        ) {
            let (width, signed) = self.int_ty_info(lhs);
            // Precise bitvector modeling of bitwise/shift ops is only affordable
            // when both operands are concrete constants (it folds to a numeral).
            // On symbolic operands, mixing Z3's Int and BitVector theories makes
            // every later solve blow up, so we fall back to an unknown integer
            // there (taint is still propagated below). See handle_int_binary_op.
            let both_const =
                matches!(lhs, Operand::Constant(_)) && matches!(rhs, Operand::Constant(_));
            self.handle_int_binary_op(dest_key, op, &lhs_int, &rhs_int, width, signed, both_const);

            // Propagate taint through the operation: the result is tainted if
            // either operand is. This must run even for operations whose value
            // we do not model (bitwise/shift), otherwise taint is lost through
            // integer arithmetic — a soundness gap for the taint analysis.
            if self.operand_tainted(lhs) || self.operand_tainted(rhs) {
                self.curr.set_taint(dest_key, true);
                let o = self
                    .operand_origin(lhs)
                    .or_else(|| self.operand_origin(rhs));
                if let Some(o) = o {
                    self.curr.set_taint_origin(dest_key, &o);
                }
            }
            return;
        }

        // Handle string comparisons
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            if let (Some(lhs_str), Some(rhs_str)) = (
                self.get_string_from_operand(lhs),
                self.get_string_from_operand(rhs),
            ) {
                let eq_result = self.curr.string_eq(&lhs_str, &rhs_str);
                let final_result = if matches!(op, BinOp::Eq) {
                    eq_result
                } else {
                    self.curr.not(&eq_result)
                };
                self.curr.assign_bool(dest_key, final_result);
            }
        }
        if self.operand_tainted(lhs) || self.operand_tainted(rhs) {
            self.curr.set_taint(dest_key, true);
        }
    }

    // Handle integer binary operations
    fn handle_int_binary_op(
        &mut self,
        dest_key: &str,
        op: BinOp,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
        width: u32,
        signed: bool,
        precise: bool,
    ) {
        use BinOp::*;
        // Bitwise/shift result: precise bitvector value when affordable
        // (constant operands), otherwise an unknown integer.
        macro_rules! bitop {
            ($f:ident) => {{
                if precise {
                    let v = self.curr.$f(lhs, rhs, width, signed);
                    self.curr.assign_int(dest_key, v);
                } else {
                    self.curr.create_int(dest_key);
                }
            }};
        }
        match op {
            // Comparisons return booleans
            Eq => self.curr.assign_bool(dest_key, self.curr.int_eq(lhs, rhs)),
            Ne => self
                .curr
                .assign_bool(dest_key, self.curr.not(&self.curr.int_eq(lhs, rhs))),
            Lt => self.curr.assign_bool(dest_key, self.curr.int_lt(lhs, rhs)),
            Le => self.curr.assign_bool(dest_key, self.curr.int_le(lhs, rhs)),
            Gt => self.curr.assign_bool(dest_key, self.curr.int_gt(lhs, rhs)),
            Ge => self.curr.assign_bool(dest_key, self.curr.int_ge(lhs, rhs)),

            // Arithmetic operations return integers
            Add => self.curr.assign_int(dest_key, self.curr.add(lhs, rhs)),
            Sub => self.curr.assign_int(dest_key, self.curr.sub(lhs, rhs)),
            Mul => self.curr.assign_int(dest_key, self.curr.mul(lhs, rhs)),
            Div => self.curr.assign_int(dest_key, self.curr.div(lhs, rhs)),
            Rem => self.curr.assign_int(dest_key, self.curr.rem(lhs, rhs)),

            // Bitwise and shift operations, modeled at the operands' machine
            // width via Z3 bitvectors when both operands are concrete (see the
            // `bitop!` macro and SymExec::bit_and etc.).
            BitAnd => bitop!(bit_and),
            BitOr => bitop!(bit_or),
            BitXor => bitop!(bit_xor),
            Shl => bitop!(shl),
            Shr => bitop!(shr),

            // Handle overflow operations
            // These operations return tuples (result, overflow_flag) instead of just the result
            AddWithOverflow | SubWithOverflow | MulWithOverflow => {
                // For overflow operations, we create the arithmetic result and assume no overflow
                let arithmetic_result = match op {
                    AddWithOverflow => self.curr.add(lhs, rhs),
                    SubWithOverflow => self.curr.sub(lhs, rhs),
                    MulWithOverflow => self.curr.mul(lhs, rhs),
                    _ => unreachable!(),
                };

                // Store the arithmetic result in field 0 of the destination
                let field0_key = format!("{}.f0", dest_key);
                self.curr.assign_int(&field0_key, arithmetic_result);

                // Store false (no overflow) in field 1 of the destination
                let field1_key = format!("{}.f1", dest_key);
                self.curr
                    .assign_bool(&field1_key, self.curr.static_bool(false));
            }

            _ => {
                // Value not modeled precisely (offset/cmp/unchecked shifts):
                // treat the result as an unknown integer so downstream uses
                // still have a symbolic value rather than nothing. Taint is
                // handled by the caller (handle_binary_operation).
                self.curr.create_int(dest_key);
            }
        }
    }

    // Determine the bit width and signedness of an operand's integer type,
    // used to model bitwise/shift operations at the correct machine width.
    // Defaults to a signed 64-bit view when the type is not a fixed-width int
    // (pointer-sized types are treated as 64-bit for this target).
    fn int_ty_info(&self, op: &Operand<'tcx>) -> (u32, bool) {
        let ty = op.ty(&self.mir_body.local_decls, self.tcx);
        match ty.kind() {
            TyKind::Int(it) => {
                let w = match it {
                    IntTy::I8 => 8,
                    IntTy::I16 => 16,
                    IntTy::I32 => 32,
                    IntTy::I64 => 64,
                    IntTy::I128 => 128,
                    IntTy::Isize => 64,
                };
                (w, true)
            }
            TyKind::Uint(ut) => {
                let w = match ut {
                    UintTy::U8 => 8,
                    UintTy::U16 => 16,
                    UintTy::U32 => 32,
                    UintTy::U64 => 64,
                    UintTy::U128 => 128,
                    UintTy::Usize => 64,
                };
                (w, false)
            }
            _ => (64, true),
        }
    }

    // Handle unary operations: negation, logical/bitwise NOT, pointer metadata.
    fn handle_unary_operation(&mut self, dest_key: &str, op: UnOp, operand: &Operand<'tcx>) {
        match op {
            UnOp::Neg => {
                if let Some(v) = self.get_int_from_operand(operand) {
                    let zero = self.curr.static_int(0);
                    let neg = self.curr.sub(&zero, &v);
                    self.curr.assign_int(dest_key, neg);
                } else {
                    self.curr.create_int(dest_key);
                }
            }
            UnOp::Not => {
                if let Some(b) = self.get_bool_from_operand(operand) {
                    let nb = self.curr.not(&b);
                    self.curr.assign_bool(dest_key, nb);
                } else {
                    // Bitwise NOT on an integer: result value is unknown.
                    self.curr.create_int(dest_key);
                }
            }
            // PtrMetadata (and any other unary op): unknown integer result.
            _ => {
                self.curr.create_int(dest_key);
            }
        }
        if self.operand_tainted(operand) {
            self.curr.set_taint(dest_key, true);
        }
    }

    // Handle reference operations: `x = &y`
    // This is important for tracking when PathBuf objects are borrowed as &Path
    fn handle_reference_operation(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
        // `copy_variable_value` is field-sensitive (base-local aware) and already
        // sets/propagates taint. A trailing exact-key `propagate_taint` here would
        // CLOBBER a real field taint back to false for `dest = &s.field` (src_key
        // "N.fK" is tainted via its base local "N", but is_tainted checks only the
        // exact key) — killing the bare-field zip-slip flow (openshaiya `&file.name`,
        // mff `&f.file_name`). So do not re-propagate.
        self.copy_variable_value(&src_key, dest_key);
        // need to keep track of the aliases as well, so updates can be properly applied
        self.aliases.insert(dest_key.to_string(), src_key);
    }

    // Resolve an alias to its original variable , if no alias exists return the variable back
    pub(crate) fn resolve_alias(&self, key: &str) -> String {
        let mut cur = key;
        // prevent cycles
        let mut seen = std::collections::HashSet::new();
        while let Some(next) = self.aliases.get(cur) {
            if !seen.insert(cur.to_string()) {
                break;
            }
            cur = next;
        }
        cur.to_string()
    }

    // Handle cast operations: `x = y as T`
    // Needed for various type conversions in path operations
    fn handle_cast_operation(&mut self, dest_key: &str, operand: &Operand<'tcx>) {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            let src_key = self.place_key(place);
            // copy value + taint (field-sensitive; the trailing exact-key
            // propagate_taint was removed — see handle_reference_operation).
            self.copy_variable_value(&src_key, dest_key);

            // preserve aliasing across the cast (to the base, not just the immediate key)
            let base = self.resolve_alias(&src_key);
            self.aliases.insert(dest_key.to_string(), base);
        }
    }
    // Handle copy for dereference operations
    // Used in some compiler optimizations
    fn handle_copy_for_deref(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
        // field-sensitive; trailing exact-key propagate_taint removed (it clobbered
        // base-local field taint — see handle_reference_operation).
        self.copy_variable_value(&src_key, dest_key);
    }

    // Copy a variable's value from source to destination
    fn copy_variable_value(&mut self, src_key: &str, dest_key: &str) {
        if let Some(string_val) = self.curr.get_string(src_key).cloned() {
            self.curr.assign_string(dest_key, string_val);
        } else if let Some(int_val) = self.curr.get_int(src_key).cloned() {
            self.curr.assign_int(dest_key, int_val);
        } else if let Some(bool_val) = self.curr.get_bool(src_key).cloned() {
            self.curr.assign_bool(dest_key, bool_val);
        }
        // Field-sensitive: propagate taint (and its provenance) from the source
        // (or its base local) to the destination.
        // All value-provenance facts (taint, origin, and the entry-vs-base `composed`
        // bit) move together through the one choke point — so a composed path stays
        // composed through the plumbing hops (deref/clone/as_ref/parent/…) that route
        // it to a sink, and any future fact rides along without touching this site.
        self.propagate_value_facts(src_key, dest_key);
    }

    // Assign a constant value to a variable
    fn assign_constant_value(
        &mut self,
        dest_key: &str,
        constant: &rustc_middle::mir::ConstOperand<'tcx>,
    ) {
        let const_val = &constant.const_;

        // Try different constant types
        if let Some(scalar_int) = const_val.try_to_scalar_int() {
            let int_val = scalar_int.to_int(scalar_int.size()) as i64;
            let z3_int = self.curr.static_int(int_val.into());
            self.curr.assign_int(dest_key, z3_int);
        } else if let Some(bool_val) = const_val.try_to_bool() {
            self.curr
                .assign_bool(dest_key, self.curr.static_bool(bool_val));
        } else if let Some(string_val) =
            get_operand_const_string(self.tcx, &Operand::Constant(Box::new(constant.clone())))
        {
            self.curr
                .assign_string(dest_key, self.curr.static_string(&string_val));
        } else {
            // println!(
            //     "    Could not assign constant to {} - unrecognized type",
            //     dest_key
            // );
        }
    }

    // conditional branch handling with satisfiability checking
    // prevents exploring unsatisfiable paths
    fn handle_switch_int(&mut self, discr: Operand<'tcx>, targets: SwitchTargets) {
        let local = match discr {
            Operand::Copy(place) | Operand::Move(place) => place.local,
            Operand::Constant(_) | Operand::RuntimeChecks(_) => return, // Can't branch on these
        };

        let local_key = local.as_usize().to_string();

        if let Some(bool_condition) = self.curr.get_bool(&local_key).cloned() {
            // Boolean switch: create two paths with opposite constraints
            let (val0, bb0) = targets.iter().next().unwrap();
            let bb_else = targets.otherwise();

            let mut true_state = self.curr.clone();
            let mut false_state = self.curr.clone();

            // Add appropriate constraints based on the branch value
            let (true_constraint, false_constraint) = if val0 == 0 {
                (true_state.not(&bool_condition), bool_condition.clone())
            } else {
                (bool_condition.clone(), false_state.not(&bool_condition))
            };

            if self.operand_tainted(&discr) {
                let o = self.operand_origin(&discr);
                true_state.taint_path(o.clone());
                false_state.taint_path(o);
            }
            true_state.add_constraint(true_constraint);
            if self.is_path_satisfiable(&true_state) {
                self.stack.push((true_state, bb0));
            }
            // Check if the false branch is satisfiable before exploring it
            false_state.add_constraint(false_constraint);
            if self.is_path_satisfiable(&false_state) {
                self.stack.push((false_state, bb_else));
            }
        } else {
            // Unknown condition: explore all branches
            let discr_origin = self.operand_origin(&discr);
            for (_, bb) in targets.iter() {
                let mut st = self.curr.clone();
                if self.operand_tainted(&discr) {
                    st.taint_path(discr_origin.clone());
                }
                self.stack.push((st, bb));
            }
            let mut st = self.curr.clone();
            if self.operand_tainted(&discr) {
                st.taint_path(discr_origin.clone());
            }
            self.stack.push((self.curr.clone(), targets.otherwise()));
        }
    }

    // Enhanced runtime assertion handling with satisfiability checking
    fn handle_assert(
        &mut self,
        cond: Operand<'tcx>,
        expected: bool,
        target: BasicBlock,
        unwind: UnwindAction,
    ) {
        if let Some(local_idx) = get_operand_local(&cond) {
            if let Some(bool_condition) = self.curr.get_bool(&local_idx.to_string()).cloned() {
                // Create success path with assertion constraint
                let mut success_state = self.curr.clone();
                let success_constraint = if expected {
                    bool_condition.clone()
                } else {
                    success_state.not(&bool_condition)
                };
                success_state.add_constraint(success_constraint);
                if self.operand_tainted(&cond) {
                    let o = self.operand_origin(&cond);
                    success_state.taint_path(o);
                }

                // Only explore the success path if it's satisfiable
                if self.is_path_satisfiable(&success_state) {
                    self.stack.push((success_state, target));
                }

                // Create failure path (if there's an unwind handler)
                if let UnwindAction::Cleanup(cleanup_bb) = unwind {
                    let mut failure_state = self.curr.clone();
                    let failure_constraint = if expected {
                        failure_state.not(&bool_condition)
                    } else {
                        bool_condition
                    };
                    failure_state.add_constraint(failure_constraint);
                    if self.operand_tainted(&cond) {
                        let o = self.operand_origin(&cond);
                        failure_state.taint_path(o);
                    }

                    // Only explore the failure path if it's satisfiable
                    if self.is_path_satisfiable(&failure_state) {
                        self.stack.push((failure_state, cleanup_bb));
                    }
                }
            } else {
                // Unknown condition: assume assertion passes
                let mut st = self.curr.clone();
                if self.operand_tainted(&cond) {
                    let o = self.operand_origin(&cond);
                    st.taint_path(o);
                }
                self.stack.push((st, target));
            }
        } else {
            // Can't analyze condition
            let mut st = self.curr.clone();
            if self.operand_tainted(&cond) {
                let o = self.operand_origin(&cond);
                st.taint_path(o);
            }
            self.stack.push((st, target));
        }
    }

    fn find_handler(
        &self,
        path: &str,
    ) -> Option<(CallHandler<'tcx, 'mir, 'ctx>, Vec<SinkInformation>)> {
        if let Some((h, sinks)) = self.handlers.get(path) {
            return Some((*h, sinks.clone()));
        }
        // Also match after stripping turbofish generics, so a catalog suffix like
        // `ZipFile::name` resolves on `zip::read::ZipFile::<R>::name`.
        let stripped = crate::matching::strip_generics(path);
        self.handlers
            .iter()
            .filter(|(k, _)| {
                let k = k.as_str();
                crate::matching::seg_match(path, k) || crate::matching::seg_match(&stripped, k)
            })
            .max_by_key(|(k, _)| k.len())
            .map(|(_, (h, sinks))| (*h, sinks.clone()))
    }

    // Handle function calls - this is completely rewritten to detect path operations
    fn handle_function_call(
        &mut self,
        func: Operand<'tcx>,
        args: Box<[rustc_span::source_map::Spanned<Operand<'tcx>>]>,
        dest: Place<'tcx>,
        target: Option<BasicBlock>,
        unwind: UnwindAction,
    ) {
        // Set when the callee is a confining accessor/validator (SANITIZERS): its
        // result is trusted, so the generic arg->result taint propagation below is
        // suppressed (otherwise a value validated by e.g. `enclosed_name()` would
        // still be treated as tainted at a downstream sink -> false positive).
        let mut sanitized = false;
        // Set when the callee is a registered taint SOURCE (SOURCE_FUNCTIONS): the
        // source handler defines the value's origin, so the generic arg->result
        // propagation below must NOT overwrite it with the receiver's chain (e.g.
        // `Mmap::as_slice()` reads the archive bytes — its origin is "the file read",
        // not the param that named the file the `Mmap` was opened from).
        let mut is_source_call = false;
        if let Some(def_id) = get_operand_def_id(&func) {
            let path = self.def_path_str(def_id);

            let stripped = crate::matching::strip_generics(&path);
            if crate::settings::SANITIZERS
                .iter()
                .any(|s| crate::matching::seg_match(&path, s) || crate::matching::seg_match(&stripped, s))
            {
                sanitized = true;
            }
            if crate::settings::SOURCE_FUNCTIONS
                .iter()
                .any(|s| crate::matching::seg_match(&path, s) || crate::matching::seg_match(&stripped, s))
            {
                is_source_call = true;
            }

            if let Some((handler, sinks)) = self.find_handler(&path) {
                let arg_vec: Vec<Operand<'tcx>> = args.iter().map(|s| s.node.clone()).collect();
                let base_call = Call {
                    func_def_id: def_id,
                    args: arg_vec,
                    dest,
                    span: get_operand_span(&func),
                    sink: None,
                };

                if sinks.is_empty() {
                    handler(self, base_call);
                } else {
                    for s in sinks {
                        let mut c = base_call.clone();
                        c.sink = Some(s);
                        handler(self, c);
                    }
                }
            }

            // Interprocedural: if the callee's summary says a parameter routes a
            // tainted value into a sink, and we pass a tainted argument in that
            // position, that sink is reachable from this call — report it here.
            let matched: Vec<ParamSink> = self
                .param_sinks
                .get(&path)
                .or_else(|| self.param_sinks.get(&stripped))
                .cloned()
                .unwrap_or_default();
            for ps in matched {
                if let Some(sp) = args.get(ps.param_idx) {
                    if self.operand_tainted(&sp.node) {
                        // Taint path = the argument's chain, plus this call as the
                        // hop into the callee that internally reaches the sink.
                        let base = self
                            .operand_origin(&sp.node)
                            .or_else(|| self.curr.path_taint_origin.clone());
                        // Entry-vs-base: the callee's summary already dropped bare
                        // output-dir base sinks (they never entered its post-filtered
                        // findings), so anything here is legit; we still pass the
                        // call-site arg's composedness so the caller's own
                        // bare-target-origin promotion applies consistently.
                        let composed = self.operand_composed(&sp.node);
                        let chain = self.append_chain(base, &path, get_operand_span(&func));
                        let chain = if chain.is_empty() {
                            format!("arg[{}]", ps.param_idx)
                        } else {
                            chain
                        };
                        self.record_sink_hit_prov(
                            &chain,
                            &ps.sink_func,
                            &ps.forbidden_val,
                            ps.span,
                            ps.example.clone(),
                            composed,
                        );
                    }
                }
            }

            // Higher-order call: ANY call that passes a CLOSURE whose element/arg routes
            // into a sink, together with a TAINTED non-closure argument, is applying
            // that closure to tainted data — so follow the taint into the closure. This
            // is fully generic (no adaptor name list): it covers `coll.for_each(|x|…)`,
            // `.map`, `.try_for_each`, `.fold`, AND any user-defined higher-order fn,
            // because it keys on "an argument is a closure" + "another argument is
            // tainted", not on the callee's name. (The captured-upvar case is handled
            // separately at the closure's construction site.) Attributed to the tainted
            // data argument's origin (ranked: a real source over a synthetic seed).
            for (ci, cop) in args.iter().enumerate() {
                let Some(did) = self.closure_def_of_operand(&cop.node) else {
                    continue;
                };
                let Some(csinks) = self.closure_sinks.get(&did).cloned() else {
                    continue;
                };
                let arg_sinks: Vec<_> = csinks
                    .iter()
                    .filter(|c| matches!(c.input, ClosureInput::Arg(_)))
                    .collect();
                if arg_sinks.is_empty() {
                    continue;
                }
                // Best-ranked tainted NON-closure argument = the data fed to the closure.
                let mut src_origin: Option<String> = None;
                let mut best_rank = 0u8;
                let mut any_tainted = false;
                for (i, s) in args.iter().enumerate() {
                    if i == ci || !self.operand_tainted(&s.node) {
                        continue;
                    }
                    any_tainted = true;
                    let o = self.operand_origin(&s.node);
                    let r = Self::origin_rank(o.as_deref());
                    if src_origin.is_none() || r > best_rank {
                        src_origin = o;
                        best_rank = r;
                    }
                }
                if !any_tainted {
                    continue;
                }
                let base = src_origin.or_else(|| self.curr.path_taint_origin.clone());
                let chain = self.append_chain(base, &path, get_operand_span(&func));
                let chain = if chain.is_empty() {
                    "closure input".to_string()
                } else {
                    chain
                };
                for cs in arg_sinks {
                    self.record_sink_hit_ex(
                        &chain,
                        &cs.sink_func,
                        &cs.forbidden_val,
                        cs.span,
                        cs.example.clone(),
                    );
                }
            }
        }

        // taint propagation: a tainted argument taints the call's result (carrying
        // its taint-path chain, with this call appended as a hop) — unless the
        // callee is a sanitizer, whose result is confined (taint cleared).
        let callee_path = get_operand_def_id(&func).map(|d| self.def_path_str(d));
        let callee_span = get_operand_span(&func);
        let dest_key = self.place_key(&dest);
        if sanitized {
            self.curr.set_taint(&dest_key, false);
        } else if is_source_call {
            // A recognized source already set the dest's taint + origin; don't let
            // the receiver's chain clobber the "reads bytes"/source origin.
        } else if args.iter().any(|sp| self.operand_tainted(&sp.node)) {
            // Combine the args' facts: the result is tainted, and it is `composed` if any
            // arg was — so an unmodeled reshaping of a composed path (`outpath.parent()`,
            // `.as_path()`, `.to_owned()`, …) keeps carrying its appended component.
            let mut f = self.fold_operand_facts(&args);
            f.tainted = true;
            // Rank the origin: prefer a real-source arg over a co-tainted synthetic
            // seed (e.g. credit `join(outdir, name)` to `name`'s archive source, not
            // to the output-dir param that the public-param seed also tainted).
            let prev = self.best_tainted_origin(&args);
            let chain = match &callee_path {
                Some(cp) => self.append_chain(prev, cp, callee_span),
                None => prev.unwrap_or_default(),
            };
            f.origin = if chain.is_empty() { None } else { Some(chain) };
            self.apply_value_facts(&dest_key, &f);
        }

        // Return-origin summary: if the callee INTERNALLY reads untrusted input and hands
        // back data derived from it (e.g. `PakFile::read` opens the file, reads it, and
        // returns the parsed archive), taint the RESULT with that real read origin — even
        // when no argument is tainted — so `let archive = read(path)` makes `archive`
        // dirty-from-the-file, and a sink in a DIFFERENT function (`archive.extract(..)`)
        // is traced back to the actual read rather than a rough public-param guess. This
        // is the mirror of the ParamSink ("dirty flows IN") summary: "dirty comes OUT".
        if !sanitized {
            let ret_origin = callee_path.as_ref().and_then(|cp| {
                self.return_sources
                    .get(cp)
                    .or_else(|| self.return_sources.get(&crate::matching::strip_generics(cp)))
                    .cloned()
            });
            if let Some(origin) = ret_origin {
                self.curr.set_taint(&dest_key, true);
                self.curr.set_taint_origin(&dest_key, &origin);
            }
        }

        // Generic container / mutable-receiver taint: if the RECEIVER (arg 0) is a
        // `&mut` reference (so the callee can store INTO it) and any OTHER argument is
        // tainted, the tainted value may have been placed into the receiver — so taint
        // the receiver's underlying variable. This follows tainted data into ANY
        // collection/builder/sink-of-bytes WITHOUT naming methods: `Vec::push`,
        // `HashMap::insert`, `String::push_str`, `Writer::write`, a user
        // `add(&mut self, x)`, etc. (Returns `()` typically, so the arg->result rule
        // above can't carry it — the value went sideways into the receiver.) It's what
        // lets a value survive `c.push(tainted); for e in c { sink(e) }`.
        if args.len() >= 2 && !sanitized {
            let recv_is_mut = matches!(
                args[0].node.ty(&self.mir_body.local_decls, self.tcx).kind(),
                TyKind::Ref(_, _, m) if m.is_mut()
            );
            if recv_is_mut {
                if let Some(src) = args[1..].iter().find(|sp| self.operand_tainted(&sp.node)) {
                    if let Operand::Copy(p) | Operand::Move(p) = &args[0].node {
                        let recv_key = self.resolve_alias(&self.place_key(p));
                        // If the receiver currently holds a CONCRETE literal string
                        // (e.g. `let mut p = String::from("/tmp/")`), the mutation just
                        // folded untrusted data in, so the literal is now stale/wrong —
                        // replace it with an unconstrained symbolic so the appended
                        // untrusted part isn't hidden behind the old prefix (covers
                        // `p.push_str(evil)`, `p += evil`, any in-place string append).
                        // A symbolic/guarded value (as_string == None) is left intact.
                        let is_concrete = self
                            .curr
                            .get_string(&recv_key)
                            .and_then(|s| s.as_string())
                            .is_some();
                        if is_concrete {
                            let fresh = self.curr.fresh_string(&recv_key);
                            self.curr.assign_string(&recv_key, fresh);
                        }
                        // The stored value's facts (taint, origin, composed) flow into
                        // the receiver — e.g. pushing a composed path into a Vec/String.
                        let f = self.operand_facts(&src.node);
                        self.apply_value_facts(&recv_key, &f);
                    }
                }
            }
        }

        // Signature-based read/deserialize source: a call that CONSUMES a raw input
        // carrier and returns data is reading/parsing bytes. Two kinds of carrier,
        // treated differently on purpose:
        //   • A HANDLE (File / BufReader): consuming it IS the read — it happens
        //     INSIDE the callee (e.g. `parse_pack(File) -> Pack`). So its result is
        //     untrusted even if the handle value itself wasn't already tainted.
        //   • A BYTE BUFFER (&[u8] / Vec<u8>) needs NO special rule here: the bytes
        //     were read somewhere ELSE, so the buffer is untrusted only if it is
        //     already tainted — and in that case normal taint propagation has already
        //     carried the real source's taint+origin into the parse result. (A
        //     hardcoded/constant buffer is not tainted, so it correctly stays clean.)
        //     So this block only needs to recognize HANDLE reads.
        if !sanitized {
            let dest_ty = self.mir_body.local_decls[dest.local].ty;
            let carries = self.ty_carries_data(dest_ty);
            let handle_read = carries
                && args.iter().any(|sp| {
                    self.is_handle_carrier_ty(sp.node.ty(&self.mir_body.local_decls, self.tcx))
                });
            let origin = || match &callee_path {
                Some(cp) => format!("reads bytes @ {}", Self::short_callee(cp)),
                None => "reads bytes".to_string(),
            };
            if handle_read && !self.curr.is_tainted(&dest_key) {
                // A read whose result isn't tainted yet -> fresh read source; mark the
                // fn an extractor for the reader-provenance gate.
                self.curr.set_taint(&dest_key, true);
                self.curr.set_taint_origin(&dest_key, &origin());
                self.touched_reader = true;
            } else if carries
                && self.curr.is_tainted(&dest_key)
                && args.iter().any(|sp| {
                    self.is_handle_carrier_ty(sp.node.ty(&self.mir_body.local_decls, self.tcx))
                })
            {
                // A genuine file/reader HANDLE (File/BufReader) consumed when the
                // result is ALREADY tainted (e.g. via `File::open(param_path)`): this
                // IS a real file read, so (re)assert the read as the value's origin so
                // downstream attributes to the read, not the param that named the file.
                // Deliberately do NOT flip `touched_reader` here — that would re-open
                // the benign param-FP floor (the gate keys on `param[` sources, which
                // this real "reads bytes" origin already escapes). Handle-only (not the
                // noisy &[u8]/Vec<u8> slice carriers).
                self.curr.set_taint_origin(&dest_key, &origin());
            }
        }

        // control flow
        if let Some(next) = target {
            self.stack.push((self.curr.clone(), next));
        } else if let UnwindAction::Cleanup(clean) = unwind {
            self.stack.push((self.curr.clone(), clean));
        }
    }

    // Hassnain : Removed this function, as we are using a generic string matching fucniton now
    // fn check_write_safety(&self, path_operand: &Operand<'tcx>) -> bool {

    // Extract string value from an operand (constant or symbolic)
    pub(crate) fn get_string_from_operand(
        &self,
        operand: &Operand<'tcx>,
    ) -> Option<z3::ast::String<'ctx>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let key = self.place_key(place);
                let base = self.resolve_alias(&key);
                self.curr.get_string(&base).cloned()
            }
            Operand::Constant(_) => {
                get_operand_const_string(self.tcx, operand).map(|s| self.curr.static_string(&s))
            }
            Operand::RuntimeChecks(_) => None,
        }
    }

    // Extract integer value from an operand
    // Helper function for binary operations
    fn get_int_from_operand(&self, operand: &Operand<'tcx>) -> Option<z3::ast::Int<'ctx>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let key = self.place_key(place);
                self.curr.get_int(&key).cloned()
            }
            Operand::Constant(c) => c
                .const_
                .try_to_scalar_int()
                .map(|si| self.curr.static_int((si.to_int(si.size()) as i64).into())),
            Operand::RuntimeChecks(_) => None,
        }
    }

    // Extract a boolean value from an operand (symbolic variable or constant).
    fn get_bool_from_operand(&self, operand: &Operand<'tcx>) -> Option<z3::ast::Bool<'ctx>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let key = self.place_key(place);
                self.curr.get_bool(&key).cloned()
            }
            Operand::Constant(c) => {
                // `try_to_bool` panics on a non-`bool` scalar (it asserts size 1),
                // and `UnOp::Not` also applies to integers (bitwise NOT), so only
                // attempt the bool read when the operand really is a `bool`.
                if c.const_.ty().is_bool() {
                    c.const_.try_to_bool().map(|b| self.curr.static_bool(b))
                } else {
                    None
                }
            }
            Operand::RuntimeChecks(_) => None,
        }
    }

    // Check if a given execution state has satisfiable constraints
    fn is_path_satisfiable(&self, state: &SymExec<'ctx>) -> bool {
        // Create a temporary solver to check satisfiability
        let solver = z3::Solver::new(&state.context);

        // Add all constraints from the state
        for constraint in &state.constraints {
            solver.assert(constraint);
        }

        // Check if the constraints are satisfiable
        match solver.check() {
            SatResult::Sat => {
                // Path is satisfiable - we can explore it
                true
            }
            SatResult::Unsat => {
                // Path is unsatisfiable - skip it
                false
            }
            SatResult::Unknown => {
                // Can't determine - be conservative and explore it
                true
            }
        }
    }

    fn operand_matches_literal(&self, op: &Operand<'tcx>, lit: &str) -> bool {
        if let Some(sym) = self.get_string_from_operand(op) {
            // Ask Z3: can `sym == lit` be satisfied under current constraints?
            let eq = sym._eq(&self.curr.static_string(lit));
            matches!(self.curr.check_constraint_sat(&eq), SatResult::Sat)
        } else {
            false
        }
    }
}

#[derive(Clone)]
pub struct Call<'tcx> {
    pub func_def_id: DefId,            // DEF ID of the function being called
    pub dest: Place<'tcx>,             // Where the call return value is stored , i.e, _5 in MIR
    pub span: Option<Span>,            // location of the call in the source code
    pub args: Vec<Operand<'tcx>>,      // arguments to the function call
    pub sink: Option<SinkInformation>, // information about the sink - args index and forbidden value
}

type CallHandler<'tcx, 'mir, 'ctx> = fn(&mut MIRParser<'tcx, 'mir, 'ctx>, Call<'tcx>);
