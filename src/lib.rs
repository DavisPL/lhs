#![feature(rustc_private)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![feature(mapped_lock_guards)]
#![allow(dead_code)]

extern crate rustc_driver;
extern crate rustc_error_codes;
extern crate rustc_errors;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

extern crate rustc_abi;
extern crate rustc_data_structures;
extern crate rustc_middle;

pub mod operand;
pub mod parser;
pub mod symexec;
pub mod callback;