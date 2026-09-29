use rustc_middle::{
    mir::{Operand, Place},
    ty::TyKind,
};
use z3::SatResult;

use crate::operand::get_operand_const_string;
use crate::parser::{Call, MIRParser};

// Hassnain : Removed these function, as we are using a generic string matching fucniton now
// pub(crate) fn handle_fs_write<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
// pub(crate) fn handle_env_set_var<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {

// Hassnain : These are the functions that have been replaced / upgraded
// pub(crate) fn handle_pathbuf_deref<'tcx, 'mir, 'ctx>( --> handle_generic_deref()

pub(crate) fn handle_pathbuf_from<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    debug_assert_eq!(call.args.len(), 1);
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
    }
}

pub(crate) fn handle_path_join<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.len() < 2 {
        return;
    }
    // Entry-vs-base provenance: the result of a join is a COMPOSED path (a base with a
    // component appended), regardless of whether we could model the concrete strings —
    // so a later `create_dir_all`/`copy` on it (or its `.parent()`) is not treated as a
    // bare output-dir. Marked unconditionally when a component is joined on.
    let key = this.place_key(&call.dest);
    this.curr.set_composed(&key);
    if let (Some(base), Some(comp)) = (
        this.get_string_from_operand(&call.args[0]),
        this.get_string_from_operand(&call.args[1]),
    ) {
        let joined = this.curr.path_join(&base, &comp);
        this.curr.assign_string(&key, joined);
    }
}

pub(crate) fn handle_string_from<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // should have one argument
    if call.args.is_empty() {
        return;
    }

    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        // Write the symbolic / concrete string into the destination Place
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);

        // If the argument was tainted, the new String is tainted, too.
        if this.operand_tainted(&call.args[0]) {
            this.curr.set_taint(&key, true);
        }
    }
}

// Handle the `From` trait for String and PathBuf
pub(crate) fn handle_from_trait<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.is_empty() {
        return;
    }

    // get the destination type
    let dest_ty = this.mir_body.local_decls[call.dest.local].ty;

    // if destination string
    let is_string = match dest_ty.kind() {
        rustc_middle::ty::TyKind::Adt(adt, _) => {
            this.tcx.def_path_str(adt.did()).ends_with("string::String")
        }
        _ => false,
    };

    // if destination is PathBuf
    let is_pathbuf = match dest_ty.kind() {
        rustc_middle::ty::TyKind::Adt(adt, _) => {
            this.tcx.def_path_str(adt.did()).ends_with("path::PathBuf")
        }
        _ => false,
    };

    // if neither is true, we don't handle this
    if !is_string && !is_pathbuf {
        return;
    }

    // pull the string from arg 0
    if let Some(val) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, val);

        // propagate taint from the arg to the dest
        if this.operand_tainted(&call.args[0]) {
            this.curr.set_taint(&key, true);
        }
    }
}

