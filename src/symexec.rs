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
        }
    }
    /// Checks if there is an assignment to symbolic variables in the executor such that write_arg_name matches /proc/self/mem.
    /// This function can be used to check that a write such a `fs::write(filename, contents)` does not write to the directory
    /// /proc/self/mem. The argument write_arg_name must already be present in the environment. If it is not, an Error is returned.
    pub fn is_write_safe(&mut self, write_arg_name: &str) -> Result<z3::SatResult, &str> {
        let variable = match self.string_variables.get(write_arg_name) {
            Some(x) => x,
            None => return Err("write_arg_name is not present in the environment."),
        };
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
        solver.assert(&variable.regex_matches(&unsafe_regex));
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
        value1: &z3::ast::String<'ctx>,
        value2: &z3::ast::String<'ctx>,
    ) -> z3::ast::String<'ctx> {
        z3::ast::String::concat(self.context, &[value1, value2])
    }
    /// Creates a z3 bool expression representing whether or not two strings are equivalent.
    pub fn string_equals(
        &self,
        value1: &z3::ast::String<'ctx>,
        value2: &z3::ast::String<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1._eq(value2)
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
    pub fn logical_not(&self, value: &z3::ast::Bool<'ctx>) -> z3::ast::Bool<'ctx> {
        value.not()
    }
    /// Creates a z3 bool expression from the conjunction of two z3 bool expressions.
    pub fn logical_and(
        &self,
        value1: &z3::ast::Bool<'ctx>,
        value2: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::and(self.context, &[value1, value2])
    }
    /// Creates a z3 bool expression from the disjunction of two z3 bool expressions.
    pub fn logical_or(
        &self,
        value1: &z3::ast::Bool<'ctx>,
        value2: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        z3::ast::Bool::or(self.context, &[value1, value2])
    }
    /// Create a z3 bool expression from the equality of two z3 bool expressions.
    pub fn bool_equals(
        &self,
        value1: &z3::ast::Bool<'ctx>,
        value2: &z3::ast::Bool<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1.iff(value2)
    }
    /// Creates an uninterpreted integer with the given variable name and adds it to the executor. This function can be
    /// used to model the int arguments to a function.
    pub fn create_int(&mut self, variable_name: &str) {
        self.int_variables.insert(
            variable_name.to_string(),
            z3::ast::Int::new_const(self.context, variable_name),
        );
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
    }
    /// Creates a z3 int expression from an Rust int.
    pub fn static_int(&self, value: i64) -> z3::ast::Int<'ctx> {
        z3::ast::Int::from_i64(self.context, value)
    }
    /// Creates a z3 int expression from the addition of two z3 int expressions.
    pub fn add(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Int<'ctx> {
        z3::ast::Int::add(self.context, &[value1, value2])
    }
    /// Creates a z3 int expression from the subtraction of two z3 int expressions.
    pub fn subtract(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Int<'ctx> {
        z3::ast::Int::sub(self.context, &[value1, value2])
    }
    /// Creates a z3 int expression from the multiplication of two z3 int expressions.
    pub fn multiply(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Int<'ctx> {
        z3::ast::Int::mul(self.context, &[value1, value2])
    }
    /// Creates a z3 int expression from the division of two z3 int expressions.
    pub fn divide(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Int<'ctx> {
        value1.div(value2)
    }
    /// Creates a z3 int expression from the remainder division (modulo) of two z3 int expressions.
    pub fn modulo(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Int<'ctx> {
        value1.rem(value2)
    }
    /// Creates a z3 bool expression from the less than comparison of two z3 int expressions.
    pub fn less_than(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1.lt(value2)
    }
    /// Creates a z3 bool expression from the less than or equal comparison of two z3 int expressions.
    pub fn less_than_or_equal(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1.le(value2)
    }
    /// Creates a z3 bool expression from the greater than comparison of two z3 int expressions.
    pub fn greater_than(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1.gt(value2)
    }
    /// Creates a z3 bool expression from the greater than or equal comparison of two z3 int expressions.
    pub fn greater_than_or_equal(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1.ge(value2)
    }
    /// Creates a z3 bool expression from the equality comparison of two z3 int expressions.
    pub fn int_equals(
        &self,
        value1: &z3::ast::Int<'ctx>,
        value2: &z3::ast::Int<'ctx>,
    ) -> z3::ast::Bool<'ctx> {
        value1._eq(value2)
    }
}
