use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::interpret::AllocRange;
use rustc_middle::mir::interpret::ConstAllocation;
use rustc_middle::mir::Rvalue::{self, BinaryOp, Use};
use rustc_middle::mir::{
    BasicBlock, CallSource, Const, ConstValue, Local, Place, SourceInfo, UnwindAction, ProjectionElem , ProjectionKind
};
use rustc_middle::mir::{BinOp, Body, Operand, StatementKind, SwitchTargets, TerminatorKind};
use rustc_middle::ty::ScalarInt;
use rustc_middle::ty::TyKind;
use rustc_session::config::OptLevel::Size;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::process::Termination;
use std::sync::Arc;
use z3::SatResult;

use crate::operand::*;
use crate::symexec::SymExec;

const DEF_ID_FS_WRITE: usize = 2345;

pub struct MIRParser<'a, 'ctx> {
    mir_body: &'a Body<'a>,
    pub curr: SymExec<'ctx>,
    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,
}

impl<'a, 'ctx> MIRParser<'a, 'ctx> {
    pub fn new(mir_body: &'a Body<'a>, z3: SymExec<'ctx>) -> Self {
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
            _ => println!("Unhandled BinOp."),
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
                                Self::parse_int_bin_op(
                                    self_curr,
                                    op,
                                    local,
                                    &left_var.clone(),
                                    &right_var.clone(),
                                ); // How to do w/o cloning?
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
                            let num =
                                Self::get_integer_constant(constant.try_to_scalar_int().unwrap());
                            let right_var = (&self_curr.static_int(num.into())).clone();
                            Self::parse_int_bin_op(
                                self_curr,
                                op,
                                local,
                                &left_var.clone(),
                                &right_var.clone(),
                            ); // How to do w/o cloning?
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

                let local_case = place.local.as_usize().to_string();
                let projection_case = place.projection;

                dbg!("Local Use: {}", &local_case);
                
                if !projection_case.is_empty(){ // https://doc.rust-lang.org/stable/nightly-rustc/rustc_middle/mir/type.ProjectionKind.html
                    return;
                    let projection_case = place.projection;
                    dbg!{"Projection Use: {}", &projection_case}; 
                    for elem in projection_case.iter(){ 
                        match elem {
                            ProjectionElem::Deref => {
                                dbg!("Deref: {:?}", elem);
                            },
                            ProjectionElem::Field(fieldidx, ty) => {
                                dbg!("Field: {:?}", fieldidx.as_usize().to_string());
                                dbg!("ProjectionElem: {:?}", ty); //Ethan i think this is what you need
                                // return ;
                            },
                            ProjectionElem::Index(local) => {
                                dbg!("Index: {:?}", local);
                            },
                            ProjectionElem::ConstantIndex{offset , min_length, from_end} => {
                                dbg!("Offset: {:?}", offset);
                                dbg!("Min Length: {:?}", min_length);
                                dbg!("From End: {:?}", from_end);
                            },
                            ProjectionElem::Subslice{from, to, from_end} => {
                                dbg!("From: {:?}", from);
                                dbg!("To: {:?}", to);
                                dbg!("From End: {:?}", from_end);
                            },
                            ProjectionElem::Downcast(option_symbol, variantidx) => {
                                dbg!("Option Symbol: {:?}", option_symbol);
                                dbg!("Variant Index: {:?}", variantidx.as_usize().to_string());
                                // return None;
                            },
                            ProjectionElem::OpaqueCast(ty) => {
                                dbg!("Opaque Cast: {:?}", ty);
                            },
                            // ProjectionElem::UnwrapUnsafeBinder(ty) => {// the documentation says this is a thing, but compiler complains
                            //     dbg!("Unwrap Unsafe Binder: {:?}", ty);
                            // }
                            ProjectionElem::Subtype(ty) => {
                                dbg!("Subtype: {:?}", ty);
                            }
                        }
                    }
                }

                if let Some(_) = self_curr.get_int(local) {
                    self_curr.assign_int(local, self_curr.get_int(local_case.as_str()).unwrap().clone());
                } else if let Some(_) = self_curr.get_bool(local) {
                    self_curr
                        .assign_bool(local, self_curr.get_bool(local_case.as_str()).unwrap().clone());
                } else if let Some(_) = self_curr.get_string(local) {
                    self_curr.assign_string(
                        local,
                        self_curr.get_string(local_case.as_str()).unwrap().clone(),
                    );
                }
            }
            Operand::Constant(ref constant) => {

                dbg!{"Parsing Use: {}", &constant};
                
                let op = constant.clone();
                let constant = op.const_;
                if let Some(var) = self_curr.get_int(local) {
                    let num = Self::get_integer_constant(constant.try_to_scalar_int().unwrap());
                    self_curr.assign_int(local, self_curr.static_int(num.into()));
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
                        StatementKind::Assign(val) => {
                            Self::parse_assign(&mut self.curr, val.clone())
                        }
                        //_ => println!("unknown statement..."),
                        _ => (),
                    }
                }
                // Terminator
                match &bb_data.terminator().kind {
                    // https://doc.rust-lang.org/beta/nightly-rustc/rustc_middle/mir/enum.TerminatorKind.html
                    TerminatorKind::Goto { target } => self.parse_bb(*target),
                    TerminatorKind::SwitchInt { discr, targets } => {
                        self.parse_switch_int(bb, discr.clone(), targets.clone())
                    }
                    TerminatorKind::UnwindResume => self.parse_return(), //"like a return marks the end of this invocation of function"
                    TerminatorKind::UnwindTerminate (..) => self.parse_return(), // "Indicates that the landing pad is finished and that the process should terminate." (untested)
                    TerminatorKind::Return => self.parse_return(),
                    TerminatorKind::Unreachable => self.parse_return(), //Indicates a terminator that can never be reached, executing this is undefined beahvior. therefore, we ignore it.
                    TerminatorKind::Drop {
                        place,
                        target,
                        unwind,
                        replace,
                    } => self.parse_bb(*target),
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
                    TerminatorKind::TailCall {
                        func, //Operand<'tcx>
                        args,   // Box<[Spanned<Operand<'tcx>>]>
                        fn_span, //Span
                    } =>{
                        unimplemented!("TailCall not implemented. The destination, target and unwind are not provided, they are to be taken from the current stazck frame.")
                        // probably need to make changes to the struture of the stack to accomodate this. Possible soltion make an Enum for the stack frame, with variants for Call and TailCall and then have the stack be a vector of this enum. 
                    },
                    TerminatorKind::Assert{
                        cond ,  // Operand<'tcx>,
                        expected, // bool,
                        msg , // Box<AssertMessage<'tcx>>,
                        target, // BasicBlock,
                        unwind, // UnwindAction,
                    } => {
                        unimplemented!("Assert not implemented")
                    },
                    TerminatorKind::Yield{
                        value, // Operand<'tcx>,
                        resume, // BasicBlock,
                        resume_arg, //Place<'tcx>,
                        drop, // Option<BasicBlock>,
                    } => {
                        unimplemented!("Yield not implemented")
                    },
                    TerminatorKind::CoroutineDrop{ 
                    //Indicates the end of dropping a coroutine. Semantically just a return (from the coroutines drop glue). Only permitted in the same situations as yield. Documentation says , need clarification because it is not even really in the cutrent body and are there type system constraints on these terminators? should there be a 'block type' like cleanup blocks for them? 
                    } => {
                        unimplemented!("CoroutineDrop not implemented")
                    },
                    TerminatorKind::FalseEdge {
                        real_target,
                        imaginary_target,
                    } => self.parse_bb(*real_target), // untested
                    TerminatorKind::FalseUnwind {
                        real_target,
                        unwind,
                    } => self.parse_bb(*real_target), // untested
                    TerminatorKind::InlineAsm {
                        // asm_macro , //InlineAsmMacro - Documentation says this field should be here, but compiler complains when I add this one. 
                        template , //&'tcx[InlineAsmTemplatePiece],
                        operands, // Box<[InlineAsmOperand<'tcx>]>,
                        options, //InlineAsmOptions,
                        line_spans, //&'tcx[Span],
                        targets , //Box<[BasicBlock]>,
                        unwind, //UnwindAction,
                    } =>{
                        unimplemented!("InlineAsm not implemented");
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

    // TODO: Detect cycle/loop
    // TODO: Run loops twice, see if constraints change. Stop when constraints stop changing
    
    // Path Merging 
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

        
        match self.curr.get_bool(local.as_usize().to_string().as_str()) {
            Some(z3_bool) => {
                // TODO: differentiate between local == bool and local != bool (2 >= args in switchInt, match statement)

                // Bool variable -- only 2 arguments
                // Note: The `iter()` does not contain the `otherwise` branch/else branch
                let mut conjunction = self.curr.static_bool(true); // dummy to hold conjunction
                for (value, target) in targets.iter() {
                    // Check reachability of z3_bool == true
                    if self.curr.check_constraint_sat(z3_bool) == z3::SatResult::Sat {
                        println!("{} = true is reachable", local.as_usize().to_string().as_str());
                        let mut cloned_curr = self.curr.clone();
                        cloned_curr.add_constraint(z3_bool.clone());
                        self.stack.push((cloned_curr, target));
                    } else {
                        println!("{} = true is NOT reachable", local.as_usize().to_string().as_str());
                    }
                    // Get negation of bool z3 var
                    let not_z3_bool = self.curr.not(&z3_bool);
                    conjunction = self.curr.and(&conjunction, &not_z3_bool);
                }
                // if/else on bool: [(true, bb1), (otherwise, bb3)]
                // match on i32: [(1, bb3), (2, bb4), (otherwise, bb5)]

                if self.curr.check_constraint_sat(&conjunction) == z3::SatResult::Sat {
                    println!("otherwise/else is reachable");
                    self.curr.add_constraint(conjunction);
                    self.parse_bb(targets.otherwise())
                } else {
                    // Stolen from parse_return
                    // TODO: Make a helper function with better naming to do this instead
                    if let Some((next_curr, next_bb)) = self.stack.pop() {
                        self.curr = next_curr; // Move popped stack value into self.curr
                        // TODO: Probably better to have path_label instead of path_count
                        self.path_count += 1;
                        println!("START: Path {}!", self.path_count);
                        self.parse_bb(next_bb)
                    } else {
                        // There are no more paths! The stack is empty
                        None
                    }
                }
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
            // TODO: Probably better to have path_label instead of path_count
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
