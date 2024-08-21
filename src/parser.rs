extern crate rustc_data_structures;
extern crate rustc_middle;

use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::Body;
use rustc_middle::mir::Rvalue;
use rustc_middle::mir::{
    BasicBlock, CallSource, Const, ConstValue, Local, Place, SourceInfo, UnwindAction,
};
use rustc_middle::mir::{Operand, SwitchTargets};
use rustc_middle::mir::{StatementKind, TerminatorKind};
use rustc_middle::ty::TyKind;
use std::collections::HashMap;
use std::sync::Arc;
// use rustc_span::span_encoding::Span;

#[path = "../z3/src/symexec.rs"]
pub mod symexec;

/*
pub struct Environment {
    type_map: HashMap<Local, u32>,
    pc: Vec<String>,
}

impl Environment {
    fn new() -> Self {
        Environment {
            type_map: HashMap::new(),
            pc: Vec::new(),
        }
    }
    fn clone(&self) -> Self {
        Environment {
            type_map: self.type_map.clone(),
            pc: self.pc.clone(),
        }
    }
}
*/

pub struct MIRParser<'a, 'ctx> {
    mir_body: MappedReadGuard<'a, Body<'a>>,
    curr: symexec::SymExec<'ctx>,
    stack: Vec<(symexec::SymExec<'ctx>, BasicBlock)>,
    arg_count: usize,
}

impl<'a, 'ctx> MIRParser<'a, 'ctx> {
    pub fn new(mir_body: MappedReadGuard<'a, Body<'a>>, z3: symexec::SymExec<'ctx>) -> Self {
        let arg_count = mir_body.arg_count;
        MIRParser {
            mir_body,
            curr: z3,
            stack: Vec::new(),
            arg_count,
        }
    }

    pub fn parse(&mut self) {
        // We start traversing the function MIR body from bb0
        self.parse_bb(BasicBlock::from_usize(0));
    }

    // fn get_operand(o: Operand<'tcx>) {
    // }

    // use rustc_middle::mir::syntax::BinOp;
    // fn bin_op(
    //     &mut self,
    //     op: BinOp,
    //     operand: Box<(Operand<'tcx>, Operand<'tcx>)>,
    // ) {
    //     let lhs = todo!();
    //     match op {
    //         BinOp::Eq => todo!(),
    //         _ => println!("unknown binary operation"),
    //     }
    // }

    fn assignment<'tcx>(&mut self, val: Box<(Place<'tcx>, Rvalue<'tcx>)>) {
        let (place, val) = *val;
        let local = place.local.as_usize();
        match val {
            // BinaryOp(op, operand) => self.bin_op(op, operand),
            _ => println!("unknown assignment operation"),
        }
    }

