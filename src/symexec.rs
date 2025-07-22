use std::collections::HashMap;
use z3;
use z3::ast::{Ast, Regexp};

#[derive(Debug, Clone)]
pub struct Slot<T> {
    pub value: T,
    pub flag: bool,
}

impl<T> Slot<T> {
    pub fn new(value: T) -> Self {
        Self { value, flag: false }
    }
    pub fn with_flag(value: T, flag: bool) -> Self {
        Self { value, flag }
    }
}

// TODO: Hassnain
// Pls copy 48 lines of documentation back into this file from
// https://github.com/DavisPL/lhs/blob/11a7afdb78e12ed92db8f44a1bca19433110089b/src/symexec.rs

#[derive(Debug, Clone)]
pub struct SymExecBool<'ctx> {
    pub context: &'ctx z3::Context,
    pub string_variables: HashMap<String, Slot<z3::ast::String<'ctx>>>,
    pub int_variables: HashMap<String, Slot<z3::ast::Int<'ctx>>>,
    pub bool_variables: HashMap<String, Slot<z3::ast::Bool<'ctx>>>,
    pub constraints: Vec<z3::ast::Bool<'ctx>>,
    pub interval_map: HashMap<String, (Option<i128>, Option<i128>)>,
}

impl<'ctx> SymExecBool<'ctx> {
    pub fn new(context: &'ctx z3::Context) -> Self {
        Self {
            context,
            string_variables: HashMap::new(),
            int_variables: HashMap::new(),
            bool_variables: HashMap::new(),
            constraints: Vec::new(),
            interval_map: HashMap::new(),
        }
    }

