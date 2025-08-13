extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

extern crate rustc_data_structures;
extern crate rustc_metadata;

use rustc_data_structures::steal::Steal;
use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::DefKind;
use rustc_interface::{interface::Compiler, Queries};
use rustc_middle::mir::Body;
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::FileNameDisplayPreference;

use rustc_data_structures::sync::Lrc;
use rustc_middle::query::LocalCrate;
use rustc_middle::util::Providers;
use rustc_session::search_paths::PathKind;
use rustc_span::Span;

use crate::parser::MIRParser;
use rustc_span::source_map::SourceMap;
// use crate::symexec;
use crate::symexec::SymExecBool as SymExec;

use std::collections::HashMap;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};

const DEF_ID_PATH_BUF: usize = 5175;

pub struct LCallback {}

impl LCallback {
    pub fn new() -> Self {
        LCallback {}
    }
}

impl Callbacks for LCallback {
    // fn config(&mut self, config: &mut Config) {
    //     config.override_queries = Some(|_, providers| {
    //         providers.extern_queries.used_crate_source = |tcx, cnum| {
    //             let mut providers = Providers::default();
    //             rustc_metadata::provide(&mut providers);
    //             let mut crate_source = (providers.extern_queries.used_crate_source)(tcx, cnum);
    //             // HACK: rustc will emit "crate ... required to be available in rlib format, but
    //             // was not found in this form" errors once we use `tcx.dependency_formats()` if
    //             // there's no rlib provided, so setting a dummy path here to workaround those errors.
    //             Lrc::make_mut(&mut crate_source).rlib = Some((PathBuf::new(), PathKind::All));
    //             crate_source
    //         };
    //     });
    // }

    fn after_analysis<'tcx>(
        &mut self,
        compiler: &Compiler,
        queries: &'tcx Queries<'tcx>,
    ) -> Compilation {
        queries.global_ctxt().unwrap().enter(|tcx| {
            let hir_map = tcx.hir();
            // let source_map = tcx.sess.source_map();
            for local_def_id in tcx.hir().krate().owners.indices() {
                let def_id = local_def_id.to_def_id();
                if tcx.def_kind(local_def_id) == DefKind::Fn {
                    // println!("MIR for function: {:?}", local_def_id);
                    // let mir_body = tcx.optimized_mir(def_id);
                    let mir_body = tcx.optimized_mir(local_def_id); // This is a Steal<RwLock<Option>>
                                                                    // let mir_string = source_map.span_to_string(mir_body.span, FileNameDisplayPreference::Local);
                                                                    // println!("{mir_string}");
                    trace_mir_body(tcx, mir_body)
                }
            }
        });
        Compilation::Continue
    }
}

pub fn trace_mir_body<'tcx>(tcx: TyCtxt<'tcx>, mir_body: &'tcx Body<'tcx>) {
    // dbg!("{}", &mir_body);
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    let mut ev = SymExec::new(&ctx);

    for (local, local_decl) in mir_body.local_decls.iter_enumerated() {
        match local_decl.ty.kind() {
            TyKind::Int(_) => ev.create_int(local.as_usize().to_string().as_str()),
            TyKind::Uint(_) => ev.create_int(local.as_usize().to_string().as_str()),
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

                // Now we have a Vec containing all DefIds
                for def_id in &def_ids {
                    if def_id.index.as_usize() == DEF_ID_PATH_BUF {
                        ev.create_uninterpreted_string(local.as_usize().to_string().as_str());
                        break;
                    }
                }
            }
            // _ => println!("Unsupported Type: {}", local_decl.ty),
            _ => {
                // println!("Unsupported Type: {}", local_decl.ty);
            }
        }
    }

    // let mut mir_parser = MIRParser::new(mir_body, ev);
    let mut mir_parser = MIRParser::new(tcx, mir_body, ev);
    // let fs_write_span: Option<rustc_span::Span> = mir_parser.parse();
    let dangerous_spans: HashMap<(String, String), Vec<Span>> = mir_parser.parse();
    println!("=== Dangerous Spans ===");
    if dangerous_spans.is_empty() {
        println!("No dangerous spans found.");
    } else {
        let sm = tcx.sess.source_map();
        for ((func, arg), spans) in &dangerous_spans {
            println!("=== {} (arg = {:?}) ===", func, arg);
            for (i, sp) in spans.iter().enumerate() {
                let loc = sm.span_to_string(*sp, FileNameDisplayPreference::Local); // or span_to_filename_string if you prefer
                println!("  [{:02}] {}", i + 1, loc);
            }
        }
        dump_danger_csv(
            tcx.sess.source_map(),
            &dangerous_spans,
            "dangerous_spans.csv",
        );
    }
}

pub fn dump_danger_csv(sm: &SourceMap, map: &HashMap<(String, String), Vec<Span>>, path: &str) {
    let path = Path::new(path);

    // Check if the file already exists
    let file_exists = path.exists();

    // Open the file in append mode
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(path) {
        let mut writer = BufWriter::new(file);

        // Only write header if file didn't exist before
        if !file_exists {
            let _ = writeln!(writer, "function,value,index,span");
        }

        for ((func, arg), spans) in map {
            for (i, sp) in spans.iter().enumerate() {
                let span_str = sm.span_to_string(*sp, FileNameDisplayPreference::Local);
                let _ = writeln!(writer, "{},{},{},{}", func, arg, i + 1, span_str);
            }
        }
    }
}