pub(crate) fn generic_string_handler<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // Which arg to look at (defaults to 0 if no SinkInformation)
    let idx = call.sink.map(|s| s.arg_idx).unwrap_or(0);
    let Some(arg) = call.args.get(idx) else {
        return;
    };

    // Extract the string value of the argument. If the value is TAINTED but has no
    // modeled string (e.g. it was built by an unmodeled combiner like `[..].join(..)`
    // that only set the taint flag), fall back to a fresh UNCONSTRAINED symbolic
    // string so the sink can still test "could this be `..`?". Safe for guards: a
    // guarded value carries a stored symbolic (with its constraints), so this branch —
    // which only fires when there is NO stored string — never discards a constraint.
    let sym_str = this.get_string_from_operand(arg).or_else(|| {
        if this.operand_tainted(arg) {
            match arg {
                Operand::Copy(p) | Operand::Move(p) => Some(this.curr.fresh_string(&this.place_key(p))),
                _ => None,
            }
        } else {
            None
        }
    });
    if let Some(sym_str) = sym_str {
        let dest_key = this.place_key(&call.dest);
        this.curr.assign_string(&dest_key, sym_str.clone());

        // propagate taint from the arg to the dest
        if this.operand_tainted(arg) {
            this.curr.set_taint(&dest_key, true);
        }

        if let Some(info) = call.sink {
            let s: &z3::ast::String<'ctx> = &sym_str;
            // let dest_expr = this.curr.get_string(&dest_key).unwrap();
            let use_regex = info.forbidden_val.contains('*');
            let tainted = this.operand_tainted(arg);

            // Report in two cases:
            //  - TAINTED value that COULD take the forbidden shape in some
            //    execution (the taint-flow finding). We only run the cheap
            //    positive query here; `Unknown` (solver timeout) is treated as
            //    "could" so a timeout can never hide a real finding.
            //  - UNTAINTED value that is FORCED to the forbidden shape in ALL
            //    executions (a hardcoded constant). The negated query is only run
            //    in this branch — it is the expensive one on symbolic strings, but
            //    an untainted sink argument is almost always a concrete constant,
            //    so it stays cheap and we avoid the costly query on the hot path.
            // A concrete example value the tainted arg could take at the sink, pulled
            // from Z3's model when the finding fires — "what value can it be here."
            let mut example: Option<String> = None;
            let fire = if tainted {
                let r = if use_regex {
                    let (r, w) = this.curr.witness_string_matches(s, info.forbidden_val);
                    example = w;
                    r
                } else {
                    this.curr.could_equal_literal(s, info.forbidden_val)
                };
                matches!(r, z3::SatResult::Sat | z3::SatResult::Unknown)
            } else {
                let r = if use_regex {
                    this.curr.check_string_always_matches(s, info.forbidden_val)
                } else {
                    this.curr.must_equal_literal(s, info.forbidden_val)
                };
                r == z3::SatResult::Unsat
            };

            if fire {
                if let Some(span) = call.span {
                    let func_path = this.def_path_str(call.func_def_id);
                    // Source = where the tainted argument came from; for an
                    // always-match constant there is no taint source, so label it.
                    let source = this
                        .operand_origin(arg)
                        .or_else(|| this.curr.path_taint_origin.clone())
                        .unwrap_or_else(|| if tainted { "unknown".into() } else { "constant".into() });
                    // Entry-vs-base routing (see `record_sink_hit_prov`): a base-position
                    // sink fed a BARE value is deferred and only kept if the value proves
                    // to be a real target path elsewhere in the function.
                    let composed = this.operand_composed(arg);
                    this.record_sink_hit_prov(&source, &func_path, info.forbidden_val, span, example, composed);
                }
            }
        }
    }
}

// Hassnain : Removed these two becuase we are using handle_generic_source now
// pub(crate) fn handle_env_args<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
// pub(crate) fn handle_env_var<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {

pub(crate) fn handle_generic_source<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    let key = this.place_key(&call.dest);
    this.curr.set_taint(&key, true);
    // Record where this taint originates (the source function), generics stripped
    // for readability, so a downstream finding can name it.
    let source = crate::matching::strip_generics(&this.def_path_str(call.func_def_id));
    this.curr.set_taint_origin(&key, &source);
    // Bind a persistent fresh symbolic Z3 string so downstream guards
    // (contains/starts_with/ends_with -> str_* constraints) and joins can reason
    // about this value's CONTENT in the solver. Without it the value has only a
    // taint flag and no string the solver can constrain, so a guard could never
    // make the sink query UNSAT. Recall-safe: only ever adds a symbolic string.
    if this.curr.get_string(&key).is_none() {
        this.curr.create_uninterpreted_string(&key);
    }
    // Reading untrusted archive/stream data (an entry-name accessor, a file/socket
    // read) marks the function as an extractor, so its public-param findings are
    // kept; a CLI/env source does not (see `run_body`'s reader-provenance gate).
    if crate::matching::is_reader_provenance_source(&source) {
        this.touched_reader = true;
    }
}

