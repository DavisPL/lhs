use rustc_middle::mir::{
    BasicBlock, BinOp, Body, CallSource, Operand, Place, ProjectionElem, Rvalue, StatementKind,
    SwitchTargets, TerminatorKind, UnwindAction,
};

use rustc_hir::def_id::DefId;
use rustc_middle::ty::{data_structures::IndexMap, TyCtxt};

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

#[derive(Clone, Copy, Debug)]
pub struct SinkInformation {
    pub arg_idx: usize,
    pub forbidden_val: &'static str,
}

pub struct MIRParser<'tcx, 'mir, 'ctx>
where
    'mir: 'tcx,
{
    mir_body: &'mir Body<'tcx>,
    pub curr: SymExec<'ctx>,

    // Stack for iterative basic block processing
    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,

    // Loop handling: track how many times we've visited each basic block
    visit_counts: HashMap<BasicBlock, u32>,

    // Collection of all dangerous write locations found during analysis
    // TODO: Vec<AnalysisResult>
    // Or: HashMap<(String, Operand), AnalysisResult>
    dangerous_spans: HashMap<(String, String), Vec<Span>>,
    aliases: HashMap<String, String>, // Hashmap for aliases check

    // registry of “interesting” callees → handler
    handlers: IndexMap<String, (CallHandler<'tcx, 'mir, 'ctx>, Vec<SinkInformation>)>,
    tcx: TyCtxt<'tcx>,
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
        };

        // built-ins we always want
        p.add_builtin_handlers();
        p
    }

    fn record_sink_hit(&mut self, func_path: &str, arg: &str, span: Span) {
        self.dangerous_spans
            .entry((func_path.to_string(), arg.to_string()))
            .or_default()
            .push(span);
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
        for (path, arg_idx, forbidden) in SINK_FUNCTION_ARGS {
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
        self.register_handler("std::path::PathBuf::deref", handle_pathbuf_deref);
        self.register_handler("std::path::Path::new", handle_path_new);
        self.register_handler("std::path::Path::to_path_buf", handle_path_to_path_buf);
        self.register_handler("std::ops::Deref::deref", handle_pathbuf_deref);
        self.register_handler("core::ops::deref::Deref::deref", handle_pathbuf_deref);
        self.register_handler("std::path::Path::join", handle_path_join);
        self.register_handler("std::path::PathBuf::push", handle_pathbuf_push);

        //some traits that are used implicitly
        self.register_handler("core::convert::From::from", handle_from_trait);
        self.register_handler("std::convert::From::from", handle_from_trait);

        self.register_handler("alloc::string::String::from", handle_string_from);
        self.register_handler("std::string::String::from", handle_string_from);
        self.register_handler("std::ffi::OsString::from", handle_string_from);
    }

    fn operand_tainted(&self, op: &Operand<'tcx>) -> bool {
        // If the path is tainted, everything is considered tainted
        if self.curr.path_taint {
            return true;
        }
        match op {
            Operand::Copy(p) | Operand::Move(p) => self.curr.is_tainted(&self.place_key(p)),
            Operand::Constant(_) => false,
        }
    }

    // Main entry point: analyze the MIR and return all dangerous write locations
    pub fn parse(&mut self) -> HashMap<(String, String), Vec<Span>> {
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

        self.dangerous_spans.clone()
    }

    fn def_path_str(&self, def_id: DefId) -> String {
        self.tcx.def_path_str(def_id)
    }

    // Convert a Place (memory location + projections) into a stable string key
    // Example: _1.field[2] becomes "1.f0[2]"
    fn place_key(&self, place: &Place<'tcx>) -> String {
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
                ProjectionElem::Subtype(_) => key.push_str("::sub"),
            }
        }
        key
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
            TerminatorKind::Return
            | TerminatorKind::Unreachable
            | TerminatorKind::CoroutineDrop
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate { .. }
            | TerminatorKind::TailCall { .. } => {
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
            Rvalue::Aggregate(_, operands) => {
                // For single-operand aggregates (like PathBuf wrapping a string), copy the value
                if operands.len() == 1 {
                    if let Some((_, operand)) = operands.iter_enumerated().next() {
                        if let Operand::Copy(place) | Operand::Move(place) = operand {
                            let src_key = self.place_key(&place);
                            self.copy_variable_value(&src_key, &dest_key);
                        }
                    }
                }
            }

            // Copy for dereference: used in some compiler optimizations
            Rvalue::CopyForDeref(place) => {
                self.handle_copy_for_deref(&dest_key, &place);
            }

            // Other operations we don't currently handle
            _ => {
                // println!("Unsupported Rvalue in assignment: {:?}", rvalue);
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
            self.handle_int_binary_op(dest_key, op, &lhs_int, &rhs_int);

            if let Some(result) = self.curr.get_int(dest_key) {
            } else if let Some(result) = self.curr.get_bool(dest_key) {
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
    ) {
        use BinOp::*;
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
                // println!("Unsupported binary operation: {:?}", op);
            }
        }
    }

    // Handle reference operations: `x = &y`
    // This is important for tracking when PathBuf objects are borrowed as &Path
    fn handle_reference_operation(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
        self.copy_variable_value(&src_key, dest_key);
        self.curr.propagate_taint(&src_key, dest_key);
        // need to keep track of the aliases as well, so updates can be properly applied
        self.aliases.insert(dest_key.to_string(), src_key);
    }

    // Resolve an alias to its original variable , if no alias exists return the variable back
    fn resolve_alias(&self, key: &str) -> String {
        self.aliases
            .get(key)
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }

    // Handle cast operations: `x = y as T`
    // Needed for various type conversions in path operations
    fn handle_cast_operation(&mut self, dest_key: &str, operand: &Operand<'tcx>) {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            let src_key = self.place_key(place);
            self.copy_variable_value(&src_key, dest_key);
            self.curr.propagate_taint(&src_key, dest_key);
        }
    }

    // Handle copy for dereference operations
    // Used in some compiler optimizations
    fn handle_copy_for_deref(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
        self.copy_variable_value(&src_key, dest_key);
        self.curr.propagate_taint(&src_key, dest_key); // I think this is needed
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
        self.curr.propagate_taint(src_key, dest_key);
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
            get_operand_const_string(&Operand::Constant(Box::new(constant.clone())))
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
            Operand::Constant(_) => return, // Can't branch on constant
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
                true_state.path_taint = true;
                false_state.path_taint = true;
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
            for (_, bb) in targets.iter() {
                let mut st = self.curr.clone();
                if self.operand_tainted(&discr) {
                    st.path_taint = true;
                }
                self.stack.push((st, bb));
            }
            let mut st = self.curr.clone();
            if self.operand_tainted(&discr) {
                st.path_taint = true;
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
                    success_state.path_taint = true;
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
                        failure_state.path_taint = true;
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
                    st.path_taint = true;
                }
                self.stack.push((st, target));
            }
        } else {
            // Can't analyze condition
            let mut st = self.curr.clone();
            if self.operand_tainted(&cond) {
                st.path_taint = true;
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
        self.handlers
            .iter()
            .filter(|(k, _)| path.starts_with(k.as_str()) || path.ends_with(k.as_str()))
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
        if let Some(def_id) = get_operand_def_id(&func) {
            let path = self.def_path_str(def_id);

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
        }

        // taint propagation
        let dest_key = self.place_key(&dest);
        if args.iter().any(|sp| self.operand_tainted(&sp.node)) {
            self.curr.set_taint(&dest_key, true);
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
    fn get_string_from_operand(&self, operand: &Operand<'tcx>) -> Option<z3::ast::String<'ctx>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let key = self.place_key(place);
                self.curr.get_string(&key).cloned()
            }
            Operand::Constant(_) => {
                get_operand_const_string(operand).map(|s| self.curr.static_string(&s))
            }
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

// Hassnain : Removed these function, as we are using a generic string matching fucniton now
// fn handle_fs_write<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
// fn handle_env_set_var<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {

fn handle_pathbuf_from<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
    debug_assert_eq!(call.args.len(), 1);
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
    }
}

fn handle_pathbuf_deref<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    debug_assert_eq!(call.args.len(), 1);
    if let Some(s) = this.get_string_from_operand(&call.args[0]) {
        let key = this.place_key(&call.dest);
        this.curr.assign_string(&key, s);
    }
}

fn handle_path_join<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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

fn handle_string_from<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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
fn handle_from_trait<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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

fn generic_string_handler<'tcx, 'mir, 'ctx>(
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
// fn handle_env_args<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
// fn handle_env_var<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {

fn handle_generic_source<'tcx, 'mir, 'ctx>(
    this: &mut MIRParser<'tcx, 'mir, 'ctx>,
    call: Call<'tcx>,
) {
    let key = this.place_key(&call.dest);
    this.curr.set_taint(&key, true);
}

fn handle_pathbuf_push<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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

fn handle_path_new<'tcx, 'mir, 'ctx>(this: &mut MIRParser<'tcx, 'mir, 'ctx>, call: Call<'tcx>) {
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

fn handle_path_to_path_buf<'tcx, 'mir, 'ctx>(
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
