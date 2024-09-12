use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::interpret::AllocRange;
use rustc_middle::mir::interpret::ConstAllocation;
use rustc_middle::mir::Rvalue::{self, BinaryOp, Use};
use rustc_middle::mir::{
    BasicBlock, CallSource, Const, ConstValue, Local, Place, SourceInfo, UnwindAction,
};
use rustc_middle::mir::{BinOp, Body, Operand, StatementKind, SwitchTargets, TerminatorKind};
use rustc_middle::ty::ScalarInt;
use rustc_middle::ty::TyKind;
use rustc_session::config::OptLevel::Size;
use std::collections::HashMap;
use std::sync::Arc;
use z3::SatResult;

use crate::operand::*;
use crate::symexec::SymExec;

const DEF_ID_FS_WRITE: usize = 2345;

pub struct MIRParser<'a, 'ctx> {
    mir_body: MappedReadGuard<'a, Body<'a>>,
    pub curr: SymExec<'ctx>,
    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,
}

impl<'a, 'ctx> MIRParser<'a, 'ctx> {
    pub fn new(mir_body: MappedReadGuard<'a, Body<'a>>, z3: SymExec<'ctx>) -> Self {
        MIRParser {
            mir_body,
            curr: z3,
            stack: Vec::new(),
            path_count: 0,
        }
    }

    pub fn parse(&mut self) -> Option<rustc_span::Span> {
        // We start traversing the function MIR body from bb0
        println!("START: Path 0!");
        self.parse_bb(BasicBlock::from_usize(0))
    }

    // Update SymExec state off of numeric BinOp
    fn parse_int_bin_op(
        self_curr: &mut SymExec<'ctx>,
        op: BinOp,
        local: &str,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) {
        match op {
            // Logical 
            BinOp::Eq => self_curr.assign_bool(local, self_curr.int_eq(lhs, rhs)),
            BinOp::Ne => self_curr.assign_bool(local, self_curr.not(&self_curr.int_eq(lhs, rhs))),
            BinOp::Lt => self_curr.assign_bool(local, self_curr.int_lt(lhs, rhs)),
            BinOp::Le => self_curr.assign_bool(local, self_curr.int_le(lhs, rhs)),
            BinOp::Gt => self_curr.assign_bool(local, self_curr.int_gt(lhs, rhs)),
            BinOp::Ge => self_curr.assign_bool(local, self_curr.int_ge(lhs, rhs)),
            // TODO: BinOp::Cmp => 
            // Arithmetic
            BinOp::Add => self_curr.assign_int(local, self_curr.add(lhs, rhs)),
            BinOp::Sub => self_curr.assign_int(local, self_curr.sub(lhs, rhs)),
            BinOp::Mul => self_curr.assign_int(local, self_curr.mul(lhs, rhs)),
            BinOp::Div => self_curr.assign_int(local, self_curr.div(lhs, rhs)),
            BinOp::Rem => self_curr.assign_int(local, self_curr.rem(lhs, rhs)),
            // TODO: Unchecked, WithOverflow variants
            //      WithOverflow: outputs a tuple (bool, numeric), where bool signifies
            //      whether there was an overflow or not, can we just always assume there isn't an overflow?
            //      Maybe we only care about overflow when we  have a concrete abstract domain that we
            //      we already evaluate to check if within the bounds.
            //      Handling/predicting when UB happens?
            // TODO: BitXor, BitAnd, BitOr?
            _ => println!("Unhandled BinOp.")
        }
    }

