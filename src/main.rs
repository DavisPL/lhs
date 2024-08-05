// use std::env;
// use std::process;
use clap::Parser;

use mir_analyzer::Args;

#[allow(dead_code)]

fn main() {
    let config = Args::parse();
    // let args: Vec<String> = env::args().collect();

    // let config = Config::build(&args).unwrap_or_else(|err| {
    //     eprintln!("Problem parsing arguments: {err}");
    //     process::exit(1);
    // });

    // println!("Searching for {}", config.query);
    // println!("In file {}", config.file_path);

    // if let Err(e) = rust_find::run(config) {
    //     eprintln!("Application error: {e}");
    //     process::exit(1);
    // }
    mir_analyzer::run(config);
}