pub(crate) fn handle_pathbuf_push<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.len() < 2 {
        return;
    }

    let self_key = match &call.args[0] {
        Operand::Copy(p) | Operand::Move(p) => this.place_key(p),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => return,
    };

    // resolve the alias to the original variable , if no alias, return self_key
    let pointee_key = this.resolve_alias(&self_key);

    // Entry-vs-base: pushing a component makes the receiver a COMPOSED path.
    this.curr.set_composed(&pointee_key);

    let base_opt = this.curr.get_string(&pointee_key).cloned();
    let comp_opt = this.get_string_from_operand(&call.args[1]);

    if let (Some(base), Some(comp)) = (base_opt, comp_opt) {
        let joined = this.curr.path_join(&base, &comp);
        this.curr.assign_string(&pointee_key, joined);
    }
    if this.operand_tainted(&call.args[1]) || this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&pointee_key, true);
        // Provenance: prefer the pushed component, else the receiver.
        let o = this
            .operand_origin(&call.args[1])
            .or_else(|| this.operand_origin(&call.args[0]));
        if let Some(o) = o {
            this.curr.set_taint_origin(&pointee_key, &o);
        }
    }
}

pub(crate) fn handle_path_new<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // new<T: AsRef<OsStr>>(s: T) -> &Path
    if call.args.is_empty() {
        return;
    }
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
        if this.operand_tainted(&call.args[0]) {
            this.curr.set_taint(&key, true);
        }
    }
}

pub(crate) fn handle_path_to_path_buf<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // &Path -> PathBuf (dest)
    if call.args.is_empty() {
        return;
    }
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
        if this.operand_tainted(&call.args[0]) {
            this.curr.set_taint(&key, true);
        }
    }
}

pub(crate) fn handle_string_from_utf8_lossy<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // String::from_utf8_lossy(&[u8]) -> Cow<'_, str>
    if call.args.is_empty() {
        return;
    }

    let dest_key = this.place_key(&call.dest);

    // If we can see the string from the arg, reuse it - otherwise make new
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        this.curr.assign_string(&dest_key, s);
    } else {
        // dbg!("think Hassnain!!"); - leaving this comment here, since I find it funny
        // the solution below is what I thought of
        // IF we cannot see a string from the arg, we create a z3 string that can be anything
        let sym = this.curr.get_or_fresh_string(&dest_key);
        this.curr.assign_string(&dest_key, sym);
    }

    // Propagate taint: &[u8] input taints the Cow<str> result.
    if this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&dest_key, true);
    }
}

pub(crate) fn handle_read_into_buf<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // std::io::Read::{read, read_exact}(&mut self, buf: &mut [u8])
    // buffer is at index = 1
    if call.args.len() < 2 {
        return;
    }
    if let Operand::Copy(p) | Operand::Move(p) = &call.args[1] {
        let key = this.place_key(p);
        let base = this.resolve_alias(&key);

        // mark both the handle and the underlying buffer as tainted, recording
        // the read as the taint's origin
        let source = crate::matching::strip_generics(&this.def_path_str(call.func_def_id));
        this.curr.set_taint(&key, true); // &mut [u8]
        this.curr.set_taint_origin(&key, &source);
        this.curr.set_taint(&base, true); // [u8; N] backing array
        this.curr.set_taint_origin(&base, &source);
    }
}

// Calling format calles these three
// core::fmt::rt::Argument::<'a>::new_display -> get's called one time for each arg with {} in format
// std::fmt::Arguments::<'a>::new_v1 -> once upper one is done with all, this gets called
// std::fmt::format -> finally, this gets called to actually format the string

pub(crate) fn handle_fmt_arg_new_display<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // I think an idea here would be to keep track of all the constraint that comes with args in some hashmap
}

pub(crate) fn handle_fmt_arguments_new_v1<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // this one get's called once, so it has all the args, once we have all of them, we should be combining the constraints or something here?
    // TODO fix later
}

pub(crate) fn handle_fmt_format<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    // this should store the result of the formatting in actual hashmap
}

pub(crate) fn handle_string_from_utf8<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.is_empty() {
        return;
    }
    let dest_key = this.place_key(&call.dest);
    // see if you can get a string from the argument, if not make a new one
    let s = this.curr.get_or_fresh_string(&dest_key);
    this.curr.assign_string(&dest_key, s);

    // If the Vec<u8> came from the network, taint the Result
    if this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&dest_key, true);
    }
}

