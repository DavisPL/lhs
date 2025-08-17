#![feature(rustc_private)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![feature(mapped_lock_guards)]
#![allow(dead_code)]

use clap::Parser;
use clap::ValueEnum;

use std::fs;
use std::fs::{File, FileType};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const DEF_ID_PATH_BUF: usize = 5175;

use lhs::callback::LCallback;
use lhs::parser::MIRParser;
use lhs::symexec;

// -------------------- START RUSTC PORTION --------------------
extern crate rustc_driver;
extern crate rustc_error_codes;
extern crate rustc_errors;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

extern crate rustc_data_structures;
extern crate rustc_metadata;
extern crate rustc_middle;

use std::{path, process, str, sync::Arc};

use rustc_errors::registry;
use rustc_hash::FxHashMap;
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_session::config;
use rustc_span::FileNameDisplayPreference;

use rustc_data_structures::steal::Steal;
use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_hir::def::DefKind;
use rustc_middle::mir::Body;

use rustc_middle::query::LocalCrate;
use rustc_middle::util::Providers;
use rustc_session::search_paths::PathKind;
use rustc_driver::{run_compiler, Callbacks};
// -------------------- END RUSTC PORTION --------------------

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<String>>();
    get_callback_mir(args);
}

#[derive(Parser, Debug)]
#[command(name = "LHS: Leveraging (not) HIR via Symbolic execution")]
#[command(version = "1.0.1")]
#[command(about = "A command line utility for searching for analyzing a given Rust code's MIR and verifying proc/self/mem safety", long_about = None)]
pub struct Args {
    /// Filepath of Rust code
    #[arg(short, long, required = true)]
    pub source: String,

    /// Tool action options
    #[arg(short, long, required = true, value_enum)]
    pub action: Action,

    /// Toggle matching with numeric variables
    #[arg(long)]
    pub numeric: bool,
}

#[derive(Debug, ValueEnum, Clone, Eq, PartialEq)]
pub enum Action {
    Trace,
    Blocks,
    Local,
    Query,
    Callback,
}

fn get_callback_mir(args: Vec<String>) {
    // let out = process::Command::new("rustc")
    //     .arg("--print=sysroot")
    //     .current_dir(".")
    //     .output()
    //     .unwrap();
    // let sysroot = str::from_utf8(&out.stdout).unwrap().trim().to_string();
    // let rustc_args: Vec<String> = args;

    let mut callbacks = LCallback::new();
    // let args = std::env::args().collect::<Vec<String>>();
    // args is a Vec<String>?
    run_compiler(&args, &mut callbacks);
}
