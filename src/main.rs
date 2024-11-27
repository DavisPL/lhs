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

use rustc_data_structures::sync::Lrc;
use rustc_middle::query::LocalCrate;
use rustc_middle::util::Providers;
use rustc_session::search_paths::PathKind;
// -------------------- END RUSTC PORTION --------------------

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<String>>();
    get_callback_mir(args);
    // let config = Args::parse();
    // // Attempt to make PathBuf and error if invalid filepath
    // let path: PathBuf = path::PathBuf::from(&config.source);
    // if config.action == Action::Callback {
    //     get_callback_mir(config);
    // } else {
    //     get_mir_body(path, config);
    // }
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
    rustc_driver::RunCompiler::new(&args, &mut callbacks)
        .run()
        .unwrap();
}

// For now assuming there should only be one function in the Rust file
pub fn get_mir_body(path: PathBuf, args: Args) {
    let out = process::Command::new("rustc")
        .args(["--print=sysroot", "--emit=metadata", "-Zalways-encode-mir"])
        .current_dir(".")
        .output()
        .unwrap();
    println!("{:?}", out);
    let sysroot = str::from_utf8(&out.stdout).unwrap().trim();
    let config = rustc_interface::Config {
        // Command line options
        opts: config::Options {
            maybe_sysroot: Some(path::PathBuf::from(sysroot)),
            ..config::Options::default()
        },
        // cfg! configuration in addition to the default ones
        crate_cfg: Vec::new(),            // FxHashSet<(String, Option<String>)>
        crate_check_cfg: Vec::new(),      // CheckCfg
        input: config::Input::File(path), // enum with either File(PathBuf) or Str(FileName, String) -- have CLI for 1st!
        output_dir: None,                 // Option<PathBuf>
        output_file: None,                // Option<PathBuf>
        file_loader: None,                // Option<Box<dyn FileLoader + Send + Sync>>
        locale_resources: rustc_driver::DEFAULT_LOCALE_RESOURCES,
        lint_caps: FxHashMap::default(), // FxHashMap<lint::LintId, lint::Level>
        // This is a callback from the driver that is called when [`ParseSess`] is created.
        psess_created: None, //Option<Box<dyn FnOnce(&mut ParseSess) + Send>>
        // This is a callback from the driver that is called when we're registering lints;
        // it is called during plugin registration when we have the LintStore in a non-shared state.
        //
        // Note that if you find a Some here you probably want to call that function in the new
        // function being registered.
        register_lints: None, // Option<Box<dyn Fn(&Session, &mut LintStore) + Send + Sync>>
        // This is a callback from the driver that is called just after we have populated
        // the list of queries.
        //
        // The second parameter is local providers and the third parameter is external providers.
        override_queries: None, // Option<fn(&Session, &mut ty::query::Providers<'_>, &mut ty::query::Providers<'_>)>
        // Registry of diagnostics codes.
        registry: registry::Registry::new(rustc_errors::codes::DIAGNOSTICS),
        make_codegen_backend: None,
        expanded_args: Vec::new(),
        ice_file: None,
        hash_untracked_state: None,
        using_internal_features: Arc::default(),
    };

    // // Use `config` to load external crates
    // // Set the Config.override_queries
    // config.override_queries = Some(|_, providers| {
    //     providers.extern_queries.used_crate_source = |tcx, cnum| {
    //         let mut providers = Providers::default();
    //         rustc_metadata::provide(&mut providers);
    //         let mut crate_source = (providers.extern_queries.used_crate_source)(tcx, cnum);
    //         // HACK: rustc will emit "crate ... required to be available in rlib format, but
    //         // was not found in this form" errors once we use `tcx.dependency_formats()` if
    //         // there's no rlib provided, so setting a dummy path here to workaround those errors.
    //         Lrc::make_mut(&mut crate_source).rlib = Some((PathBuf::new(), PathKind::All)); // TODO: maybe mutate the pathbuf in this, to accept
    //         crate_source
    //     };
    // });

    rustc_interface::run_compiler(config, |compiler| {
        // Run the compiler
        compiler.enter(|queries| {
            // F: for<'tcx> FnOnce(&'tcx Queries<'tcx>) -> T
            // Parse the program and print the syntax tree.
            let parse = queries.parse().unwrap().get_mut().clone();
            // Analyze the program and inspect the types of definitions.
            queries.global_ctxt().unwrap().enter(|tcx| {
                // if tcx.sess.opts.optimize != OptLevel::No {
                //     tcx.dcx().warn("Miri does not support optimizations: the opt-level is ignored. The only effect \
                //         of selecting a Cargo profile that enables optimizations (such as --release) is to apply \
                //         its remaining settings, such as whether debug assertions and overflow checks are enabled.");
                // }
                let hir_map = tcx.hir();
                // Get all LocalDefID's (DefID's local to current krate)
                for local_def_id in tcx.hir().krate().owners.indices() {
                    let def_id = local_def_id.to_def_id();
                    if tcx.def_kind(local_def_id) == DefKind::Fn {
                        println!("MIR for function: {:?}", local_def_id);
                        // Construct mir_built body for function
                        let mir_body = tcx.optimized_mir(def_id); // tcx.mir_built(local_def_id).borrow();
                                                                  // Print position of current function being analyzed using Span
                                                                  // let span_data = mir_body.span.data();
                                                                  // let needs_stealing = &Steal::new(optimized_mir);
                                                                  // let mir_body = needs_stealing.clone().borrow();
                                                                  // println!("{:#?}", mir_body);
                                                                  // println!("{:#?}", mir_body.to_owned());
                        let source_map = tcx.sess.source_map(); // Can we move this outside and have the same functionality?
                        let mir_string: String = source_map
                            .span_to_string(mir_body.span, FileNameDisplayPreference::Local); // Might not display nice with Local?
                        println!("{mir_string}");
                        // We got the mir_body! Let's pass it into our analyzer/parser
                        if args.action == Action::Trace {
                            dbg!("{}", &mir_body);
                        }
                        match args.action {
                            Action::Trace => trace_mir_body(mir_body),
                            Action::Blocks => print_basic_blocks(mir_body),
                            Action::Local => print_local_decls(mir_body),
                            Action::Query => trace_mir_body(mir_body),
                            _ => (),
                        }
                    }
                }
            })
        });
    });
}

