use rustc_middle::mir::{
    BasicBlock, BinOp, Body, CallSource, Operand, Place, ProjectionElem, Rvalue, StatementKind,
    SwitchTargets, TerminatorKind, UnwindAction,
};
use z3::SatResult;

use crate::operand::{
    get_operand_const_string, get_operand_def_id, get_operand_local, get_operand_span,
};
use crate::symexec::SymExec;
use std::collections::{HashMap, HashSet};

const DEF_ID_FS_WRITE: usize = 2_345;       // std::fs::write
const DEF_PATHBUF_FROM: usize = 3_072;      // PathBuf::from - for PathBuf construction
const DEF_PATHBUF_DEREF: usize = 3_557;     // PathBuf::deref - for PathBuf to &Path conversion (apparently when we do a.join and if a .path is a PathBuf, .deref is called on a internally - this gave me a lot of trouble) 
const DEF_ID_JOIN: usize = 5_328;           // Path::join - for path joining operations

const MAX_LOOP_ITER: u32 = 5; // Maximum loop iterations before widening

pub struct MIRParser<'mir, 'ctx> {
    mir_body: &'mir Body<'mir>,
    pub curr: SymExec<'ctx>,
    
    // Stack for iterative basic block processing
    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,
    
    // Loop handling: track how many times we've visited each basic block
    visit_counts: HashMap<BasicBlock, u32>,
    
    // Collection of all dangerous write locations found during analysis
    dangerous_spans: Vec<rustc_span::Span>,
}

impl<'mir, 'ctx> MIRParser<'mir, 'ctx> {
    pub fn new(body: &'mir Body<'mir>, z3: SymExec<'ctx>) -> Self {
        Self {
            mir_body: body,
            curr: z3,
            stack: Vec::new(),
            path_count: 0,
            visit_counts: HashMap::new(),
            dangerous_spans: Vec::new(),
        }
    }

    // Main entry point: analyze the MIR and return all dangerous write locations
    pub fn parse(&mut self) -> Vec<rustc_span::Span> {
        println!("=== Starting MIR Analysis ===");
        // Start symbolic execution from the entry block (bb0)
        self.stack.push((self.curr.clone(), BasicBlock::from_usize(0)));
        println!("START: Path 0!");

        // Process all execution paths iteratively
        while let Some((state, bb)) = self.stack.pop() {
            self.curr = state;

            if let Some(is_terminal) = self.parse_bb_iterative(bb) {
                if is_terminal {
                    self.path_count += 1;
                    println!("START: Path_{}!", self.path_count);
                }
            }
        }

        // Report results
        if !self.dangerous_spans.is_empty() {
            println!("WARNING: \nFound {} dangerous writes to /proc/self/mem", self.dangerous_spans.len());
            for (i, span) in self.dangerous_spans.iter().enumerate() {
                println!("  [{}] {:?}", i + 1, span);
            }
        } else {
            println!("\nNo dangerous writes detected");
        }

        self.dangerous_spans.clone()
    }

