use std::collections::HashMap;
use z3;
use z3::ast::{Ast, Regexp};

#[derive(Debug, Clone)]
pub struct SymExec<'ctx> {
    pub context: &'ctx z3::Context,
    pub string_variables: HashMap<String, z3::ast::String<'ctx>>,
    pub int_variables: HashMap<String, z3::ast::Int<'ctx>>,
    pub bool_variables: HashMap<String, z3::ast::Bool<'ctx>>,
    pub constraints: Vec<z3::ast::Bool<'ctx>>,

    pub interval_map: HashMap<String, (Option<i128>, Option<i128>)>, // for narrowing down the range of values
}

impl<'ctx> SymExec<'ctx> {
    /// Creates an empty SymExec with a supplied Z3 context.
    pub fn new(context: &'ctx z3::Context) -> Self {
        SymExec {
            context,
            string_variables: HashMap::new(),
            int_variables: HashMap::new(),
            bool_variables: HashMap::new(),
            constraints: Vec::new(),
            interval_map: HashMap::new(),
        }
    }
    /// Checks if the constraints in the executor and the new constraint are satisfiable.
    pub fn check_constraint_sat(&self, new_constraint: &z3::ast::Bool<'ctx>) -> z3::SatResult {
        let solver = z3::Solver::new(self.context);
        for constraint in &self.constraints {
            solver.assert(constraint);
        }
        solver.assert(new_constraint);
        solver.check()
    }
    /// Checks if there is an assignment to symbolic variables in the executor such that write_arg_name matches /proc/self/mem.
    /// This function can be used to check that a write such a `fs::write(filename, contents)` does not write to the directory
    /// /proc/self/mem. The argument write_arg_name must already be present in the environment. If it is not, an Error is returned.
    pub fn is_write_safe(&self, expr: &z3::ast::String<'ctx>) -> Result<z3::SatResult, &str> {
        let solver = z3::Solver::new(self.context);
        
        for constraint in &self.constraints {
            solver.assert(constraint);
        }
        let slash = Regexp::literal(self.context, "/");
        let dot_slash = Regexp::literal(self.context, "./");
        let regex_parts = &[
            &slash,
            &Regexp::union(self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(self.context, "proc"),
            &slash,
            &Regexp::union(self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(self.context, "self"),
            &slash,
            &Regexp::union(self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(self.context, "mem"),
        ];
        let unsafe_regex = Regexp::concat(self.context, regex_parts);
        solver.assert(&expr.regex_matches(&unsafe_regex));
        let result = solver.check();
        if result == z3::SatResult::Sat {
            let model = solver.get_model().expect("Model should exist.");
            println!("Model: ");
            println!("{}", model);
        }
        Ok(result)
    }
    /// Adds a constraint to the executor. This constraint will be used for all satisfiability checks.
    pub fn add_constraint(&mut self, constraint: z3::ast::Bool<'ctx>) {
        self.constraints.push(constraint)
    }
    /// Creates an uninterpreted string with the given variable name and adds it to the executor. This function can be
    /// used to model the string arguments to a function.
    pub fn create_uninterpreted_string(&mut self, variable_name: &str) {
        self.string_variables.insert(
            variable_name.to_string(),
            z3::ast::String::new_const(self.context, variable_name),
        );
    }
    /// Gets the z3 string expression with the given variable name from the executor. If the variable name is not
    /// present, None is returned.
    pub fn get_string(&self, variable_name: &str) -> Option<&z3::ast::String<'ctx>> {
        self.string_variables.get(variable_name)
    }
    /// Assigns a string with the given value to the given variable name, adding it to the executor. Can also be used
    /// to replace the value of a string variable.
    pub fn assign_string(&mut self, variable_name: &str, value: z3::ast::String<'ctx>) {
        self.string_variables
            .insert(variable_name.to_string(), value);
    }
    /// Creates a z3 string expression from a Rust static string.
    pub fn static_string(&self, value: &str) -> z3::ast::String<'ctx> {
        z3::ast::String::from_str(self.context, value).expect("Value should not contain NUL bytes.")
    }
    /// Creates a z3 string expression from the concatenation of two strings.
    pub fn concat_strings(
        &self,
        lhs: &z3::ast::String<'ctx>,
        rhs: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        z3::ast::String::concat(self.context, &[lhs, rhs])
    }

    /// Creates a z3 string for path join operation (like Path::join or PathBuf::join)
    pub fn path_join(
        &self,
        base: &z3::ast::String<'ctx>,
        component: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> { 

        // TODO : CHECK if component is empty , if it is we do not need to add a separator
        // Create a "/" separator
        let separator = self.static_string("/");

        

        let a = z3::ast::Bool::ite(
            &base._eq(&self.static_string("")), // If base is empty
            component, // Return component as is
            &z3::ast::Bool::ite(
                &component._eq(&self.static_string("")), // If component is empty
                base, // Return base as is
                &z3::ast::Bool::ite(
                    &separator.prefix(component),// if component prefix matches "/"
                        &component.clone() ,
                    &z3::ast::Bool::ite(
                        // &base.suffix(&separator),
                        &separator.suffix(&base) ,
                        &self.concat_strings(base, component), // Concatenate base + component
                        {
                            let with_sep = self.concat_strings(base, &separator);
                            &self.concat_strings(&with_sep, component)
                        }
                    )
                ),
            )
        );

        dbg!(&a);

        a



        
            
        
        
        // Concatenate base + "/" + component

    }

    /// Creates a z3 bool expression representing whether or not two strings are equivalent.
    pub fn string_eq(
        &self,
        lhs: &z3::ast::String<'ctx>,
        rhs: &z3::ast::String<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs._eq(rhs)
    }
    /// Creates an uninterpreted boolean with the given variable name and adds it to the executor. This function can be
    /// used to model the bool arguments to a function.
    pub fn create_uninterpreted_bool(&mut self, variable_name: &str) {
        self.bool_variables.insert(
            variable_name.to_string(),
            z3::ast::Bool::new_const(self.context, variable_name),
        );
    }
    /// Gets the z3 bool expression with the given variable name from the executor. If the variable name is not
    /// present, None is returned.
    pub fn get_bool(&self, variable_name: &str) -> Option<&z3::ast::Bool<'ctx>> {
        self.bool_variables.get(variable_name)
    }
    /// Assigns a bool with the given value to the given variable name, adding it to the executor. Can also be used
    /// to replace the value of a bool variable.
    pub fn assign_bool(&mut self, variable_name: &str, value: z3::ast::Bool<'ctx>) {
        self.bool_variables.insert(variable_name.to_string(), value);
    }
    /// Creates a z3 bool expression from a Rust bool.
    pub fn static_bool(&self, value: bool) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(self.context, value)
    }
    /// Creates a z3 bool expression from the negation of a z3 bool expression.
    pub fn not(&self, value: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        value.not()
    }
    /// Creates a z3 bool expression from the conjunction of two z3 bool expressions.
    pub fn and(&self, lhs: &z3::ast::Bool<'ctx>, rhs: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::and(self.context, &[lhs, rhs])
    }
    /// Creates a z3 bool expression from the disjunction of two z3 bool expressions.
    pub fn or(&self, lhs: &z3::ast::Bool<'ctx>, rhs: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::or(self.context, &[lhs, rhs])
    }
    /// Create a z3 bool expression from the equality of two z3 bool expressions.
    pub fn bool_eq(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs._eq(rhs)
    }
    /// Create a z3 bool expression from the less than comparison of two z3 bool expressions.
    pub fn bool_lt(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        self.and(&self.not(lhs), rhs)
    }
    /// Create a z3 bool expression from the less than or equal comparison of two z3 bool expressions.
    pub fn bool_le(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(&self.context, true)
    }
    /// Create a z3 bool expression from the greater than comparison of two z3 bool expressions.
    pub fn bool_gt(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        self.and(rhs, &self.not(lhs))
    }
    /// Create a z3 bool expression from the greater than or equal comparison of two z3 bool expressions.
    pub fn bool_ge(
        &self,
        lhs: &z3::ast::Bool<'ctx>,
        rhs: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::from_bool(&self.context, true)
    }
    /// Creates an uninterpreted integer with the given variable name and adds it to the executor. This function can be
    /// used to model the int arguments to a function.
    pub fn create_int(&mut self, variable_name: &str) {
        self.int_variables.insert(
            variable_name.to_string(),
            z3::ast::Int::new_const(self.context, variable_name),
        );
        self.interval_map
            .insert(variable_name.to_string(), (None, None));
    }
    /// Gets the z3 int expression with the given variable name from the executor. If the variable name is not
    /// present, None is returned.
    pub fn get_int(&self, variable_name: &str) -> Option<&z3::ast::Int<'ctx>> {
        self.int_variables.get(variable_name)
    }
    /// Assigns a int with the given value to the given variable name, adding it to the executor. Can also be used
    /// to replace the value of a int variable.
    pub fn assign_int(&mut self, variable_name: &str, value: z3::ast::Int<'ctx>) {
        self.int_variables.insert(variable_name.to_string(), value);
        self.interval_map
            .insert(variable_name.to_string(), (None, None));
    }
    /// Creates a z3 int expression from an Rust int.
    pub fn static_int(&self, value: i128) -> z3::ast::Int<'ctx> {
        if value > i64::MIN.into() && value < i64::MAX.into() {
            z3::ast::Int::from_i64(self.context, value.try_into().unwrap())
        } else {
            let negative = if value < 0 { true } else { false };
            let unsigned_value: u128 = value.abs() as u128;
            let least_significant_bits: u64 = (unsigned_value & 0xFFFFFFFFFFFFFFFF) as u64;
            let most_significant_bits: u64 = (unsigned_value >> 64) as u64;
            let z3_least_significant_bits: z3::ast::Int<'_> =
                z3::ast::Int::from_u64(self.context, least_significant_bits);
            let z3_most_significant_bits: z3::ast::Int<'_> =
                z3::ast::Int::from_u64(self.context, most_significant_bits);
            let z3_two: z3::ast::Int<'_> = z3::ast::Int::from_u64(self.context, 2);
            let z3_neg_1: z3::ast::Int<'_> = z3::ast::Int::from_i64(self.context, -1);
            let z3_two_power_64: z3::ast::Int<'_> = self.mul(
                &z3::ast::Int::from_u64(self.context, 2u64.pow(32)),
                &z3::ast::Int::from_u64(self.context, 2u64.pow(32)),
            );
            if negative {
                self.mul(
                    &z3_neg_1,
                    &self.add(
                        &self.mul(&z3_most_significant_bits, &z3_two_power_64),
                        &z3_least_significant_bits,
                    ),
                )
            } else {
                self.add(
                    &self.mul(&z3_most_significant_bits, &z3_two_power_64),
                    &z3_least_significant_bits,
                )
            }
        }
    }
    /// Creates a z3 int expression from the addition of two z3 int expressions.
    pub fn add(&self, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::add(self.context, &[lhs, rhs])
    }
    /// Creates a z3 int expression from the subtraction of two z3 int expressions.
    pub fn sub(&self, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::sub(self.context, &[lhs, rhs])
    }
    /// Creates a z3 int expression from the multiplication of two z3 int expressions.
    pub fn mul(&self, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        z3::ast::Int::mul(self.context, &[lhs, rhs])
    }
    /// Creates a z3 int expression from the division of two z3 int expressions.
    pub fn div(&self, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        lhs.div(rhs)
    }
    /// Creates a z3 int expression from the remainder division (modulo) of two z3 int expressions.
    pub fn rem(&self, lhs: &z3::ast::Int<'ctx>, rhs: &z3::ast::Int<'ctx>) -> z3::ast::Int<'ctx> {
        lhs.rem(rhs)
    }
    /// Creates a z3 bool expression from the less than comparison of two z3 int expressions.
    pub fn int_lt(
        &self,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs.lt(rhs)
    }
    /// Creates a z3 bool expression from the less than or equal comparison of two z3 int expressions.
    pub fn int_le(
        &self,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs.le(rhs)
    }
    /// Creates a z3 bool expression from the greater than comparison of two z3 int expressions.
    pub fn int_gt(
        &self,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs.gt(rhs)
    }
    /// Creates a z3 bool expression from the greater than or equal comparison of two z3 int expressions.
    pub fn int_ge(
        &self,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs.ge(rhs)
    }
    /// Creates a z3 bool expression from the equality comparison of two z3 int expressions.
    pub fn int_eq(
        &self,
        lhs: &z3::ast::Int<'ctx>,
        rhs: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        lhs._eq(rhs)
    }

    pub fn int_interval(&self, var: &str) -> (Option<i128>, Option<i128>) {
        self.interval_map.get(var).cloned().unwrap_or((None, None))
    }

    pub fn set_interval(&mut self, var: &str, lo: Option<i128>, hi: Option<i128>) {
        self.interval_map.insert(var.to_string(), (lo, hi));
    }

    pub fn widen(
        (l0, h0): (Option<i128>, Option<i128>),
        (l1, h1): (Option<i128>, Option<i128>),
    ) -> (Option<i128>, Option<i128>) {
        (
            match (l0, l1) {
                (Some(a), Some(b)) if a == b => Some(a),
                _ => None,
            },
            match (h0, h1) {
                (Some(a), Some(b)) if a == b => Some(a),
                _ => None,
            },
        )
    }
}

#[test]
pub fn test_static_int() {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let exec_struct = SymExec::new(&ctx);
    let int1 = exec_struct.static_int(10);
    let int2 = exec_struct.static_int(-10);
    let int3 = exec_struct.static_int((i64::MAX - 5).into());
    let int4 = exec_struct.static_int((i64::MIN + 5).into());
    let int5 = exec_struct.add(&int1, &int3);
    assert_eq!(int1.to_string(), "10");
    assert_eq!(int2.to_string(), "(- 10)");
    assert_eq!(int3.to_string(), "9223372036854775802");
    assert!(
        exec_struct.check_constraint_sat(&exec_struct.int_lt(&int4, &int5)) == z3::SatResult::Sat
    );
}
