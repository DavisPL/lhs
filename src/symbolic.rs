use std::collections::HashMap;
use std::env::var;
use z3::{self, ast::Ast};
use z3::ast::Regexp;

pub struct Environment<'ctx> {
    context: z3::Context,
    string_variables: HashMap<String, z3::ast::String<'ctx>>,
    constraints: Vec<z3::ast::Bool<'ctx>>,
}

impl<'ctx> Environment<'ctx> {
    /// Creates an empty Environment.
    pub fn new() -> Self {
        let config = z3::Config::new();
        let context = z3::Context::new(&config);
        Environment {
            context,
            string_variables: HashMap::new(),
            constraints: Vec::new(),
        }
    }
    /// Creates an uninterpreted string with the given variable name and adds it to the Environment. This function can be
    /// used to model the string arguments of a function.
    pub fn create_string(&'ctx mut self, variable_name: &str) {
        self.string_variables.insert(
            variable_name.to_string(),
            z3::ast::String::new_const(&self.context, variable_name),
        );
    }
    /// Creates a static string with the given variable name and value and adds it to the environment. This function can be
    /// used to model a declaration such as `let name = "Bob";`.
    pub fn create_static_string(&'ctx mut self, variable_name: &str, value: &str) {
        self.string_variables.insert(
            variable_name.to_string(),
            z3::ast::String::new_const(&self.context, variable_name),
        );
        let variable = self
            .string_variables
            .get(variable_name)
            .expect("Variable should be present in hashmap");
        let const_z3_value = z3::ast::String::from_str(&self.context, value)
            .expect("Value should not contain NUL bytes");
        self.constraints.push(variable._eq(&const_z3_value))
    }
    /// Creates a string assigned to another string with the given variable name and adds it to the Environment. This function
    /// can be used to model an assignment such as `let person = name;`, where name is another string. The argument other_name
    /// must already be present in the Environment. If it is not, an Error is returned.
    pub fn create_string_copy(
        &'ctx mut self,
        variable_name: &str,
        other_name: &str,
    ) -> Result<(), &str> {
        self.string_variables.insert(
            variable_name.to_string(),
            z3::ast::String::new_const(&self.context, variable_name),
        );
        let variable = self
        .string_variables
        .get(variable_name)
        .expect("Variable should be present in hashmap");
        let other_variable = match self.string_variables.get(other_name) {
            Some(x) => x,
            None => return Err("other_name is not present in the environment.")
        };
        self.constraints.push(variable._eq(other_variable));
        Ok(())
    }
    /// Checks if there is an assignment to symbolic variables in the Environment such that write_arg_name matches /proc/self/mem.
    /// This function can be used to check that a write such as `fs::write(filename, contents)` does not write to the directory
    /// /proc/self/mem. The argument write_arg_name must already be present in the environment. If it is not, an Error is returned.
    /// Additionally, an error is returned if z3 cannot determine if there is or is not an assignment to symbolic variables such that 
    /// write_arg_name matches /proc/self/mem.
    pub fn write_safety(&'ctx mut self, write_arg_name: &str) -> Result<bool, &str> {
        let variable = match self.string_variables.get(write_arg_name) {
            Some(x) => x,
            None => return Err("write_arg_name is not present in the environment.")
        };
        let solver = z3::Solver::new(&self.context);
        for constraint in &self.constraints {
            solver.assert(constraint);
        }
        let slash = Regexp::literal(&self.context, "/");
        let dot_slash = Regexp::literal(&self.context, "./");

        let regex_parts = &[
            &slash,
            &Regexp::union(&self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(&self.context, "self"),
            &slash,
            &Regexp::union(&self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(&self.context, "proc"),
            &slash,
            &Regexp::union(&self.context, &[&slash, &dot_slash]).star(),
            &Regexp::literal(&self.context, "mem"),
        ];
        let unsafe_regex = Regexp::concat(&self.context, regex_parts);
        solver.assert(&variable.regex_matches(&unsafe_regex));
        match solver.check() {
            z3::SatResult::Sat => Ok(true),
            z3::SatResult::Unsat => Ok(false),
            z3::SatResult::Unknown => Err("z3 returned Unknown."),
        }
    }
}

impl<'ctx> Default for Environment<'ctx> {
    fn default() -> Self {
        Self::new()
    }
}
