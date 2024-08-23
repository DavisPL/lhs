extern crate rustc_data_structures;
extern crate rustc_middle;

use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::BinOp;
use rustc_middle::mir::Body;
use rustc_middle::mir::Rvalue;
use rustc_middle::mir::Rvalue::BinaryOp;
use rustc_middle::mir::Rvalue::Use;
use rustc_middle::mir::{
    BasicBlock, CallSource, Const, ConstValue, Local, Place, SourceInfo, UnwindAction,
};
use rustc_middle::mir::{Operand, SwitchTargets};
use rustc_middle::mir::{StatementKind, TerminatorKind};
use rustc_middle::ty::TyKind;
use std::collections::HashMap;
use std::sync::Arc;

use rustc_middle::mir::interpret::ConstAllocation;
// use rustc_span::span_encoding::Span;

#[path = "../z3/src/symexec.rs"]
pub mod symexec;

const DEF_ID_FS_WRITE : u32 = 2345;

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
                    Operand::Constant(constant) => {
                        let operand = *constant;
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
                        }
                        // Constant strings not supported right now
                        // else if let Some(var) = self_curr.get_string(left_local) {
                        //     self_curr.assign_bool(
                        //         local,
                        //         self_curr.string_equals(var, constant.eval().unwrap()),
                        //     );
                        // }
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
            BinOp::Eq => todo!(),
            _ => println!("unknown binary operation"),
        }
    }

    fn r#use<'tcx>(self_curr: &mut symexec::SymExec<'ctx>, local: &str, operand: Operand<'tcx>) {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => {
                let place = place.local.as_usize().to_string();
                if let Some(var) = self_curr.get_int(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.int_equals(var, self_curr.get_int(place.as_str()).unwrap()),
                    );
                } else if let Some(var) = self_curr.get_bool(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.bool_equals(var, self_curr.get_bool(place.as_str()).unwrap()),
                    );
                } else if let Some(var) = self_curr.get_string(local) {
                    self_curr.assign_bool(
                        local,
                        self_curr.string_equals(var, self_curr.get_string(place.as_str()).unwrap()),
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

    pub fn parse_bb(&mut self, bb: BasicBlock) {
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
        let func_def_id = self.parse_operand(&func); //passing it func, gives def_id
        // println!("Func DefId: {:?}", func_def_id);
         if func_def_id == Some(DEF_ID_FS_WRITE) {
            //   println!("Found function DefId in call: {:?}", def_id);
            println!("Found fs::write call");
         }
         
        self.parse_args(&args); // I can get the args, but do i need to do something about this, or just call sovler? How will solver know that i have to check for these variables, do I create a z3 model? 

        // println!("Destination: {:?}", destination); //_4 what does this mean?

        // println!("Target: {:?}", target); //this is a basic block, so do i just call parse_bb for it?
        // parse_bb(target.unwrap());
        // call parse_bb for target

        // println!("Unwind: {:?}", unwind); //this is also a basic block, so do i just call parse_bb for it?, but this is for unwind, do we need this? this is _4 in this case - we ignore unwinding

       // println!("Call Source: {:?}", call_source); // https://doc.rust-lang.org/beta/nightly-rustc/rustc_middle/mir/syntax/enum.CallSource.html do we care about this?
    }

    fn parse_operand<'tcx>(&self, operand: &Operand<'tcx>) -> Option<u32> {
        match operand {
            Operand::Copy(_place) => {
                // Handle the case for Operand::Copy if necessary, otherwise return None
                println!("here 225"); // Placeholder for Copy case
                None
            }
            Operand::Move(place) => {
                // this is for function arguments.
                println!("here 230"); // Placeholder for Move case
                // pub struct Place<'tcx> {
                //     pub local: Local,
                //     pub projection: &'tcx List<PlaceElem<'tcx>>,
                // }
                let local = place.local;
                let projection = place.projection;
    
                println!("Local: {:?}", local);  // ths is the variable number like _1, _2 etc.
                println!("Projection: {:?}", projection); // this is [] - it means value is accessed directly.

                // println!("{:?}" , projection.)

                /*
                enum PlaceElem<'tcx> {
                    Deref,
                    Field(FieldIdx, Ty<'tcx>),
                    Index(Local),
                    ConstantIndex {
                        offset: u64,
                        min_length: u64,
                        from_end: bool,
                    },
                    Subslice {
                        from: u64,
                        to: u64,
                        from_end: bool,
                    },
                    Downcast(Option<Symbol>, VariantIdx),
                    OpaqueCast(Ty<'tcx>),
                    Subtype(Ty<'tcx>),
                }
                */  

                // match elem {
                //     PlaceElem::Deref => {
                //         println!("Dereferencing");
                //     }
                //     PlaceElem::Field(field_idx, ty) => {
                //         println!("Accessing field {:?} of type {:?}", field_idx, ty);
                //     }
                //     PlaceElem::Index(local) => {
                //         println!("Indexing with local {:?}", local);
                //     }
                //     PlaceElem::ConstantIndex { offset, min_length, from_end } => {
                //         println!("Indexing with constant offset: {}, min length: {}, from end: {}", offset, min_length, from_end);
                //     }
                //     PlaceElem::Subslice { from, to, from_end } => {
                //         println!("Subslice from: {}, to: {}, from end: {}", from, to, from_end);
                //     }
                //     PlaceElem::Downcast(symbol, variant_idx) => {
                //         println!("Downcasting to variant {:?} with symbol {:?}", variant_idx, symbol);
                //     }
                //     PlaceElem::OpaqueCast(ty) => {
                //         println!("Opaque casting to type {:?}", ty);
                //     }
                //     PlaceElem::Subtype(ty) => {
                //         println!("Subtyping to type {:?}", ty);
                //     }
                // }
                
                None
            }
            Operand::Constant(place) => {
                //this is for funciton calls 
                //also for arga when you hardcode them , e.g, std::fs::write("a.txt", "Hello, world!").unwrap();
                let const_span = place.span;
                println!("Const Span: {:?}", const_span);

                let const_user_ty = place.user_ty;
                println!("Const User Ty: {:?}", const_user_ty);
                let constant = place.const_;
                
                // pub enum Const<'tcx> {
                //     Ty(Ty<'tcx>, Const<'tcx>),
                //     Unevaluated(UnevaluatedConst<'tcx>, Ty<'tcx>),
                //     Val(ConstValue<'tcx>, Ty<'tcx>),
                // }
                match constant {
                    Const::Ty(_ty, _const) => {
                        println!("here 236"); // Placeholder for Ty case
                        None
                    }
                    Const::Unevaluated(_unevaluated_const, _ty) => {
                        println!("here 240"); // Placeholder for Unevaluated case
                        None
                    }
                    Const::Val(const_value, ty) => {
                        // println!("here 244"); 
                        println!("Const Value: {:?} {:?}", const_value , ty); 

                        match const_value {
                            ConstValue::Slice { data, meta } => {
                                if let Some(str_data) = self.extract_string_from_const(&data, meta) {
                                    println!("Extracted string: {:?}", str_data);
                                }
                            }
                            _ => {
                                println!("Unhandled ConstValue variant");
                            }
                        }
    

                        if let TyKind::FnDef(def_id, idk) = ty.kind() {
                            println!("Found function DefId in const: {:?}", def_id);
                            println!("IDK: {:?}", idk);
                            return Some(def_id.index.as_u32());
                        }
                        None
                    }
                }
            }
        }
    }
    

    fn extract_string_from_const<'tcx>(
        &self,
        data: &'tcx ConstAllocation<'tcx>, //tpub struct ConstAllocation<'tcx>(pub Interned<'tcx, Allocation>);
        meta: u64,
    ) -> Option<String> {
        println!("Data: {:?}", data);
        println!("Meta: {:?}", meta);

        //0: Interned<'tcx, Allocation>
        let allocation = &data.0.align; //this is alignment

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


        println!("Allocation: {:?}", allocation);

        // Typically, the `Interned<Allocation>` type has methods to access the allocation data
        // You might need to use an API method like `inspect` or something similar to get the bytes

        // Assuming we have a method to get the raw bytes of the allocation
        let string_length = meta as usize;

        None
    }
    
    fn parse_args<'tcx>(&self, args: &[rustc_span::source_map::Spanned<Operand<'tcx>>]) {
        for arg in args.iter() {
            println!("Arg: {:?}", arg);
            if let Some(def_id) = self.parse_operand(&arg.node) {
                println!("Found function DefId in args: {:?}", def_id);
            }
        }
    }
}
// }