    pub fn parse_bb(&mut self, bb: BasicBlock) {
        match self.mir_body.basic_blocks.get(bb) {
            Some(bb_data) => {
                // Statements
                for statement in &bb_data.statements {
                    match &statement.kind {
                        StatementKind::Assign(val) => self.assignment(val.clone()),
                        _ => println!("unknown statement..."),
                    }
                }
                // Terminator
                match &bb_data.terminator().kind {
                    TerminatorKind::Goto { target } => self.parse_bb(*target),
                    TerminatorKind::SwitchInt { discr, targets } => {
                        self.parse_switch_int(discr.clone(), targets.clone())
                    }
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
                        );
                    }
                    TerminatorKind::Return => self.parse_return(),
                    _ => println!("unknown terminator"),
                }
            }
            // ERROR: Couldn't find the bb we were supposed to process
            None => eprintln!("I couldn't find the bb :("),
        }
    }

    pub fn parse_switch_int(&mut self, discr: Operand, targets: SwitchTargets) {
        // Fetch the LHS Local variable, this will be important for updating PC
        let local: Local;
        match discr {
            Operand::Copy(place) => local = place.local,
            Operand::Move(place) => local = place.local,
            Operand::Constant(_) => return, // don't know when constant is used yet? if (true)?
        }
        // will need to make a vector here to keep track of !a && !b -> !a && !b && !c -> etc. for all targets
        let mut curr_pc: Vec<z3::ast::Bool<'ctx>> = Vec::new();
        for (value, target) in targets.iter() {
            // Make a clone of curr
            let mut cloned_curr = self.curr.clone();
            // Update the clone's PC
            // cloned_curr
            //     .constraints
            //     .push(format!("{} = {}", local.as_usize(), value)); // this is temp wrong and definitely not general
            //                                                         // Append to Negation PC vector for the otherwise branch
            let curr_constraint = cloned_curr
                .get_bool(local.as_usize().to_string().as_str())
                .unwrap()
                .clone();
            //static_bool(local.as_usize() != value as usize);
            println!("{:#?}", curr_constraint);

            cloned_curr.add_constraint(curr_constraint.clone());

            curr_pc.push(self.curr.logical_not(&curr_constraint));

            // curr_pc.push(format!("{} != {}", local.as_usize(), value));
            // Push updated clone to parser's stack
            self.stack.push((cloned_curr, target));
        }
        // -----> We take the otherwise branch (right to left DFS... for now?)
        // Update current PC
        self.curr.constraints.append(&mut curr_pc);
        // Then move current to the next bb
        self.parse_bb(targets.otherwise());
    }

    pub fn parse_return(&mut self) {
        // Drop curr
        // Replace curr with stack top
        if let Some((next_curr, next_bb)) = self.stack.pop() {
            self.curr = next_curr; // Move popped stack value into self.curr
            self.parse_bb(next_bb);
        } else {
            // There are no more paths! The stack is empty
            return;
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
    ) {
        println!("Call to function");
        // println!("Function: {:?}", func);
        // println!("Args: {:?}", args);
        // println!("Destination: {:?}", destination);
        // println!("Target: {:?}", target);
        // println!("Unwind: {:?}", unwind);
        // println!("Call Source: {:?}", call_source);
        // println!("Function Span: {:?}", fn_span); # the struct is private

        let local: Local;
        match func {
            Operand::Copy(place) => {
                // Place<'tcx>
                let local = place.local;
                // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
            }
            Operand::Move(place) => {
                let local = place.local;
                // println!("Local: {:?}", local); //Not sure about this just copied, switchInt format
            }
            Operand::Constant(place) => {
                // Box<ConstOperand<'tcx>>
                /*
                so this ConstOperand consists of
                pub struct ConstOperand<'tcx> {
                    pub span: Span, # this spam is again rustc_span::span_encoding::Span, where, span_encoding is private
                    pub user_ty: Option<UserTypeAnnotationIndex>,
                    pub const_: Const<'tcx>,
                }

                */
                let const_span = place.span;
                let const_user_ty = place.user_ty;
                let constant = place.const_;

                println!("Span: {:?}", const_span); // examples/simple.rs:2:5: 2:19 (#0) This is where the function is present in the source code
                println!("User Type: {:?}", const_user_ty); // None , idk what this is
                println!("Constant: {:?}", constant); // Val(ZeroSized, FnDef(DefId(1:2345 ~ std[c0a3]::fs::write), [&'{erased} str, &'{erased} str]))

                /*
                Now this const is
                pub enum Const<'tcx> {
                    Ty(Ty<'tcx>, Const<'tcx>),
                    Unevaluated(UnevaluatedConst<'tcx>, Ty<'tcx>),
                    Val(ConstValue<'tcx>, Ty<'tcx>),
                }
                */

                match constant {
                    Const::Ty(_ty, _const) => {
                        println!("here 236"); // Don't know
                    }
                    Const::Unevaluated(_unevaluated_const, _ty) => {
                        println!("here 240"); // Don't know
                    }
                    Const::Val(const_value, ty) => {
                        // println!("here 244");
                        println!("Const Value: {:?}", const_value); //ZeroSized, don't know what this is
                        println!("Type: {:?}", ty); //FnDef(DefId(1:2345 ~ std[c0a3]::fs::write), [&'{erased} str, &'{erased} str])

                        if let TyKind::FnDef(def_id, _) = ty.kind() {
                            if def_id.index.as_u32() == 2345 {
                                //2345 is def_id of std::fs::write , need a better way to do this
                                println!("Call to std::fs::write detected.");
                            }
                        }
                    }
                }
            } // Now we have to match the constant value

              //    println!("{:?}" , constant.literal);
        }
    }
}
// }
