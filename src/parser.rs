extern crate rustc_abi;
extern crate rustc_data_structures;
extern crate rustc_middle;
use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::Rvalue::{self, BinaryOp, Use};
use rustc_middle::mir::{
    BasicBlock, CallSource, Const, ConstValue, Local, Place, SourceInfo, UnwindAction,
};
use rustc_middle::mir::{BinOp, Body, Operand, StatementKind, SwitchTargets, TerminatorKind};
use rustc_middle::ty::TyKind;
use std::collections::HashMap;
use std::sync::Arc;

use rustc_middle::mir::interpret::AllocRange;

use rustc_middle::mir::interpret::ConstAllocation;
use rustc_session::config::OptLevel::Size;

use z3::SatResult;

// use rustc_abi::Size;
// use rustc_hash::Size;

// use rustc_span::span_encoding::Span;

#[path = "../z3/src/symexec.rs"]
pub mod symexec;

const DEF_ID_FS_WRITE: usize = 2345;

pub struct MIRParser<'a, 'ctx> {
    mir_body: MappedReadGuard<'a, Body<'a>>,
    pub curr: symexec::SymExec<'ctx>,
    stack: Vec<(symexec::SymExec<'ctx>, BasicBlock)>,
}

impl<'a, 'ctx> MIRParser<'a, 'ctx> {
    pub fn new(mir_body: MappedReadGuard<'a, Body<'a>>, z3: symexec::SymExec<'ctx>) -> Self {
        MIRParser {
            mir_body,
            curr: z3,
            stack: Vec::new(),
        }
    }

    pub fn parse(&mut self) -> Option<rustc_span::Span> {
        // We start traversing the function MIR body from bb0
        self.parse_bb(BasicBlock::from_usize(0))
    }

