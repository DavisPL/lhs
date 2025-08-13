//!
//! Settings and constants to configure the analysis
//!
//!
pub const MAX_LOOP_ITER: u32 = 5; // Maximum loop iterations before widening

pub const ENV_VARS_TO_TRACK: &[&str] = &["RUSTC", "CARGO"]; // env's we want to track/protect against

pub const SOURCE_FUNCTIONS: &[&str] = &["std::env::args", "std::env::args_os"];

/// Function name, argument number we want to check, and what value or pattern of values it should not be
pub const SINK_FUNCTION_ARGS: &[(&str, usize, &str)] = &[
    ("std::fs::write", 0, "/proc/self/mem"),
    ("std::env::set_var", 0, "RUSTC"),
    ("std::process::Command::new", 0, "rm -rf *"),
];