    fn set_flag<T>(
        map: &mut HashMap<String, Slot<T>>,
        name: &str,
        flag: bool,
    ) -> Result<(), &'static str> {
        map.get_mut(name)
            .map(|s| s.flag = flag)
            .ok_or("unknown variable")
    }
    fn get_flag<T>(map: &HashMap<String, Slot<T>>, name: &str) -> Option<bool> {
        map.get(name).map(|s| s.flag)
    }

    pub fn create_uninterpreted_string_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::String::new_const(self.context, name);
        self.string_variables
            .insert(name.into(), Slot::with_flag(v, flag));
    }
    pub fn create_uninterpreted_string(&mut self, name: &str) {
        self.create_uninterpreted_string_with_flag(name, false);
    }

    pub fn create_int_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::Int::new_const(self.context, name);
        self.int_variables
            .insert(name.into(), Slot::with_flag(v, flag));
        self.interval_map.insert(name.into(), (None, None));
    }
    pub fn create_int(&mut self, name: &str) {
        self.create_int_with_flag(name, false);
    }

    pub fn create_uninterpreted_bool_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::Bool::new_const(self.context, name);
        self.bool_variables
            .insert(name.into(), Slot::with_flag(v, flag));
    }
    pub fn create_uninterpreted_bool(&mut self, name: &str) {
        self.create_uninterpreted_bool_with_flag(name, false);
    }

    pub fn assign_string(&mut self, name: &str, value: z3::ast::String<'ctx>) {
        self.string_variables
            .insert(name.into(), Slot::with_flag(value, false));
    }
    pub fn assign_int(&mut self, name: &str, value: z3::ast::Int<'ctx>) {
        self.int_variables
            .insert(name.into(), Slot::with_flag(value, false));
        self.interval_map.entry(name.into()).or_insert((None, None));
    }
    pub fn assign_bool(&mut self, name: &str, value: z3::ast::Bool<'ctx>) {
        self.bool_variables
            .insert(name.into(), Slot::with_flag(value, false));
    }

    pub fn get_string(&self, name: &str) -> Option<&z3::ast::String<'ctx>> {
        self.string_variables.get(name).map(|s| &s.value)
    }
    pub fn get_int(&self, name: &str) -> Option<&z3::ast::Int<'ctx>> {
        self.int_variables.get(name).map(|s| &s.value)
    }
    pub fn get_bool(&self, name: &str) -> Option<&z3::ast::Bool<'ctx>> {
        self.bool_variables.get(name).map(|s| &s.value)
    }

    pub fn set_string_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        Self::set_flag(&mut self.string_variables, name, flag)
    }
    pub fn string_flag(&self, name: &str) -> Option<bool> {
        Self::get_flag(&self.string_variables, name)
    }
    pub fn set_int_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        Self::set_flag(&mut self.int_variables, name, flag)
    }
    pub fn int_flag(&self, name: &str) -> Option<bool> {
        Self::get_flag(&self.int_variables, name)
    }
    pub fn set_bool_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        Self::set_flag(&mut self.bool_variables, name, flag)
    }
    pub fn bool_flag(&self, name: &str) -> Option<bool> {
        Self::get_flag(&self.bool_variables, name)
    }

    pub fn add_constraint(&mut self, c: z3::ast::Bool<'ctx>) {
        self.constraints.push(c)
    }
    pub fn check_constraint_sat(&self, new_c: &z3::ast::Bool<'ctx>) -> z3::SatResult {
        let s = z3::Solver::new(self.context);
        for c in &self.constraints {
            s.assert(c);
        }
        s.assert(new_c);
        s.check()
    }

    pub fn is_write_safe(&self, expr: &z3::ast::String<'ctx>) -> Result<z3::SatResult, &str> {
        let s = z3::Solver::new(self.context);
        for c in &self.constraints {
            s.assert(c);
        }
        let slash = Regexp::literal(self.context, "/");
        let dot = Regexp::literal(self.context, "./");
        let pat = &[
            &slash,
            &Regexp::union(self.context, &[&slash, &dot]).star(),
            &Regexp::literal(self.context, "proc"),
            &slash,
            &Regexp::union(self.context, &[&slash, &dot]).star(),
            &Regexp::literal(self.context, "self"),
            &slash,
            &Regexp::union(self.context, &[&slash, &dot]).star(),
            &Regexp::literal(self.context, "mem"),
        ];
        s.assert(&expr.regex_matches(&Regexp::concat(self.context, pat)));
        Ok(s.check())
    }

    pub fn static_string(&self, v: &str) -> z3::ast::String<'ctx> {
        z3::ast::String::from_str(self.context, v).unwrap()
    }
    pub fn concat_strings(
        &self,
        a: &z3::ast::String<'ctx>,
        b: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        z3::ast::String::concat(self.context, &[a, b])
    }
    pub fn path_join(
        &self,
        base: &z3::ast::String<'ctx>,
        comp: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        let sep = self.static_string("/");
        z3::ast::Bool::ite(
            &base._eq(&self.static_string("")),
            comp,
            &z3::ast::Bool::ite(
                &comp._eq(&self.static_string("")),
                base,
                &z3::ast::Bool::ite(
                    &sep.prefix(comp),
                    comp,
                    &z3::ast::Bool::ite(&sep.suffix(base), &self.concat_strings(base, comp), {
                        let tmp = self.concat_strings(base, &sep);
                        &self.concat_strings(&tmp, comp)
                    }),
                ),
            ),
        )
    }

    pub fn string_eq(
        &self,
        a: &z3::ast::String<'ctx>,
        b: &z3::ast::String<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }

    pub fn static_bool(&self, v: bool) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(self.context, v)
    }
    pub fn not(&self, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        b.not()
    }
    pub fn and(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::and(self.context, &[a, b])
    }
    pub fn or(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::or(self.context, &[a, b])
    }
    pub fn bool_eq(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }
    pub fn bool_lt(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        self.and(&self.not(a), b)
    }
    pub fn bool_gt(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        self.and(b, &self.not(a))
    }

    pub fn static_int(&self, v: i128) -> z3::ast::Int<'ctx> {
        if (i64::MIN as i128..=i64::MAX as i128).contains(&v) {
            z3::ast::Int::from_i64(self.context, v as i64)
        } else {
            let neg = v < 0;
            let u: u128 = v.abs() as u128;
            let lo = (u & 0xFFFFFFFFFFFFFFFF) as u64;
            let hi = (u >> 64) as u64;
            let lo_i = z3::ast::Int::from_u64(self.context, lo);
            let hi_i = z3::ast::Int::from_u64(self.context, hi);
            let two64 = self.mul(
                &z3::ast::Int::from_u64(self.context, 1 << 32),
                &z3::ast::Int::from_u64(self.context, 1 << 32),
            );
            let sum = self.add(&self.mul(&hi_i, &two64), &lo_i);
            if neg {
                self.mul(&z3::ast::Int::from_i64(self.context, -1), &sum)
            } else {
                sum
            }
        }
    }
    pub fn add(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::add(self.context, &[a, b])
    }
    pub fn sub(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::sub(self.context, &[a, b])
    }
    pub fn mul(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::mul(self.context, &[a, b])
    }
    pub fn div(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        a.div(b)
    }
    pub fn rem(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        a.rem(b)
    }
    pub fn int_eq(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }
    pub fn int_lt(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.lt(b)
    }
    pub fn int_le(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.le(b)
    }
    pub fn int_gt(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.gt(b)
    }
    pub fn int_ge(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.ge(b)
    }

    pub fn int_interval(&self, v: &str) -> (Option<i128>, Option<i128>) {
        self.interval_map.get(v).cloned().unwrap_or((None, None))
    }
    pub fn set_interval(&mut self, v: &str, lo: Option<i128>, hi: Option<i128>) {
        self.interval_map.insert(v.into(), (lo, hi));
    }
    pub fn widen(
        (l0, h0): (Option<i128>, Option<i128>),
        (l1, h1): (Option<i128>, Option<i128>),
    ) -> (Option<i128>, Option<i128>) {
        (
            if l0 == l1 { l0 } else { None },
            if h0 == h1 { h0 } else { None },
        )
    }

    fn regex_from_pattern(&self, pat: &str) -> z3::ast::Regexp<'ctx> {
        if !pat.contains('*') {
            return Regexp::literal(self.context, pat);
        }
        let mut parts = Vec::new();
        for (i, seg) in pat.split('*').enumerate() {
            if i > 0 {
                parts.push(Regexp::full(self.context));
            }
            if !seg.is_empty() {
                parts.push(Regexp::literal(self.context, seg));
            }
        }
        let refs: Vec<&Regexp<'ctx>> = parts.iter().collect();
        match refs.len() {
            0 => Regexp::full(self.context),
            1 => refs[0].clone(),
            _ => Regexp::concat(self.context, &refs),
        }
    }
    pub fn check_string_matches(
        &self,
        expr: &z3::ast::String<'ctx>,
        pattern: &str,
    ) -> z3::SatResult {
        let s = z3::Solver::new(self.context);
        for c in &self.constraints {
            s.assert(c);
        }
        s.assert(&expr.regex_matches(&self.regex_from_pattern(pattern)));
        s.check()
    }

    pub fn is_tainted(&self, name: &str) -> bool {
        self.string_flag(name).unwrap_or(false)
            || self.int_flag(name).unwrap_or(false)
            || self.bool_flag(name).unwrap_or(false)
    }

    /// Force a taint value (true = tainted, false = clean) on *all* slots
    /// that might carry this variable’s name.
    pub fn set_taint(&mut self, name: &str, flag: bool) {
        if !(self.set_string_flag(name, flag).is_ok()
            || self.set_int_flag(name, flag).is_ok()
            || self.set_bool_flag(name, flag).is_ok())
        {
            // if we see a type we don't know, we assume it's a string
            // (this is a hack, but it works for now)
            let s = z3::ast::String::new_const(self.context, name);
            self.string_variables
                .insert(name.into(), Slot::with_flag(s, flag));
        }
    }

    /// Copy taint from `src` → `dest` (used by the MIR interpreter).
    pub fn propagate_taint(&mut self, src: &str, dest: &str) {
        self.set_taint(dest, self.is_tainted(src));
    }
    pub fn dump_taint(&self) {
        println!("─── TAINT STATUS ───");
        for (k, v) in &self.string_variables {
            println!("str  {}  tainted={}", k, v.flag);
        }
        for (k, v) in &self.int_variables {
            println!("int  {}  tainted={}", k, v.flag);
        }
        for (k, v) in &self.bool_variables {
            println!("bool {}  tainted={}", k, v.flag);
        }
        println!("────────────────────");
    }
}

#[test]
fn test_static_int() {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let exec = SymExecBool::new(&ctx);
    let a = exec.static_int(10);
    let b = exec.static_int(-10);
    let c = exec.static_int((i64::MAX - 5).into());
    let d = exec.static_int((i64::MIN + 5).into());
    let e = exec.add(&a, &c);
    assert_eq!(a.to_string(), "10");
    assert_eq!(b.to_string(), "(- 10)");
    assert_eq!(c.to_string(), "9223372036854775802");
    assert!(exec.check_constraint_sat(&exec.int_lt(&d, &e)) == z3::SatResult::Sat);
}