    fn eq_op<'tcx>(
        self_curr: &mut symexec::SymExec<'ctx>,
        local: &str,
        lhs: Operand<'tcx>,
        rhs: Operand<'tcx>,
    ) {
        match lhs {
            Operand::Copy(place) | Operand::Move(place) => {
                let left_local = place.local.as_usize().to_string();
                // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
                match rhs {
                    Operand::Copy(place) | Operand::Move(place) => {
                        let right_local = place.local.as_usize().to_string();
                        // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
                        if let Some(var) = self_curr.get_int(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.int_equals(
                                    var,
                                    self_curr.get_int(right_local.as_str()).unwrap(),
                                ),
                            );
                        } else if let Some(var) = self_curr.get_bool(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.bool_equals(
                                    var,
                                    self_curr.get_bool(right_local.as_str()).unwrap(),
                                ),
                            );
                        } else if let Some(var) = self_curr.get_string(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.string_equals(
                                    var,
                                    self_curr.get_string(right_local.as_str()).unwrap(),
                                ),
                            );
                        }
                    }
                    Operand::Constant(ref constant) => {
                        let operand = constant.clone();
                        let constant = operand.const_;
                        // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
                        if let Some(var) = self_curr.get_int(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.int_equals(
                                    var,
                                    &self_curr
                                        .static_int(constant.try_to_scalar_int().unwrap().to_i64()),
                                ),
                            );
                        } else if let Some(var) = self_curr.get_bool(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.bool_equals(
                                    var,
                                    &self_curr.static_bool(constant.try_to_bool().unwrap()),
                                ),
                            );
                        } else if let Some(var) = self_curr.get_string(left_local.as_str()) {
                            self_curr.assign_bool(
                                local,
                                self_curr.string_equals(
                                    var,
                                    &self_curr.static_string(
                                        Self::parse_operand_get_const_string(&rhs)
                                            .unwrap()
                                            .as_str(),
                                    ),
                                ),
                            );
                        }
                    }
                }
            }
            Operand::Constant(place) => {}
        }
    }

    fn bin_op<'tcx>(
        self_curr: &mut symexec::SymExec<'ctx>,
        local: &str,
        op: BinOp,
        operand: Box<(Operand<'tcx>, Operand<'tcx>)>,
    ) {
        let (lhs, rhs) = *operand;
        match op {
            BinOp::Eq => Self::eq_op(self_curr, local, lhs, rhs),
            _ => println!("unknown binary operation"),
        }
    }

    fn r#use<'tcx>(self_curr: &mut symexec::SymExec<'ctx>, local: &str, operand: Operand<'tcx>) {
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
                // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
                if let Some(var) = self_curr.get_int(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.int_equals(
                            var,
                            &self_curr.static_int(constant.try_to_scalar_int().unwrap().to_i64()),
                        ),
                    );
                } else if let Some(var) = self_curr.get_bool(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.bool_equals(
                            var,
                            &self_curr.static_bool(constant.try_to_bool().unwrap()),
                        ),
                    );
                } else if let Some(var) = self_curr.get_string(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.string_equals(
                            var,
                            &self_curr.static_string(
                                Self::parse_operand_get_const_string(&operand)
                                    .unwrap()
                                    .as_str(),
                            ),
                        ),
                    );
                }
            }
            _ => println!("unsupported"),
        }
    }

    fn assignment<'tcx>(
        self_curr: &mut symexec::SymExec<'ctx>,
        val: Box<(Place<'tcx>, Rvalue<'tcx>)>,
    ) {
        let (place, val) = *val;
        let binding = place.local.as_usize().to_string();
        let local = binding.as_str();
        match val {
            Use(operand) => Self::r#use(self_curr, local, operand),
            BinaryOp(op, operand) => Self::bin_op(self_curr, local, op, operand),
            _ => println!("unknown assignment operation"),
        }
    }

    pub fn parse_bb(&mut self, bb: BasicBlock) -> Option<rustc_span::Span> {
        match self.mir_body.basic_blocks.get(bb) {
            Some(bb_data) => {
                // Statements
                for statement in &bb_data.statements {
                    match &statement.kind {
                        StatementKind::Assign(val) => Self::assignment(&mut self.curr, val.clone()),
                        _ => println!("unknown statement..."),
                    }
                }
                // Terminator
                match &bb_data.terminator().kind {
                    TerminatorKind::Goto { target } => self.parse_bb(*target),
                    TerminatorKind::SwitchInt { discr, targets } => self.parse_switch_int(discr.clone(), targets.clone()),
                    TerminatorKind::Call {
                        func,        // <Operand<'tcx>>
                        args,        //Box<[Spanned<Operand<'tcx>>]>
                        destination, // Place<'tcx>
                        target,      // Option<BasicBlock>
                        unwind,      // UnwindAction
                        call_source, // CallSource
                        fn_span,     // Span
                    } => {
                        self.parse_call(
                            func.clone(),
                            args.clone(),
                            destination.clone(),
                            target.clone(),
                            unwind.clone(),
                            call_source.clone(),
                            // fn_span.clone(),
                        )
                    }
                    TerminatorKind::Return => self.parse_return(),
                    _ => {println!("unknown terminator"); None},
                    // TODO: Handle drop and other terminators that are focused on unwinding
                }
            }
            // ERROR: Couldn't find the bb we were supposed to process
            None => {eprintln!("I couldn't find the bb :("); None},
        }
    }

    pub fn parse_switch_int(&mut self, discr: Operand, targets: SwitchTargets) -> Option<rustc_span::Span> {
        // Fetch the LHS Local variable, this will be important for updating PC
        let local: Local;
        match discr {
            Operand::Copy(place) => local = place.local,
            Operand::Move(place) => local = place.local,
            Operand::Constant(_) => return None, // don't know when constant is used yet? if (true)?
        }
        // Vector to keep track of !a && !b -> !a && !b && !c -> etc. for all targets
        let mut curr_pc: Vec<z3::ast::Bool<'ctx>> = Vec::new();
        // TODO: differentiate between local == bool and local != bool (2 >= args in switchInt)
        for (value, target) in targets.iter() {
            // Get bool z3 var
            let curr_constraint = self.curr
                .get_bool(local.as_usize().to_string().as_str())
                .unwrap() // This shouldn't panic, the bool Local should exist
                .clone();
            // If path is reachable, make a clone and add to stack
            if self.curr.check_constraint_sat(&curr_constraint) == z3::SatResult::Sat {
                let mut cloned_curr = self.curr.clone();
                cloned_curr.add_constraint(curr_constraint.clone());
                self.stack.push((cloned_curr, target));
            }
            // Add negated constraint to vector
            curr_pc.push(self.curr.logical_not(&curr_constraint));
        }
        // -----> We take the otherwise branch (right to left DFS... for now?)
        // Update curr PC and take its path
        self.curr.constraints.append(&mut curr_pc);
        self.parse_bb(targets.otherwise())
    }

    pub fn parse_return(&mut self) -> Option<rustc_span::Span> {
        // Replace curr with stack top
        if let Some((next_curr, next_bb)) = self.stack.pop() {
            self.curr = next_curr; // Move popped stack value into self.curr
            self.parse_bb(next_bb)
        } else {
            // There are no more paths! The stack is empty
            None
        }
    }

    pub fn parse_call<'tcx>(
        &mut self,
        func: Operand<'tcx>,
        args: Box<[rustc_span::source_map::Spanned<Operand<'tcx>>]>,
        destination: Place<'tcx>,
        target: Option<BasicBlock>,
        unwind: UnwindAction,
        call_source: CallSource,
        // fn_span: Span,
    ) -> Option<rustc_span::Span> {
        let func_def_id = Self::parse_operand_get_def_id(&func); //passing it func, gives def_id
                                                                 // println!("Func DefId: {:?}", func_def_id);

        if func_def_id == Some(DEF_ID_FS_WRITE) {
            // println!("Found function DefId in call: {:?}", def_id);
            println!("Found fs::write call");
            let mut first_arg = self.parse_operand(&args[0].node);
            if first_arg == Some(1) {
                let arg = Self::parse_operand_get_const_string(&args[0].node);
                let obj = self.curr.static_string(arg.unwrap().as_str());
                let result = self.curr.is_write_safe(&obj);
                match result {
                    Ok(sat_result) => match sat_result {
                        z3::SatResult::Sat => {
                            // println!("The result is SAT.");
                            // need to return a span here, because write to /proc/self/mem is a safety violation
                            return self.get_span_from_operand(&func);
                        }
                        z3::SatResult::Unsat => {}
                        z3::SatResult::Unknown => {}
                    },
                    Err(e) => {
                        println!(
                            "An error occurred parse_call , contact Hassnain and Anirudh: {}",
                            e
                        );
                    }
                }
            } else {
                // println!("First Arg: {:?}", first_arg);
                match first_arg {
                    Some(arg) => {
                        let arg = self.curr.get_string(arg.to_string().as_str()).unwrap();
                        let result = self.curr.is_write_safe(arg);
                        match result {
                            Ok(sat_result) => match sat_result {
                                z3::SatResult::Sat => {
                                    // println!("The result is SAT.");
                                    // need to return a span here, because write to /proc/self/mem is a safety violation
                                    return self.get_span_from_operand(&func);
                                }
                                z3::SatResult::Unsat => {}
                                z3::SatResult::Unknown => {}
                            },
                            Err(e) => {
                                println!(
                                "An error occurred parse_call , contact Hassnain and Anirudh: {}",
                                e
                            );
                            }
                        }
                    }
                    None => {
                        println!("Parse Call : This should never happen, contact Hassnain if this is printed");
                    }
                }
            }
        }
        self.parse_bb(target.unwrap())

        // self.parse_args(&args);

        // self.parse_args(&args);
        // I can get the args, but do i need to do something about this, or just call sovler? How will solver know that i have to check for these variables, do I create a z3 model?

        // println!("Destination: {:?}", destination); //_4 what does this mean?

        // println!("Target: {:?}", target); //this is a basic block, so do i just call parse_bb for it?
        // parse_bb(target.unwrap());
        // call parse_bb for target

        // println!("Unwind: {:?}", unwind); //this is also a basic block, so do i just call parse_bb for it?, but this is for unwind, do we need this? this is _4 in this case - we ignore unwinding

        // println!("Call Source: {:?}", call_source); // https://doc.rust-lang.org/beta/nightly-rustc/rustc_middle/mir/syntax/enum.CallSource.html do we care about this?
    }

    fn parse_operand_get_def_id<'tcx>(operand: &Operand<'tcx>) -> Option<usize> {
        match operand {
            Operand::Copy(_place) => {
                println!(
                    "Parse Operand : This should never happen, contact Hassnain if this is printed"
                );
                None
            }
            Operand::Move(place) => None,
            Operand::Constant(place) => {
                let constant = place.const_;
                match constant {
                    Const::Ty(_ty, _const) => {
                        println!("Parse Operand : This should never happen, contact Hassnain if this is printed");
                        None
                    }
                    Const::Unevaluated(_unevaluated_const, _ty) => {
                        println!("Parse Operand : This should never happen, contact Hassnain if this is printed");
                        None
                    }
                    Const::Val(const_value, ty) => {
                        if let TyKind::FnDef(def_id, idk) = ty.kind() {
                            return Some(def_id.index.as_usize());
                        }
                        None
                    }
                }
            }
        }
    }

    fn parse_operand_get_const_string<'tcx>(operand: &Operand<'tcx>) -> Option<String> {
        match operand {
            Operand::Copy(_place) => {
                println!(
                    "Parse Operand : This should never happen, contact Hassnain if this is printed"
                );
                None
            }
            Operand::Move(place) => {
                println!(
                    "Parse Operand : This should never happen, contact Hassnain if this is printed"
                );
                None
            }
            Operand::Constant(place) => {
                let constant = place.const_;
                match constant {
                    Const::Ty(_ty, _const) => {
                        println!("Parse Operand : This should never happen, contact Hassnain if this is printed");
                        None
                    }
                    Const::Unevaluated(_unevaluated_const, _ty) => {
                        println!("Parse Operand : This should never happen, contact Hassnain if this is printed");
                        None
                    }
                    Const::Val(const_value, ty) => {
                        match const_value {
                            ConstValue::Slice { data, meta } => {
                                if let Some(str_data) = Self::extract_string_from_const(&data, meta)
                                {
                                    return Some(str_data);
                                }
                            }
                            _ => {
                                println!("Parse Operand : This should never happen, contact Hassnain if this is printed");
                            }
                        }
                        None
                    }
                }
            }
        }
    }

    fn parse_operand<'tcx>(&self, operand: &Operand<'tcx>) -> Option<usize> {
        match operand {
            Operand::Copy(_place) => {
                println!(
                    "Parse Operand : This should never happen, contact Hassnain if this is printed"
                );
                None
            }
            Operand::Move(place) => {
                let local = place.local;
                let projection = place.projection;
                // println!("Local: {:?}", local); // ths is the variable number like _1, _2 etc.
                return Some(local.as_usize());
            }
            Operand::Constant(place) => {
                // println!("here I am ");
                Some(1)
                // None
            }
        }
    }

    fn extract_string_from_const<'tcx>(
        data: &'tcx ConstAllocation<'tcx>, //pub struct ConstAllocation<'tcx>(pub Interned<'tcx, Allocation>);
        meta: u64,
    ) -> Option<String> {
        println!("Data: {:?}", data);
        println!("Meta: {:?}", meta);

        //0: Interned<'tcx, Allocation>
        let range: AllocRange = AllocRange {
            start: rustc_abi::Size::from_bytes(0),
            size: rustc_abi::Size::from_bytes(meta),
        };
        let allocation = &data.0.get_bytes_unchecked(range); //this is alignment

        //https://doc.rust-lang.org/beta/nightly-rustc/rustc_middle/mir/interpret/allocation/struct.Allocation.html
        //this is probably what we need

        // pub struct Allocation<Prov: Provenance = CtfeProvenance, Extra = (), Bytes = Box<[u8]>> {
        //     bytes: Bytes,
        //     provenance: ProvenanceMap<Prov>,
        //     init_mask: InitMask,
        //     pub align: Align,
        //     pub mutability: Mutability, if mutable or not, bool
        //     pub extra: Extra,
        // }

        //if i do 0.0 available fields are: `align`, `mutability`, `extra`
        //if i do 0.1, i have PrivateZst

        // this is interned
        // pub struct Interned<'a, T>(pub &'a T, pub PrivateZst);

        // this is allocation
        // pub struct Allocation<Prov: Provenance = CtfeProvenance, Extra = (), Bytes = Box<[u8]>> {
        //     bytes: Bytes,
        //     provenance: ProvenanceMap<Prov>,
        //     init_mask: InitMask,
        //     pub align: Align,
        //     pub mutability: Mutability,
        //     pub extra: Extra,
        // }

        // let allocation = allocation.0.1;

        let a: String = String::from_utf8(allocation.to_vec()).unwrap();

        // println!("Allocation: {:?}", a);
        return Some(a);

        // Typically, the `Interned<Allocation>` type has methods to access the allocation data
        // You might need to use an API method like `inspect` or something similar to get the bytes

        // Assuming we have a method to get the raw bytes of the allocation
        // let string_length = meta as usize;

        // None
    }

    fn parse_args<'tcx>(
        &self,
        args: &[rustc_span::source_map::Spanned<Operand<'tcx>>],
    ) -> Option<usize> {
        for arg in args {
            println!("Arg: {:?}", arg.node);
            let string = self.parse_operand(&arg.node);
            println!("String: {:?}", string);
            //should reurn the variable name or value?
        }
        None
    }

    fn get_span_from_operand(&self, operand: &Operand) -> Option<rustc_span::Span> {
        match operand {
            Operand::Copy(_place) => {
                println!("get_span_from_operand : Unsupported, This funciton currently caters only for constants. ");
                return None;
            }
            Operand::Move(place) => {
                println!("get_span_from_operand : Unsupported, This funciton currently caters only for constants. ");
                // rustc_span::Span::DUMMY
                None
            }
            Operand::Constant(place) => {
                let const_span = place.span;
                // println!("Const Span: {:?}", const_span);
                return Some(const_span);
            }
        }
    }
}
// }
