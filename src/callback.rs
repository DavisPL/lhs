extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_session;
extern crate rustc_middle;
extern crate rustc_span;

extern crate rustc_data_structures;
extern crate rustc_metadata;

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::{interface::Compiler, Queries};
use rustc_middle::ty::{TyCtxt, TyKind};
use rustc_span::FileNameDisplayPreference;
use rustc_hir::def::DefKind;
use rustc_middle::mir::Body;

use rustc_data_structures::steal::Steal;
use rustc_data_structures::sync::{MappedReadGuard, ReadGuard, RwLock};

use rustc_session::search_paths::PathKind;
use rustc_data_structures::sync::Lrc;
use rustc_middle::util::Providers;
use rustc_middle::query::LocalCrate;

use crate::parser::MIRParser;
use crate::symexec;

const DEF_ID_PATH_BUF: usize = 5175;

pub struct LCallback {
    src: String,
}

impl LCallback {
    pub fn new(src: String) -> Self {
        LCallback { src }
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
                    println!("MIR for function: {:?}", local_def_id);
                    // let mir_body = tcx.optimized_mir(def_id);
                    let mir_body = tcx.optimized_mir(local_def_id); // This is a Steal<RwLock<Option>>
                    // let mir_string = source_map.span_to_string(mir_body.span, FileNameDisplayPreference::Local);
                    // println!("{mir_string}");
                    trace_mir_body(mir_body)
                }
            }

            // for def_id in tcx.mir_keys(()) {
            //     let mir = tcx.optimized_mir(def_id.to_def_id());
            //     // Do something with the optimized MIR
            //     println!("{:?}", mir);
            // }
        });
        Compilation::Continue
    }
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