    // Convert a Place (memory location + projections) into a stable string key
    // Example: _1.field[2] becomes "1.f0[2]"  
    fn place_key<'tcx>(&self, place: &Place<'tcx>) -> String {
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
            println!("bb{} exceeded limit {} — applying widening", bb.as_u32(), MAX_LOOP_ITER);
            
            // Widening: remove constraints on variables modified in this block
            let written_vars = self.collect_written_vars(bb);
            self.curr.constraints.retain(|c| !Self::constraint_mentions(&written_vars, c));
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
    fn handle_terminator(&mut self, terminator: &TerminatorKind) {
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
                func, args, destination, target, unwind, call_source, ..
            } => {
                self.handle_function_call(
                    func.clone(),
                    args.clone(), 
                    destination.clone(),
                    *target,
                    (*unwind).clone(),
                    call_source.clone(),
                );
            }

            // Runtime assertions
            TerminatorKind::Assert { cond, expected, target, unwind, .. } => {
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

            TerminatorKind::InlineAsm { targets, unwind, .. } => {
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
    fn parse_assignment<'tcx>(&mut self, assignment: Box<(Place<'tcx>, Rvalue<'tcx>)>) {
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
                println!("Unsupported Rvalue in assignment: {:?}", rvalue);
            }
        }
    }

    // Handle simple copy/move operations
    fn handle_use_operation<'tcx>(&mut self, dest_key: &str, operand: &Operand<'tcx>) {
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
    fn handle_binary_operation<'tcx>(
        &mut self,
        dest_key: &str,
        op: BinOp,
        operands: &(Operand<'tcx>, Operand<'tcx>),
    ) {
        let (lhs, rhs) = operands;

        // Try to get integer operands
        if let (Some(lhs_int), Some(rhs_int)) = (
            self.get_int_from_operand(lhs),
            self.get_int_from_operand(rhs)
        ) {      
            self.handle_int_binary_op(dest_key, op, &lhs_int, &rhs_int);
            
            if let Some(result) = self.curr.get_int(dest_key) {
                // println!("    Result: {} = {}", dest_key, result.to_string());
            } else if let Some(result) = self.curr.get_bool(dest_key) {
                // println!("    Result: {} = {}", dest_key, result.to_string());
            }
            return;
        }

        // Handle string comparisons
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            if let (Some(lhs_str), Some(rhs_str)) = (
                self.get_string_from_operand(lhs),
                self.get_string_from_operand(rhs)
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
    }

    // Handle integer binary operations
    fn handle_int_binary_op(&mut self, dest_key: &str, op: BinOp, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) {
        use BinOp::*;
        match op {
            // Comparisons return booleans
            Eq => self.curr.assign_bool(dest_key, self.curr.int_eq(lhs, rhs)),
            Ne => self.curr.assign_bool(dest_key, self.curr.not(&self.curr.int_eq(lhs, rhs))),
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
                self.curr.assign_bool(&field1_key, self.curr.static_bool(false));
        }
            
            _ => {
                println!("Unsupported binary operation: {:?}", op);
            }
        }
    }

    // Handle reference operations: `x = &y`
    // This is important for tracking when PathBuf objects are borrowed as &Path
    fn handle_reference_operation<'tcx>(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
        self.copy_variable_value(&src_key, dest_key);
    }

    // Handle cast operations: `x = y as T`
    // Needed for various type conversions in path operations
    fn handle_cast_operation<'tcx>(&mut self, dest_key: &str, operand: &Operand<'tcx>) {
        if let Operand::Copy(place) | Operand::Move(place) = operand {
            let src_key = self.place_key(place);
            self.copy_variable_value(&src_key, dest_key);
        }
    }

    // Handle copy for dereference operations
    // Used in some compiler optimizations
    fn handle_copy_for_deref<'tcx>(&mut self, dest_key: &str, place: &Place<'tcx>) {
        let src_key = self.place_key(place);
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
    }

    // Assign a constant value to a variable
    fn assign_constant_value<'tcx>(&mut self, dest_key: &str, constant: &rustc_middle::mir::ConstOperand<'tcx>) {
        let const_val = &constant.const_;
        
        // Try different constant types
        if let Some(scalar_int) = const_val.try_to_scalar_int() {
            let int_val = scalar_int.to_int(scalar_int.size()) as i64;
            let z3_int = self.curr.static_int(int_val.into());
            self.curr.assign_int(dest_key, z3_int);
        } else if let Some(bool_val) = const_val.try_to_bool() {
            self.curr.assign_bool(dest_key, self.curr.static_bool(bool_val));
        } else if let Some(string_val) = get_operand_const_string(&Operand::Constant(Box::new(constant.clone()))) {
            self.curr.assign_string(dest_key, self.curr.static_string(&string_val));
        } else {
            println!("    Could not assign constant to {} - unrecognized type", dest_key);
        }
    }

    // conditional branch handling with satisfiability checking
    // prevents exploring unsatisfiable paths
    fn handle_switch_int(&mut self, discr: Operand, targets: SwitchTargets) {
        let local = match discr {
            Operand::Copy(place) | Operand::Move(place) => place.local,
            Operand::Constant(_) => return, // Can't branch on constant
        };

        let local_key = local.as_usize().to_string();
        
        if let Some(bool_condition) = self.curr.get_bool(&local_key).cloned() {
            // println!("    Found boolean condition: {}", bool_condition.to_string());
            
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

            true_state.add_constraint(true_constraint);
            if self.is_path_satisfiable(&true_state) {
                println!("    Taking true branch to bb{}", bb0.as_u32());
                self.stack.push((true_state, bb0));
            } else {
                println!("    Skipping unsatisfiable true branch to bb{}", bb0.as_u32());
            }

            // Check if the false branch is satisfiable before exploring it
            false_state.add_constraint(false_constraint);
            if self.is_path_satisfiable(&false_state) {
                println!("    Taking false branch to bb{}", bb_else.as_u32());
                self.stack.push((false_state, bb_else));
            } else {
                println!("    Skipping unsatisfiable false branch to bb{}", bb_else.as_u32());
            }
        } else {
            //  Debug when no boolean condition is found
            println!("    No boolean condition found, exploring all branches");
            // Unknown condition: explore all branches
            for (_, bb) in targets.iter() {
                self.stack.push((self.curr.clone(), bb));
            }
            self.stack.push((self.curr.clone(), targets.otherwise()));
        }
    }

    // Enhanced runtime assertion handling with satisfiability checking
    fn handle_assert(&mut self, cond: Operand, expected: bool, target: BasicBlock, unwind: UnwindAction) {
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
                
                // Only explore the success path if it's satisfiable
                if self.is_path_satisfiable(&success_state) {
                    println!("  Taking assertion success path to bb{}", target.as_u32());
                    self.stack.push((success_state, target));
                } else {
                    println!("  Skipping unsatisfiable assertion success path to bb{}", target.as_u32());
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
                    
                    // Only explore the failure path if it's satisfiable
                    if self.is_path_satisfiable(&failure_state) {
                        println!("  Taking assertion failure path to bb{}", cleanup_bb.as_u32());
                        self.stack.push((failure_state, cleanup_bb));
                    } else {
                        println!("  Skipping unsatisfiable assertion failure path to bb{}", cleanup_bb.as_u32());
                    }
                }
            } else {
                // Unknown condition: assume assertion passes
                self.stack.push((self.curr.clone(), target));
            }
        } else {
            // Can't analyze condition
            self.stack.push((self.curr.clone(), target));
        }
    }

    // Handle function calls - this is completely rewritten to detect path operations
    fn handle_function_call<'tcx>(
        &mut self,
        func: Operand<'tcx>,
        args: Box<[rustc_span::source_map::Spanned<Operand<'tcx>>]>,
        destination: Place<'tcx>,
        target: Option<BasicBlock>,
        unwind: UnwindAction,
        _call_source: CallSource,
    ) {
        let def_id = get_operand_def_id(&func);
        let dest_key = self.place_key(&destination);

        match def_id {
            // PathBuf::from(string) -> PathBuf
            // This creates a PathBuf from a string literal or variable
            Some(DEF_PATHBUF_FROM) if !args.is_empty() => {
                if let Some(input_str) = self.get_string_from_operand(&args[0].node) {
                    self.curr.assign_string(&dest_key, input_str);
                }
            }

            // PathBuf::deref(&self) -> &Path  
            // This converts a PathBuf reference to a Path reference
            Some(DEF_PATHBUF_DEREF) if !args.is_empty() => {
                if let Some(pathbuf_str) = self.get_string_from_operand(&args[0].node) {
                    self.curr.assign_string(&dest_key, pathbuf_str);
                }
            }

            // Path::join(&self, component) -> PathBuf
            // This is the key operation that creates complex paths like "base/component"
            Some(DEF_ID_JOIN) if args.len() >= 2 => {
                let base_str = self.get_string_from_operand(&args[0].node);
                let component_str = self.get_string_from_operand(&args[1].node);

                if let (Some(base), Some(component)) = (base_str, component_str) {
                    // Using the path_join function from SymExec to create symbolic path
                    let joined_path = self.curr.path_join(&base, &component);
                    self.curr.assign_string(&dest_key, joined_path);
                }
            }

            // fs::write(path, contents)
            Some(DEF_ID_FS_WRITE) if !args.is_empty() => {
                let is_dangerous = self.check_write_safety(&args[0].node);
                
                if is_dangerous {
                    // Collect the span instead of returning immediately
                    if let Some(span) = get_operand_span(&func) {
                        println!("Found dangerous write at {:?}", span);
                        self.dangerous_spans.push(span);
                    }
                }
            }

            _ => {
                // Unhandled function calls
            }
        }

        // Continue execution to the next block
        if let Some(next_bb) = target {
            self.stack.push((self.curr.clone(), next_bb));
        } else if let UnwindAction::Cleanup(cleanup_bb) = unwind {
            self.stack.push((self.curr.clone(), cleanup_bb));
        }
    }

    // Extracted function to check if a write operation is dangerous
    // This separates the safety checking logic from the main function call handler
    fn check_write_safety<'tcx>(&self, path_operand: &Operand<'tcx>) -> bool {
        match get_operand_local(path_operand) {
            // Case 1: Direct constant string (local index 0 means it's a constant)
            Some(0) => {
                if let Some(s) = get_operand_const_string(path_operand) {
                    s == "/proc/self/mem"
                } else {
                    false
                }
            }
            // Case 2: Variable containing a symbolic string
            Some(idx) => {
                if let Some(sym_str) = self.curr.get_string(&idx.to_string()) {
                    // Use Z3 to check if this symbolic string could equal "/proc/self/mem"
                    matches!(self.curr.is_write_safe(sym_str), Ok(SatResult::Sat))
                } else {
                    false
                }
            }
            // Case 3: Cannot determine operand type
            None => false,
        }
    }

    // Extract string value from an operand (constant or symbolic)
    fn get_string_from_operand<'tcx>(&self, operand: &Operand<'tcx>) -> Option<z3::ast::String<'ctx>> {
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
    fn get_int_from_operand<'tcx>(&self, operand: &Operand<'tcx>) -> Option<z3::ast::Int<'ctx>> {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let key = self.place_key(place);
                self.curr.get_int(&key).cloned()
            }
            Operand::Constant(c) => {
                c.const_.try_to_scalar_int().map(|si| {
                    self.curr.static_int((si.to_int(si.size()) as i64).into())
                })
            }
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
                println!("    Warning: Z3 returned Unknown for satisfiability check");
                true
            }
        }
    }
}