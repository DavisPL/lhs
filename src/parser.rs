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

const DEF_ID_FS_WRITE: usize = 2_345;
const MAX_LOOP_ITER: u32 = 5; // after how many visits we “widen”

pub struct MIRParser<'mir, 'ctx> {
    mir_body: &'mir Body<'mir>,
    pub curr: SymExec<'ctx>,

    stack: Vec<(SymExec<'ctx>, BasicBlock)>,
    path_count: u32,
    current_path: Vec<BasicBlock>,
    visit_counts: HashMap<BasicBlock, u32>,
}

impl<'mir, 'ctx> MIRParser<'mir, 'ctx> {
    pub fn new(body: &'mir Body<'mir>, z3: SymExec<'ctx>) -> Self {
        Self {
            mir_body: body,
            curr: z3,
            stack: Vec::new(),
            path_count: 0,
            current_path: Vec::new(),
            visit_counts: HashMap::new(),
        }
    }

    pub fn parse(&mut self) -> Option<rustc_span::Span> {
        println!("START: Path 0!");
        self.parse_bb(BasicBlock::from_usize(0))
    }

    // Turn `Place` (+ projection) into a stable key
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

    fn collect_written_vars(&self, bb: BasicBlock) -> HashSet<String> {
        let mut vars = HashSet::new();
        for stmt in &self.mir_body.basic_blocks[bb].statements {
            if let StatementKind::Assign(bx) = &stmt.kind {
                vars.insert(self.place_key(&bx.0));
            }
        }
        vars
    }

    fn constraint_mentions(names: &HashSet<String>, c: &z3::ast::Bool<'ctx>) -> bool {
        let txt = c.to_string();
        names.iter().any(|n| txt.contains(n))
    }


    fn parse_bb(&mut self, bb: BasicBlock) -> Option<rustc_span::Span> {

        let counter = self.visit_counts.entry(bb).or_insert(0);
        *counter += 1;

        if *counter > MAX_LOOP_ITER {
            println!(
                "\tbb{} exceeded limit {} — widening",
                bb.as_u32(),
                MAX_LOOP_ITER
            );

            // clear constraints on only those variables that are in this BB
            let written = self.collect_written_vars(bb);

            self.curr
                .constraints
                .retain(|c| !Self::constraint_mentions(&written, c));

            for w in written {
                self.curr.set_interval(&w, None, None);
            }

            return self.parse_return();
        }

        // if self.current_path.contains(&bb) && *counter > 1 {
        //     let written = self.collect_written_vars(bb);
        //     self.curr
        //         .constraints
        //         .retain(|c| !Self::constraint_mentions(&written, c));
        // }

        /* ── walk statements and terminator ── */
        self.current_path.push(bb);
        let data = &self.mir_body.basic_blocks[bb];

        for stmt in &data.statements {
            if let StatementKind::Assign(val) = &stmt.kind {
                self.parse_assign(val.clone());
            }
        }

        let res = match &data.terminator().kind {
            TerminatorKind::Goto { target } => self.parse_bb(*target),

            TerminatorKind::Return
            | TerminatorKind::Unreachable
            | TerminatorKind::CoroutineDrop
            | TerminatorKind::UnwindResume
            | TerminatorKind::UnwindTerminate { .. } => self.parse_return(),

            TerminatorKind::SwitchInt { discr, targets } => {
                self.parse_switch_int(discr.clone(), targets.clone())
            }

            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                call_source,
                ..
            } => self.parse_call(
                func.clone(),
                args.clone(),
                destination.clone(),
                *target,
                (*unwind).clone(),
                call_source.clone(),
            ),

            TerminatorKind::Assert {
                cond,
                expected,
                target,
                unwind,
                ..
            } => {
                
                let idx = get_operand_local(cond).unwrap_or(0);
                let pred = self
                    .curr
                    .get_bool(idx.to_string().as_str())
                    .cloned()
                    .unwrap_or_else(|| self.curr.static_bool(true));

                let mut ok = self.curr.clone();
                ok.add_constraint(if *expected {
                    pred.clone()
                } else {
                    ok.not(&pred)
                });
                self.stack.push((ok, *target));

                if let UnwindAction::Cleanup(clean) = unwind {
                    let mut bad = self.curr.clone();
                    bad.add_constraint(if *expected { bad.not(&pred) } else { pred });
                    self.stack.push((bad, *clean));
                }
                self.parse_return()
            }

            TerminatorKind::Yield { resume, drop, .. } => {
                self.stack.push((self.curr.clone(), *resume));
                if let Some(d) = drop {
                    self.stack.push((self.curr.clone(), *d));
                }
                self.parse_return()
            }
            TerminatorKind::TailCall { .. } => self.parse_return(),

            TerminatorKind::InlineAsm {
                targets, unwind, ..
            } => {
                for &t in targets {
                    self.stack.push((self.curr.clone(), t));
                }
                if let UnwindAction::Cleanup(clean) = unwind {
                    self.stack.push((self.curr.clone(), *clean));
                }
                self.parse_return()
            }

            TerminatorKind::Drop { target, unwind, .. } => {
                self.stack.push((self.curr.clone(), *target));
                if let UnwindAction::Cleanup(clean) = unwind {
                    self.stack.push((self.curr.clone(), *clean));
                }
                self.parse_return()
            }

            TerminatorKind::FalseEdge { real_target, .. } => self.parse_bb(*real_target),
            TerminatorKind::FalseUnwind { real_target, .. } => self.parse_bb(*real_target),
        };

