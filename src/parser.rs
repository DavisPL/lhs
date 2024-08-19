extern crate rustc_data_structures;
extern crate rustc_middle;

use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_middle::mir::Body;
use rustc_middle::mir::{BasicBlock, Local};
use rustc_middle::mir::{Operand, SwitchTargets};
use rustc_middle::mir::{StatementKind, TerminatorKind};
use std::collections::HashMap;
use std::sync::Arc;

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
}

impl<'a, 'ctx> MIRParser<'a, 'ctx> {
    pub fn new(mir_body: MappedReadGuard<'a, Body<'a>>, z3: symexec::SymExec<'ctx>) -> Self {
        MIRParser {
            mir_body,
            curr: z3,
            stack: Vec::new(),
        }
    }

    pub fn parse(&mut self) {
        // We start traversing the function MIR body from bb0
        self.parse_bb(BasicBlock::from_usize(0));
    }

    pub fn parse_bb(&mut self, bb: BasicBlock) {
        match self.mir_body.basic_blocks.get(bb) {
            Some(bb_data) => {
                // Statements
                for statement in &bb_data.statements {
                    match statement.kind {
                        StatementKind::Assign(_) => println!("assignment!"),
                        _ => println!("unknown statement..."),
                    }
                }
                // Terminator
                match &bb_data.terminator().kind {
                    TerminatorKind::Goto { target } => self.parse_bb(*target),
                    TerminatorKind::SwitchInt { discr, targets } => {
                        self.parse_switch_int(discr.clone(), targets.clone())
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
        let mut curr_pc: Vec<String> = Vec::new();
        for (value, target) in targets.iter() {
            // Make a clone of curr
            let mut cloned_curr = self.curr.clone();
            // Update the clone's PC
            // cloned_curr
            //     .constraints
            //     .push(format!("{} = {}", local.as_usize(), value)); // this is temp wrong and definitely not general
            //                                                         // Append to Negation PC vector for the otherwise branch
            // curr_pc.push(format!("{} != {}", local.as_usize(), value));
            // Push updated clone to parser's stack
            self.stack.push((cloned_curr, target));
        }
        // -----> We take the otherwise branch (right to left DFS... for now?)
        // Update current PC
        // self.curr.constraints.append(&mut curr_pc);
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
}
