use rustc_middle::mir::{Operand, Place};
use z3::SatResult;

use crate::parser::{Call, MIRParser};

// Hassnain : Removed these function, as we are using a generic string matching fucniton now
// pub(crate) fn handle_fs_write<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
// pub(crate) fn handle_env_set_var<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {

pub(crate) fn handle_pathbuf_from<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
    debug_assert_eq!(call.args.len(), 1);
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
    }
}

pub(crate) fn handle_pathbuf_deref<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    debug_assert_eq!(call.args.len(), 1);
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
    }
}

pub(crate) fn handle_path_join<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
    if call.args.is_empty() {
        return;
    }
    if let (Some(base), Some(comp)) = (
        this.get_string_from_operand(&call.args[0]),
        this.get_string_from_operand(&call.args[1]),
    ) {
        let joined = this.curr.path_join(&base, &comp);
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, joined);
    }
}

pub(crate) fn handle_string_from<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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
pub(crate) fn handle_from_trait<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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

    // extract the string from the argument
    if let Some(sym_str) = this.get_string_from_operand(arg) {
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

            let (could_match, always_match) = if use_regex {
                (
                    // IF there is regex, check for pattern match
                    this.curr.check_string_matches(s, info.forbidden_val) == z3::SatResult::Sat,
                    this.curr.check_string_always_matches(s, info.forbidden_val)
                        == z3::SatResult::Unsat,
                )
            } else {
                (
                    this.curr.could_equal_literal(s, info.forbidden_val) == z3::SatResult::Sat,
                    this.curr.must_equal_literal(s, info.forbidden_val) == z3::SatResult::Unsat,
                )
            };

            // Is the argument source tainted?
            let tainted = this.operand_tainted(arg);

            /*
            Report in two cases
            // Case 1
            i) Value is tainted
            ii) Value may have forbidden value in some executions.
            // Case 2
            i) Value will be forbidden in ALL execution (handle consts)
            */
            if (could_match && tainted) || always_match {
                if let Some(span) = call.span {
                    let func_path = this.def_path_str(call.func_def_id);
                    this.record_sink_hit(&func_path, info.forbidden_val, span);
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
}

pub(crate) fn handle_pathbuf_push<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
    if call.args.len() < 2 {
        return;
    }

    let self_key = match &call.args[0] {
        Operand::Copy(p) | Operand::Move(p) => this.place_key(p),
        Operand::Constant(_) => return,
    };

    // resolve the alias to the original variable , if no alias, return self_key
    let pointee_key = this.resolve_alias(&self_key);

    let base_opt = this.curr.get_string(&pointee_key).cloned();
    let comp_opt = this.get_string_from_operand(&call.args[1]);

    if let (Some(base), Some(comp)) = (base_opt, comp_opt) {
        let joined = this.curr.path_join(&base, &comp);
        this.curr.assign_string(&pointee_key, joined);
        if this.operand_tainted(&call.args[1]) || this.operand_tainted(&call.args[0]) {
            this.curr.set_taint(&pointee_key, true);
        }
    } else if this.operand_tainted(&call.args[1]) || this.operand_tainted(&call.args[0]) {
        this.curr.set_taint(&pointee_key, true);
    }
}

pub(crate) fn handle_path_new<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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