        self.current_path.pop();
        res
    }

    
    fn parse_assign<'tcx>(&mut self, boxed: Box<(Place<'tcx>, Rvalue<'tcx>)>) {
        let (place, rhs) = *boxed;
        let dst_key = self.place_key(&place);

        match rhs {
            Rvalue::Use(op) => self.parse_use(dst_key.as_str(), &op),
            Rvalue::BinaryOp(op, operands) => self.parse_bin_op(dst_key.as_str(), op, &operands),
            _ => {}
        }
    }

    fn parse_use<'tcx>(&mut self, dst: &str, op: &Operand<'tcx>) {
        match op {
            Operand::Copy(p) | Operand::Move(p) => {
                let src_key = self.place_key(p);
                if let Some(v) = self.curr.get_int(src_key.as_str()).cloned() {
                    self.curr.assign_int(dst, v);
                } else if let Some(b) = self.curr.get_bool(src_key.as_str()).cloned() {
                    self.curr.assign_bool(dst, b);
                } else if let Some(s) = self.curr.get_string(src_key.as_str()).cloned() {
                    self.curr.assign_string(dst, s);
                }
            }
            Operand::Constant(c) => {
                let k = &c.const_;
                if let Some(si) = k.try_to_scalar_int() {
                    self.curr.assign_int(
                        dst,
                        self.curr.static_int((si.to_int(si.size()) as i64).into()),
                    );
                } else if let Some(b) = k.try_to_bool() {
                    self.curr.assign_bool(dst, self.curr.static_bool(b));
                } else if let Some(s) = get_operand_const_string(op) {
                    self.curr
                        .assign_string(dst, self.curr.static_string(s.as_str()));
                }
            }
        }
    }

    fn parse_bin_op<'tcx>(
        &mut self,
        dst: &str,
        op: BinOp,
        (lhs, rhs): &(Operand<'tcx>, Operand<'tcx>),
    ) {
        let int_of = |o: &Operand<'tcx>, me: &Self| -> Option<z3::ast::Int<'ctx>> {
            match o {
                Operand::Copy(p) | Operand::Move(p) => {
                    me.curr.get_int(me.place_key(p).as_str()).cloned()
                }
                Operand::Constant(c) => c
                    .const_
                    .try_to_scalar_int()
                    .map(|si| me.curr.static_int((si.to_int(si.size()) as i64).into())),
            }
        };
        let lhs_i = int_of(lhs, self);
        let rhs_i = int_of(rhs, self);

        if let (Some(l), Some(r)) = (&lhs_i, &rhs_i) {
            use BinOp::*;
            match op {
                Eq => self.curr.assign_bool(dst, self.curr.int_eq(l, r)),
                Ne => self
                    .curr
                    .assign_bool(dst, self.curr.not(&self.curr.int_eq(l, r))),
                Lt => self.curr.assign_bool(dst, self.curr.int_lt(l, r)),
                Le => self.curr.assign_bool(dst, self.curr.int_le(l, r)),
                Gt => self.curr.assign_bool(dst, self.curr.int_gt(l, r)),
                Ge => self.curr.assign_bool(dst, self.curr.int_ge(l, r)),
                Add => self.curr.assign_int(dst, self.curr.add(l, r)),
                Sub => self.curr.assign_int(dst, self.curr.sub(l, r)),
                Mul => self.curr.assign_int(dst, self.curr.mul(l, r)),
                Div => self.curr.assign_int(dst, self.curr.div(l, r)),
                Rem => self.curr.assign_int(dst, self.curr.rem(l, r)),
                _ => {}
            }
            return;
        }

        // string eq / ne
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            let str_of = |o: &Operand<'tcx>, me: &Self| -> Option<z3::ast::String<'ctx>> {
                match o {
                    Operand::Copy(p) | Operand::Move(p) => {
                        me.curr.get_string(me.place_key(p).as_str()).cloned()
                    }
                    Operand::Constant(_) => {
                        get_operand_const_string(o).map(|s| me.curr.static_string(s.as_str()))
                    }
                }
            };
            if let (Some(a), Some(b)) = (str_of(lhs, self), str_of(rhs, self)) {
                let eq = self.curr.string_eq(&a, &b);
                self.curr.assign_bool(
                    dst,
                    if matches!(op, BinOp::Eq) {
                        eq
                    } else {
                        self.curr.not(&eq)
                    },
                );
            }
        }
    }

    /* ─────── SwitchInt ─────── */

    fn parse_switch_int(
        &mut self,
        discr: Operand,
        targets: SwitchTargets,
    ) -> Option<rustc_span::Span> {
        let local = match discr {
            Operand::Copy(p) | Operand::Move(p) => p.local,
            Operand::Constant(_) => return self.parse_return(),
        };
        let sym = self
            .curr
            .get_bool(local.as_usize().to_string().as_str())
            .cloned();

        if let Some(pred) = sym {
            let (val0, bb0) = targets.iter().next().unwrap();
            let bb_else = targets.otherwise();

            let mut t = self.curr.clone();
            let mut f = self.curr.clone();
            t.add_constraint(if val0 == 0 {
                t.not(&pred)
            } else {
                pred.clone()
            });
            f.add_constraint(if val0 == 0 { pred } else { f.not(&pred) });

            self.stack.push((t, bb0));
            self.stack.push((f, bb_else));
            self.parse_return()
        } else {
            for (_, bb) in targets.iter() {
                self.stack.push((self.curr.clone(), bb));
            }
            self.stack.push((self.curr.clone(), targets.otherwise()));
            self.parse_return()
        }
    }

    fn parse_call<'tcx>(
        &mut self,
        func: Operand<'tcx>,
        args: Box<[rustc_span::source_map::Spanned<Operand<'tcx>>]>,
        _dest: Place<'tcx>,
        target: Option<BasicBlock>,
        unwind: UnwindAction,
        _src: CallSource,
    ) -> Option<rustc_span::Span> {
        
        let is_write_def = get_operand_def_id(&func) == Some(DEF_ID_FS_WRITE);

        let mut write_is_dangerous = false;

        if !args.is_empty() {
            let path_operand = &args[0].node;
            write_is_dangerous = match get_operand_local(path_operand) {

                Some(0) => {
                    let s = get_operand_const_string(path_operand).unwrap();
                    s == "/proc/self/mem"
                }
                
                Some(idx) => {
                    if let Some(sym_str) = self.curr.get_string(idx.to_string().as_str()) {
                        matches!(self.curr.is_write_safe(sym_str), Ok(SatResult::Sat))
                    } else {
                        false
                    }
                }
                None => false,
            };
        }

        if (is_write_def || write_is_dangerous) && write_is_dangerous {
            // report only if the path is "/proc/self/mem"
            return get_operand_span(&func);
        }

        if let Some(bb) = target {
            self.parse_bb(bb)
        } else if let UnwindAction::Cleanup(clean) = unwind {
            self.parse_bb(clean)
        } else {
            self.parse_return()
        }
    }

    fn parse_return(&mut self) -> Option<rustc_span::Span> {
        if let Some((state, bb)) = self.stack.pop() {
            self.curr = state;
            self.path_count += 1;
            println!("START: Path_{}!", self.path_count);
            self.parse_bb(bb)
        } else {
            None
        }
    }
}
