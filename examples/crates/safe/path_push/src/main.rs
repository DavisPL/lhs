use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let mut out_file = Path::new(
        &env::args()
            .nth(1)
            .expect("No argument provided"),
    )
    .to_path_buf();

    out_file.push("codegen.rs");

    let contents = "// your generated code here\n";
    fs::write(out_file, contents).unwrap();
}
