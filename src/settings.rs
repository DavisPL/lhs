//!
//! Settings and constants to configure the analysis
//!

/*
    TODO: incorporate below and delete
*/

/// TODO: should be configurable as a list of things to check for
pub const ENV_TO_TRACK: &str = "RUSTC"; // Environment variable to track

pub const MAX_LOOP_ITER: u32 = 5; // Maximum loop iterations before widening

/// Should be configurable as a list
pub const FUNCTION_NAME: &str = "std::process::Command::new";
pub const FUNCTION_ARG: &str = "rm -rf *";

/*
    List of source and sink functions for the taint analysis.
*/

/// Function name
pub const SOURCE_FUNCTIONS: &[&str] = &[
    "std::env::args",
    "std::env::args_os",
    "std::env::var",
    "std::env::var_os",
];

/// Function name, argument number we want to check, and what value or pattern of values it should not be
pub const SINK_FUNCTION_ARGS: &[(&str, usize, &str)] = &[
    ("std::fs::write", 0, "/proc/self/mem"),
    ("std::fs::write", 0, "/proc/*"),
    ("std::env::set_var", 0, "RUSTC"),
    ("std::process::Command::new", 0, "rm -rf *"),
];
