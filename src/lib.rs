#![feature(rustc_private)]
#![allow(unused_variables)]
#![allow(unused_imports)]

use std::fs;
use std::path::Path;
use std::path::PathBuf;
// use std::env;
use clap::Parser;

use std::io::ErrorKind;
use std::fs::File;
use std::fs::FileType;

#[derive(Parser, Debug)]
#[command(name = "mir_analyzer")]
#[command(version = "1.0.1")]
#[command(about = "A command line utility for searching for analyzing a given Rust code's MIR and verifying proc/self/mem safety", long_about = None)]
pub struct Args {
    /// Filepath of Rust code
    #[arg(short, long, required = true)]
    pub source: String,

    // Parsing variable options?

    /// Toggle matching with numeric variables
    #[arg(long)] 
    pub numeric: bool,
}

pub fn run(args: Args) {
    // Attempt to make PathBuf and error if invalid filepath
    let path = path::PathBuf::from(&args.source);
    analyze_mir(path);
}

// -------------------- RUSTC PORTION --------------------
extern crate rustc_driver;
extern crate rustc_error_codes;
extern crate rustc_errors;
extern crate rustc_hash;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_span;

extern crate rustc_middle;


use std::{path, process, str, sync::Arc};

use rustc_errors::registry;
use rustc_hash::FxHashMap;
use rustc_session::config;

use rustc_hir::def::DefKind;
// use rustc_hir::hir::ItemKind;
// use rustc_middle::query::cached::mir_built;

pub fn analyze_mir(path: PathBuf) {
    let out = process::Command::new("rustc")
        .arg("--print=sysroot")
        .current_dir(".")
        .output()
        .unwrap();
    let sysroot = str::from_utf8(&out.stdout).unwrap().trim();
    let config = rustc_interface::Config {
        // Command line options
        opts: config::Options {
            maybe_sysroot: Some(path::PathBuf::from(sysroot)),
            ..config::Options::default()
        },
        // cfg! configuration in addition to the default ones
        crate_cfg: Vec::new(),       // FxHashSet<(String, Option<String>)>
        crate_check_cfg: Vec::new(), // CheckCfg
        input: config::Input::File(path), // enum with either File(PathBuf) or Str(FileName, String) -- have CLI for 1st!
        output_dir: None,  // Option<PathBuf>
        output_file: None, // Option<PathBuf>
        file_loader: None, // Option<Box<dyn FileLoader + Send + Sync>>
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

    rustc_interface::run_compiler(config, |compiler| {
        compiler.enter(|queries| { // F: for<'tcx> FnOnce(&'tcx Queries<'tcx>) -> T
            // Parse the program and print the syntax tree.
            let parse = queries.parse().unwrap().get_mut().clone();
            // println!("{parse:?}");
            
            // Analyze the program and inspect the types of definitions.
            queries.global_ctxt().unwrap().enter(|tcx| {
                for id in tcx.hir().items() {
                    let hir = tcx.hir();
                    let item = hir.item(id);
                    match item.kind {
                        rustc_hir::ItemKind::Static(_, _, _) | rustc_hir::ItemKind::Fn(_, _, _) => { // print out static and function variables! -- HELLO and main!
                            let name = item.ident;
                            let ty = tcx.type_of(item.hir_id().owner.def_id);
                            // println!("{name:?}:\t{ty:?}")
                        }
                        _ => (),
                    }
                }

                // Attempt of usage at `mir_built`
                let hir_map = tcx.hir();
                // Get all LocalDefID's (DefID's local to current krate)
                for local_def_id in tcx.hir().krate().owners.indices() {
                    let def_id = local_def_id.to_def_id();
                    // println!("{:?} -> {:?}", local_def_id, def_id);

                    if tcx.def_kind(local_def_id) == DefKind::Fn {
                        // println!("{:?} was a local fn trait!", local_def_id);

                        // Get MIR of function (a Steal), and borrow it to read as much as we want
                        let mir_body = tcx.mir_built(local_def_id).borrow();
                        // dbg!("{}", &mir_body);
                        println!("MIR for function: {:?}", local_def_id);
                        // Print basic blocks within function MIR
                        for (bb, data) in mir_body.basic_blocks.iter_enumerated() {
                            println!("{:?}: {:#?}", bb, data);
                        }
                        // Optionally, print the control flow graph (CFG)
                        // let cfg = mir_body.cfg();
                        // for (bb, successors) in cfg.iter() {
                        //     println!("BasicBlock {:?} -> {:?}", bb, successors);
                        // }
                    }
                }

                // // Trial
                // let hir_map = tcx.hir();
                // for local_def_id in tcx.hir().krate().owners.indices() {
                //     // let def_id = hir_map.local_def_id(*id);
                //     // let mir = tcx.optimized_mir(def_id.to_def_id());
                //     match hir_map.get(&local_def_id).unwrap().kind {
                //         ItemKind::Fn(..) => {
                //             let mir = tcx.optimized_mir(local_def_id);
                //             println!("{:#?}", mir);
                //         },
                //         _ => {}
                //     }
                    
                // }

            })
        });
    });
}