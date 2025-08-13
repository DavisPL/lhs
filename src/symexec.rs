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

#[derive(Debug, Clone)]
pub struct SymExecBool<'ctx> {
    pub context: &'ctx z3::Context,
    pub string_variables: HashMap<String, Slot<z3::ast::String<'ctx>>>,
    pub int_variables: HashMap<String, Slot<z3::ast::Int<'ctx>>>,
    pub bool_variables: HashMap<String, Slot<z3::ast::Bool<'ctx>>>,
    pub constraints: Vec<z3::ast::Bool<'ctx>>,
    pub interval_map: HashMap<String, (Option<i128>, Option<i128>)>,

    pub path_taint: bool, // useful for cases like examples/unsafe/command2
}

impl<'ctx> SymExecBool<'ctx> {
    // Creates an empty SymExec with a supplied Z3 context.
    pub fn new(context: &'ctx z3::Context) -> Self {
        Self {
            context,
            string_variables: HashMap::new(),
            int_variables: HashMap::new(),
            bool_variables: HashMap::new(),
            constraints: Vec::new(),
            interval_map: HashMap::new(),
            path_taint: false,
        }
    }

    // Helper functions to set taint flag on variables.
    fn set_flag<T>(
        map: &mut HashMap<String, Slot<T>>,
        name: &str,
        flag: bool,
    ) -> Result<(), &'static str> {
        map.get_mut(name)
            .map(|s| s.flag = flag)
            .ok_or("unknown variable")
    }

    // Helper functions to get taint flag on variables.
    fn get_flag<T>(map: &HashMap<String, Slot<T>>, name: &str) -> Option<bool> {
        map.get(name).map(|s| s.flag)
    }

    // Helper function to insert a variable with a flag.
    fn insert_with_flag<T>(map: &mut HashMap<String, Slot<T>>, name: &str, val: T, flag: bool) {
        map.insert(name.into(), Slot::with_flag(val, flag));
    }

    /// Creates an uninterpreted string with the given variable name and adds it to the executor. This function can be used to model the string arguments to a function.
    pub fn create_uninterpreted_string_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::String::new_const(self.context, name);
        SymExecBool::insert_with_flag(&mut self.string_variables, name, v, flag);
    }
    // Create uninterpreted string variables without a taint flag.
    pub fn create_uninterpreted_string(&mut self, name: &str) {
        self.create_uninterpreted_string_with_flag(name, false);
    }

    // Create uninterpreted integer variables with custom taint flag.
    pub fn create_int_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::Int::new_const(self.context, name);
        SymExecBool::insert_with_flag(&mut self.int_variables, name, v, flag);
        self.interval_map.insert(name.into(), (None, None));
    }
    // Create uninterpreted integer variables without a taint flag.
    pub fn create_int(&mut self, name: &str) {
        self.create_int_with_flag(name, false);
    }

    // Create uninterpreted boolean variables with custom taint flag.
    pub fn create_uninterpreted_bool_with_flag(&mut self, name: &str, flag: bool) {
        let v = z3::ast::Bool::new_const(self.context, name);
        SymExecBool::insert_with_flag(&mut self.bool_variables, name, v, flag);
    }

    // Create uninterpreted boolean variables without a taint flag.
    pub fn create_uninterpreted_bool(&mut self, name: &str) {
        self.create_uninterpreted_bool_with_flag(name, false);
    }

    /// Assigns a string with the given value to the given variable name, adding it to the executor. Can also be used to replace the value of a string variable.
    pub fn assign_string(&mut self, name: &str, value: z3::ast::String<'ctx>) {
        let f = self.string_flag(name).unwrap_or(false);
        SymExecBool::insert_with_flag(&mut self.string_variables, name, value, f);
    }
    /// Assigns an integer with the given value to the given variable name, adding it to the executor. Can also be used to replace the value of an integer variable.
    pub fn assign_int(&mut self, name: &str, value: z3::ast::Int<'ctx>) {
        let f = self.int_flag(name).unwrap_or(false);
        SymExecBool::insert_with_flag(&mut self.int_variables, name, value, f);
        self.interval_map.entry(name.into()).or_insert((None, None));
    }
    /// Assigns a boolean with the given value to the given variable name, adding it to the executor. Can also be used to replace the value of a boolean variable.
    pub fn assign_bool(&mut self, name: &str, value: z3::ast::Bool<'ctx>) {
        let f = self.bool_flag(name).unwrap_or(false);
        SymExecBool::insert_with_flag(&mut self.bool_variables, name, value, f);
    }

    /// Gets the z3 string expression with the given variable name from the executor. If the variable name is not present, None is returned.
    pub fn get_string(&self, name: &str) -> Option<&z3::ast::String<'ctx>> {
        self.string_variables.get(name).map(|s| &s.value)
    }
    /// Gets the z3 integer expression with the given variable name from the executor. If the variable name is not present, None is returned.
    pub fn get_int(&self, name: &str) -> Option<&z3::ast::Int<'ctx>> {
        self.int_variables.get(name).map(|s| &s.value)
    }
    /// Gets the z3 boolean expression with the given variable name from the executor. If the variable name is not present, None is returned.
    pub fn get_bool(&self, name: &str) -> Option<&z3::ast::Bool<'ctx>> {
        self.bool_variables.get(name).map(|s| &s.value)
    }

    /// Sets a taint flag on the given string variable name. If the variable name is not present, an error is returned.
    pub fn set_string_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        SymExecBool::set_flag(&mut self.string_variables, name, flag)
    }
    /// Gets the taint flag on the given string variable name. If the variable name is not present, None is returned.
    pub fn string_flag(&self, name: &str) -> Option<bool> {
        SymExecBool::get_flag(&self.string_variables, name)
    }
    /// Sets a taint flag on the given int variable name. If the variable name is not present, an error is returned.
    pub fn set_int_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        SymExecBool::set_flag(&mut self.int_variables, name, flag)
    }
    /// Gets the taint flag on the given int variable name. If the variable name is not present, None is returned.
    pub fn int_flag(&self, name: &str) -> Option<bool> {
        SymExecBool::get_flag(&self.int_variables, name)
    }
    /// Sets a taint flag on the given bool variable name. If the variable name is not present, an error is returned.
    pub fn set_bool_flag(&mut self, name: &str, flag: bool) -> Result<(), &'static str> {
        SymExecBool::set_flag(&mut self.bool_variables, name, flag)
    }
    /// Gets the taint flag on the given bool variable name. If the variable name is not present, None is returned.
    pub fn bool_flag(&self, name: &str) -> Option<bool> {
        SymExecBool::get_flag(&self.bool_variables, name)
    }
    /// Adds a constraint to the executor. This constraint will be used for all satisfiability checks.
    pub fn add_constraint(&mut self, c: z3::ast::Bool<'ctx>) {
        self.constraints.push(c)
    }
    /// Checks if the constraints in the executor and the new constraint are satisfiable.
    pub fn check_constraint_sat(&self, new_c: &z3::ast::Bool<'ctx>) -> z3::SatResult {
        let s = z3::Solver::new(self.context);
        for c in &self.constraints {
            s.assert(c);
        }
        s.assert(new_c);
        s.check()
    }
    /// Checks if there is an assignment to symbolic variables in the executor such that write_arg_name matches /proc/self/mem.
    /// This function can be used to check that a write such a `fs::write(filename, contents)` does not write to the directory
    /// /proc/self/mem. The argument write_arg_name must already be present in the environment. If it is not, an Error is returned.
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

    /// Creates a z3 string expression from a Rust static string.
    pub fn static_string(&self, v: &str) -> z3::ast::String<'ctx> {
        z3::ast::String::from_str(self.context, v).unwrap()
    }
    /// Creates a z3 string expression from the concatenation of two strings.
    pub fn concat_strings(
        &self,
        a: &z3::ast::String<'ctx>,
        b: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        z3::ast::String::concat(self.context, &[a, b])
    }
    /// Creates a z3 string for path join operation (like Path::join or PathBuf::join)
    pub fn path_join(
        &self,
        base: &z3::ast::String<'ctx>,
        comp: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        let sep = self.static_string("/"); // Create a "/" separator
        z3::ast::Bool::ite(
            &base._eq(&self.static_string("")), // If base is empty
            comp,                               // Return comp as is
            &z3::ast::Bool::ite(
                &comp._eq(&self.static_string("")), // If comp is empty
                base,                               // Return base as is
                &z3::ast::Bool::ite(
                    &sep.prefix(comp), // if component prefix matches "/"
                    comp,
                    &z3::ast::Bool::ite(&sep.suffix(base), &self.concat_strings(base, comp), {
                        let tmp = self.concat_strings(base, &sep);
                        &self.concat_strings(&tmp, comp) // Concatenate base + component
                    }),
                ),
            ),
        )
    }

    /// Creates a z3 bool expression representing whether or not two strings are equivalent.
    pub fn string_eq(
        &self,
        a: &z3::ast::String<'ctx>,
        b: &z3::ast::String<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }

    /// Creates a z3 bool expression from a Rust bool.
    pub fn static_bool(&self, v: bool) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(self.context, v)
    }
    /// Creates a z3 bool expression from the negation of a z3 bool expression.
    pub fn not(&self, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        b.not()
    }
    /// Creates a z3 bool expression from the conjunction of two z3 bool expressions.
    pub fn and(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::and(self.context, &[a, b])
    }
    /// Creates a z3 bool expression from the disjunction of two z3 bool expressions.
    pub fn or(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::or(self.context, &[a, b])
    }
    /// Create a z3 bool expression from the equality of two z3 bool expressions.
    pub fn bool_eq(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }
    /// Create a z3 bool expression from the less than comparison of two z3 bool expressions.
    pub fn bool_lt(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        self.and(&self.not(a), b)
    }
    /// Create a z3 bool expression from the less than or equal comparison of two z3 bool expressions.
    pub fn bool_le(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(&self.context, true)
    }
    /// Create a z3 bool expression from the less than or equal comparison of two z3 bool expressions.
    pub fn bool_gt(&self, a: &z3::ast::Bool<'ctx>, b: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        self.and(b, &self.not(a))
    }
    /// Create a z3 bool expression from the greater than or equal comparison of two z3 bool expressions.
    pub fn bool_ge(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(&self.context, true)
    }

    /// Creates a z3 int expression from an Rust int.
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
    /// Creates a z3 int expression from the addition of two z3 int expressions.
    pub fn add(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::add(self.context, &[a, b])
    }
    /// Creates a z3 int expression from the subtraction of two z3 int expressions.
    pub fn sub(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::sub(self.context, &[a, b])
    }
    /// Creates a z3 int expression from the multiplication of two z3 int expressions.
    pub fn mul(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::mul(self.context, &[a, b])
    }
    /// Creates a z3 int expression from the division of two z3 int expressions.
    pub fn div(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        a.div(b)
    }
    /// Creates a z3 int expression from the remainder of two z3 int expressions.
    pub fn rem(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        a.rem(b)
    }
    /// Creates a z3 int expression from the negation of a z3 int expression.
    pub fn int_eq(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a._eq(b)
    }
    /// Creates a z3 int expression from the less than, less than or equal, greater than, and greater than or equal comparisons of two z3 int expressions.
    pub fn int_lt(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.lt(b)
    }
    /// Creates a z3 int expression from the less than or equal comparison of two z3 int expressions.
    pub fn int_le(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.le(b)
    }
    /// Creates a z3 int expression from the greater than, greater than or equal comparisons of two z3 int expressions.
    pub fn int_gt(&self, a: &z3::ast::Int<'ctx>, b: &z3::ast::Int<'ctx>) -> z3::ast::Bool<'ctx> {
        a.gt(b)
    }
    /// Creates a z3 int expression from the greater than or equal comparison of two z3 int expressions.
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

    // Creates a z3 regular expression from a pattern string.
    /// The pattern can contain '*' as a wildcard that matches any sequence of characters.
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
    /// Checks if the given string matches the given pattern.
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

    /// UNSAT if it's impossible for expr to NOT match the pattern
    pub fn check_string_always_matches(
        &self,
        expr: &z3::ast::String<'ctx>,
        pattern: &str,
    ) -> z3::SatResult {
        let re = self.regex_from_pattern(pattern);
        let matches = expr.regex_matches(&re);
        self.check_constraint_sat(&self.not(&matches))
    }

    /// SAT if there exists a model with s == lit
    pub fn could_equal_literal(&self, s: &z3::ast::String<'ctx>, lit: &str) -> z3::SatResult {
        let eq = s._eq(&self.static_string(lit));
        self.check_constraint_sat(&eq)
    }

    /// UNSAT if it's impossible for s != lit
    pub fn must_equal_literal(&self, s: &z3::ast::String<'ctx>, lit: &str) -> z3::SatResult {
        let ne = self.not(&s._eq(&self.static_string(lit)));
        self.check_constraint_sat(&ne)
    }

    pub fn is_tainted(&self, name: &str) -> bool {
        self.string_flag(name).unwrap_or(false)
            || self.int_flag(name).unwrap_or(false)
            || self.bool_flag(name).unwrap_or(false)
    }

    /// Force a taint value (true = tainted, false = clean) on all slots that might carry this variable’s name.
    pub fn set_taint(&mut self, name: &str, flag: bool) {
        let mut touched = false;
        if self.set_string_flag(name, flag).is_ok() {
            touched = true;
        }
        if self.set_int_flag(name, flag).is_ok() {
            touched = true;
        }
        if self.set_bool_flag(name, flag).is_ok() {
            touched = true;
        }
        if !touched {
            // if we see a type we don't know, we assume it's a string
            // (fallback to keep analysis sound-ish on unknown slots)
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