    fn parse_bin_op<'tcx>(
        self_curr: &mut SymExec<'ctx>,
        local: &str,
        op: BinOp,
        operand: Box<(Operand<'tcx>, Operand<'tcx>)>,
    ) {
        let (lhs, rhs): (Operand<'tcx>, Operand<'tcx>) = *operand;
        match lhs {
            Operand::Copy(place) | Operand::Move(place) => {
                let left_local = place.local.as_usize().to_string();
                match rhs {
                    Operand::Copy(place) | Operand::Move(place) => {
                        let right_local = place.local.as_usize().to_string();
                        // BinOp(var, var) of numerics
                        if let Some(left_var) = self_curr.get_int(left_local.as_str()) {
                            if let Some(right_var) = self_curr.get_int(right_local.as_str()) {
                                Self::parse_int_bin_op(self_curr, op, local, &left_var.clone(), &right_var.clone()); // How to do w/o cloning?
                            }
                        // BinOp(var, var) of bools
                        } else if let Some(var) = self_curr.get_bool(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.bool_eq(
                                    var,
                                    self_curr.get_bool(right_local.as_str()).unwrap(),
                                ),
                            );
                        // BinOp(var, var) of strings
                        } else if let Some(var) = self_curr.get_string(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.string_eq(
                                    var,
                                    self_curr.get_string(right_local.as_str()).unwrap(),
                                ),
                            );
                        }
                    }
                    Operand::Constant(ref constant) => {
                        let operand = constant.clone();
                        let constant = operand.const_;
                        // BinOp(var, const) of numerics
                        if let Some(left_var) = self_curr.get_int(left_local.as_str()) {
                            let num = Self::get_integer_constant(constant.try_to_scalar_int().unwrap());
                            let right_var = (&self_curr.static_int(num)).clone();
                            Self::parse_int_bin_op(self_curr, op, local, &left_var.clone(), &right_var.clone()); // How to do w/o cloning?
                        // BinOp(var, const) of bools
                        } else if let Some(var) = self_curr.get_bool(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.bool_eq(
                                    var,
                                    &self_curr.static_bool(constant.try_to_bool().unwrap()),
                                ),
                            );
                        // BinOp(var, const) of strings
                        } else if let Some(var) = self_curr.get_string(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.string_eq(
                                    var,
                                    &self_curr.static_string(
                                        get_operand_const_string(&rhs).unwrap().as_str(),
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
            // Can't BinOp(const, var) appear? Should BinOp(const, const) be handled or is there constant evaluation in `mir_built`?
            Operand::Constant(place) => {}
        }
    }

    fn get_integer_constant(constant: ScalarInt) -> i64 {
        constant.to_int(constant.size()) as i64
    }

    // Handle StatementKind::Assign Use, simple a = Use(b)
    fn parse_use<'tcx>(self_curr: &mut SymExec<'ctx>, local: &str, operand: Operand<'tcx>) {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let place = place.local.as_usize().to_string();
                if let Some(_) = self_curr.get_int(local) {
                    self_curr.assign_int(local, self_curr.get_int(place.as_str()).unwrap().clone());
                } else if let Some(_) = self_curr.get_bool(local) {
                    self_curr
                        .assign_bool(local, self_curr.get_bool(place.as_str()).unwrap().clone());
                } else if let Some(_) = self_curr.get_string(local) {
                    self_curr.assign_string(
                        local,
                        self_curr.get_string(place.as_str()).unwrap().clone(),
                    );
                }
            }
            Operand::Constant(ref constant) => {
                let op = constant.clone();
                let constant = op.const_;
                if let Some(var) = self_curr.get_int(local) {
                    let num = Self::get_integer_constant(constant.try_to_scalar_int().unwrap());
                    self_curr.assign_int(local, self_curr.static_int(num));
                } else if let Some(var) = self_curr.get_bool(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.static_bool(constant.try_to_bool().unwrap()),
                    );
                } else if let Some(var) = self_curr.get_string(local) {
                    self_curr.assign_string(
                        local,
                        self_curr
                            .static_string(get_operand_const_string(&operand).unwrap().as_str()),
                    );
                }
            }
        }
    }

    fn parse_assign<'tcx>(self_curr: &mut SymExec<'ctx>, val: Box<(Place<'tcx>, Rvalue<'tcx>)>) {
        let (place, val) = *val;
        let binding = place.local.as_usize().to_string();
        let local = binding.as_str();
        match val {
            Use(operand) => Self::parse_use(self_curr, local, operand), // Ethan fix this :)
            BinaryOp(op, operand) => Self::parse_bin_op(self_curr, local, op, operand),
            // _ => println!("unknown assignment operation"),
            _ => (),
        }
    }

    pub fn parse_bb(&mut self, bb: BasicBlock) -> Option<rustc_span::Span> {
        match self.mir_body.basic_blocks.get(bb) {
            Some(bb_data) => {
                println!("\tbb{}", bb.as_u32());
                // Statements
                for statement in &bb_data.statements {
                    match &statement.kind {
                        StatementKind::Assign(val) => Self::parse_assign(&mut self.curr, val.clone()),
                        //_ => println!("unknown statement..."),
                        _ => (),
                    }
                }
                // Terminator
                match &bb_data.terminator().kind {
                    TerminatorKind::Goto { target } => self.parse_bb(*target),
                    TerminatorKind::SwitchInt { discr, targets } => {
                        self.parse_switch_int(bb, discr.clone(), targets.clone())
                    }
                    TerminatorKind::Call {
                        func,        // <Operand<'tcx>>
                        args,        //Box<[Spanned<Operand<'tcx>>]>
                        destination, // Place<'tcx>
                        target,      // Option<BasicBlock>
                        unwind,      // UnwindAction
                        call_source, // CallSource
                        fn_span,     // Span
                    } => self.parse_call(
                        func.clone(),
                        args.clone(),
                        destination.clone(),
                        target.clone(),
                        unwind.clone(),
                        call_source.clone(),
                    ),
                    TerminatorKind::Drop {
                        place,
                        target,
                        unwind,
                        replace,
                    } => self.parse_bb(*target),
                    TerminatorKind::FalseUnwind {
                        real_target,
                        unwind,
                    } => self.parse_bb(*real_target), // untested
                    TerminatorKind::FalseEdge {
                        real_target,
                        imaginary_target,
                    } => self.parse_bb(*real_target), // untested
                    TerminatorKind::Return => self.parse_return(),
                    _ => {
                        println!("Encountered Unknown Terminator. Results may be incorrect.");
                        None
                    } 
                    // TODO: Handle Assert, maybe we can just go down the success path?
                    // TODO: When does Unreachable appear? ex5 contains `unreachable` in bb3
                    //      Indicates a terminator that can never be reached.
                    //      Executing this terminator is UB.
                    
                }
            }
            // ERROR: Couldn't find the bb we were supposed to process.
            // This shouldn't ever happen unless the MIR CFG is faulty.
            None => {
                eprintln!("I couldn't find the bb :(");
                None
            }
        }
    }

    pub fn parse_switch_int(
        &mut self,
        curr_bb: BasicBlock,
        discr: Operand,
        targets: SwitchTargets,
    ) -> Option<rustc_span::Span> {
        // Fetch the LHS Local variable, this will be important for updating PC
        let local: Local;
        match discr {
            Operand::Copy(place) => local = place.local,
            Operand::Move(place) => local = place.local,
            Operand::Constant(_) => return None, // don't know when constant is used yet? if (true)?
        }

        // TODO: differentiate between local == bool and local != bool (2 >= args in switchInt)
        match self.curr.get_bool(local.as_usize().to_string().as_str()) {
            Some(z3_bool) => {
                // Bool variable -- only 2 arguments
                for (value, target) in targets.iter() {
                    // Get negation of bool z3 var
                    let curr_constraint = self.curr.not(&z3_bool);
                    // Check reachability of `false` constraint
                    if self.curr.check_constraint_sat(&curr_constraint) == z3::SatResult::Sat {
                        println!("\tCreating a clone @ bb{}", curr_bb.as_u32());
                        let mut cloned_curr = self.curr.clone();
                        cloned_curr.add_constraint(curr_constraint);
                        self.stack.push((cloned_curr, target));
                    } else {
                        println!("\tIgnoring path @ bb{}", curr_bb.as_u32());
                    }
                }
                // Traverse `true` constraint
                self.curr.constraints.push(z3_bool.clone());
                self.parse_bb(targets.otherwise())
            }
            None => {
                // TODO: at least handle switchInts on numerics...
                // Some numerics get produced from something like `_7 = discriminant(_4)` where _4 is a weird type like Result
                // Might just need to ignore these cases. Don't know semantics of discr.
                // In the meantime, just go down all targets but don't add any constraints.
                for (_, target) in targets.iter() {
                    self.stack.push((self.curr.clone(), target));
                }
                self.parse_bb(targets.otherwise())
            }
        }
    }

    pub fn parse_return(&mut self) -> Option<rustc_span::Span> {
        // Replace curr with stack top
        // dbg!("Debugging: {}", &self.curr);
        if let Some((next_curr, next_bb)) = self.stack.pop() {
            self.curr = next_curr; // Move popped stack value into self.curr
            self.path_count += 1;
            println!("START: Path {}!", self.path_count);
            self.parse_bb(next_bb)
        } else {
            // There are no more paths! The stack is empty
            None
        }
    }

    // TODO: Handle String comparison overloaded function calls, possibly also with PathBuf comparison
    pub fn parse_call<'tcx>(
        &mut self,
        func: Operand<'tcx>,
        args: Box<[rustc_span::source_map::Spanned<Operand<'tcx>>]>,
        destination: Place<'tcx>,
        target: Option<BasicBlock>,
        unwind: UnwindAction,
        call_source: CallSource,
    ) -> Option<rustc_span::Span> {
        let func_def_id = get_operand_def_id(&func); //passing it func, gives def_id
                                                     // println!("Func DefId: {:?}", func_def_id);
                                                     // Detected fs::write!
        if func_def_id == Some(DEF_ID_FS_WRITE) {
            // println!("Found function DefId in call: {:?}", def_id);
            println!("\tFound fs::write call");
            let first_arg = get_operand_local(&args[0].node);
            match first_arg {
                Some(arg) => {
                    let result: Result<SatResult, &str>; // To decide whether the file write is malicious
                    if arg == 0 {
                        // Constant
                        let arg = get_operand_const_string(&args[0].node);
                        let obj = self.curr.static_string(arg.unwrap().as_str());
                        result = self.curr.is_write_safe(&obj);
                    } else {
                        // Variable
                        let arg = self.curr.get_string(arg.to_string().as_str()).unwrap();
                        result = self.curr.is_write_safe(arg);
                    }
                    // Can the string be proc/self/mem?
                    match result {
                        Ok(sat_result) => match sat_result {
                            z3::SatResult::Sat => {
                                // need to return a span here, because write to /proc/self/mem is a safety violation
                                return get_operand_span(&func);
                            }
                            z3::SatResult::Unsat => {}
                            z3::SatResult::Unknown => {}
                        },
                        Err(e) => {
                            println!(
                                "An error occurred in parse_call, contact Hassnain and Anirudh: {}",
                                e
                            );
                        }
                    }
                }
                None => {
                    println!(
                        "Parse Call: This should never happen, contact Hassnain if this is printed"
                    )
                }
            }
        }
        self.parse_bb(target.unwrap())
    }
}