pub(crate) fn handle_result_unwrap_or_default<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.is_empty() {
        return;
    }

    let src_key = match &call.args[0] {
        Operand::Copy(p) | Operand::Move(p) => this.place_key(p),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => return,
    };

    let dest_key = this.place_key(&call.dest);

    if let Some(s) = this.curr.get_string(&src_key).cloned() {
        // Reuse the symbolic string
        this.curr.assign_string(&dest_key, s);
    } else {
        // if no string, make a new one , so we can add constraints on it later
        let s = this.curr.get_or_fresh_string(&dest_key);
        this.curr.assign_string(&dest_key, s);
    }
    if this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&dest_key, true);
    }
}

pub(crate) fn handle_deref_mut<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.is_empty() {
        return;
    }

    // self is the Vec<T>
    let self_key = match &call.args[0] {
        Operand::Copy(p) | Operand::Move(p) => this.place_key(p),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => return,
    };

    // dest is &mut [T]
    let dest_key = this.place_key(&call.dest);

    // Point the temp (&mut [T]) back to the underlying Vec<T>
    let base = this.resolve_alias(&self_key);
    this.aliases.insert(dest_key.clone(), base.clone());

    // If Vec was tainted, the slice is tainted and keep vec tainted as well
    if this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&dest_key, true);
        this.curr.set_taint(&base, true);
    }
}

pub(crate) fn handle_deref_generic<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.is_empty() {
        return;
    }

    // `deref` passes the pointee through unchanged, so propagate the tracked string,
    // the alias, and taint for ANY deref (String->&str, PathBuf->&Path, Box<_>, ...).
    // Previously this was gated to PathBuf->&Path only, which left `name.contains("..")`
    // (via `<String as Deref>::deref`) with no tracked string, so string-predicate
    // guards could never constrain the value. Recall-safe: only ever adds tracking.
    let self_key = match &call.args[0] {
        Operand::Copy(p) | Operand::Move(p) => this.place_key(p),
        Operand::Constant(_) | Operand::RuntimeChecks(_) => return,
    };
    let dest_key = this.place_key(&call.dest);

    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        this.curr.assign_string(&dest_key, s);
    }
    // Alias the deref temp back to the base so later lookups resolve to it.
    let base = this.resolve_alias(&self_key);
    this.aliases.insert(dest_key.clone(), base.clone());
    if this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&dest_key, true);
    }
}

// --- String-predicate modeling: tie contains/starts_with/ends_with to the tracked
// Z3 string so guard branches add real content constraints (see SymExec::str_*).
// Recall-safe: each only ADDS a modeled bool; if the string isn't tracked or the
// needle isn't a literal it does nothing, leaving the prior opaque-bool behavior.

pub(crate) fn handle_str_contains<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.len() < 2 {
        return;
    }
    let Some(s) = this.get_string_from_operand(&call.args[0]) else {
        return;
    };
    let Some(needle) = get_operand_const_string(this.tcx, &call.args[1]) else {
        return;
    };
    let b = this.curr.str_contains_lit(&s, &needle);
    let dest = this.place_key(&call.dest);
    this.curr.assign_bool(&dest, b);
}

pub(crate) fn handle_str_starts_with<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.len() < 2 {
        return;
    }
    let (Some(s), Some(prefix)) = (
        this.get_string_from_operand(&call.args[0]),
        this.get_string_from_operand(&call.args[1]),
    ) else {
        return;
    };
    let b = this.curr.str_starts_with(&s, &prefix);
    let dest = this.place_key(&call.dest);
    this.curr.assign_bool(&dest, b);
}

pub(crate) fn handle_str_ends_with<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    if call.args.len() < 2 {
        return;
    }
    let (Some(s), Some(suffix)) = (
        this.get_string_from_operand(&call.args[0]),
        this.get_string_from_operand(&call.args[1]),
    ) else {
        return;
    };
    let b = this.curr.str_ends_with(&s, &suffix);
    let dest = this.place_key(&call.dest);
    this.curr.assign_bool(&dest, b);
}