pub fn trace_mir_body<'a>(mir_body: &'a Body<'a>) {
    // dbg!("{}", &mir_body);
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let mut ev = symexec::SymExec::new(&ctx);

    for (local, local_decl) in mir_body.local_decls.iter_enumerated() {
        match local_decl.ty.kind() {
            TyKind::Int(_) => ev.create_int(local.as_usize().to_string().as_str()),
            TyKind::Str => ev.create_uninterpreted_string(local.as_usize().to_string().as_str()),
            TyKind::Char => ev.create_uninterpreted_string(local.as_usize().to_string().as_str()),
            TyKind::Ref(_, ty, _) => {
                if ty.is_str() {
                    ev.create_uninterpreted_string(local.as_usize().to_string().as_str())
                }
            }
            TyKind::Bool => ev.create_uninterpreted_bool(local.as_usize().to_string().as_str()),
            TyKind::Adt(fields, _) => {
                let def_ids = fields
                    .all_fields()
                    .map(|field| field.did)
                    .collect::<Vec<_>>();

                // Now you have a Vec containing all DefIds
                for def_id in &def_ids {
                    if def_id.index.as_usize() == DEF_ID_PATH_BUF {
                        ev.create_uninterpreted_string(local.as_usize().to_string().as_str());
                        break;
                    }
                }
            }
            _ => println!("Unsupported Type: {}", local_decl.ty),
        }
    }

    // println!("{:#?}", ev);

    let mut mir_parser = MIRParser::new(mir_body, ev);
    let fs_write_span: Option<rustc_span::Span> = mir_parser.parse();
    match fs_write_span {
        Some(sp) => {
            println!("WARNING: potential write to `/proc/self/mem`");
            println!("\t{:#?}", sp);
            // TODO: query solver model for the actual assignments,
            // and show the arguments of function that can be malicious
            // (based on arg_count)
        }
        None => println!("No potential writes to `/proc/self/mem` detected!"),
    }

    // println!("{:#?}", mir_parser.curr);
}

pub fn print_basic_blocks<'a>(mir_body: &'a Body<'a>) {
    for (bb, data) in mir_body.basic_blocks.iter_enumerated() {
        println!("{:?}: {:#?}", bb, data);
    }
}

pub fn print_local_decls<'a>(mir_body: &'a Body<'a>) {
    for (local, local_decl) in mir_body.local_decls.iter_enumerated() {
        println!("_{} = {}", local.as_usize(), local_decl.ty);
    }
}
