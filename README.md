# LHS: Leveraging (not) HIR via Symbolic execution
### A command line utility for searching for analyzing a given Rust code's MIR and verifying `std::fs::write` calls' safeness, in particular, writes to `/proc/self/mem`

## Installation

### Prerequisites
Before you begin, ensure you have the following installed:

- **Rust Nightly**: LHS requires Rust's nightly to utilize the `rustc_private` library.
- **Z3**: You must install Z3 on your system and ensure the Rust Z3 API can be accessed.

### Installing `rustc`

It's crucial to install `rustc` from the official Rust source. If you have installed Rust through third-party package managers like Homebrew, please uninstall it and reinstall using the official Rust installer. 

**Install Rust from the official source:**

Visit the official Rust installation page: [Rust Installation](https://www.rust-lang.org/tools/install)

**Installing the required nightly version:**

Once Rust is installed, you should `cd` into the project directory and run `rustup --version`, which should download and set `rustc` to the correct version from the `toolchain.toml` file.

The `rustc` version we are using is: `rustup default nightly-2024-07-21`

This is what you should be seeing when running the `rustc` and `rustup` version commands:

```bash
❯ rustc --version
rustc 1.82.0-nightly (92c6c0380 2024-07-21)
❯ rustup --version
rustup 1.27.1 (54dd3d00f 2024-04-24)
info: This is the version for the rustup toolchain manager, not the rustc compiler.
info: The currently active `rustc` version is `rustc 1.82.0-nightly (92c6c0380 2024-07-21)`
```

### Installing Z3

If you are on a Macintosh machine, please follow the following instructions to install:

```bash
brew install z3
export LIBRARY_PATH="/opt/homebrew/Cellar/z3/4.13.0/lib"
export Z3_SYS_Z3_HEADER="/opt/homebrew/Cellar/z3/4.13.0/include/z3.h"
```

It is strongly suggested to add the two environmental variables to your `~/.bashrc` or `~/.zshrc` files for easy access in the startup times.

If you are using Ubuntu (or running WSL with Ubuntu), please follow the following instuctions to install:

```bash
sudo apt-get update
sudo apt-get install z3
sudo apt install libclang-dev clang
```

You can check that Z3 installed successfully using `z3 --version`. 

## Usage

To run LHS you need to provide the path to Rust source file and action. 

You can specify an action using the -a flag. We currently support three actions, 
1) `Trace`: trace through function control flow and results
2) `Blocks`: print basic blocks
3) `Local`: print local declarations (variables)
4) `Query`: provides the result of whether or not there is a potential write to `/proc/self/mem`

An example output for the results:
```bash
❯ cargo run -- -s examples/ex1.rs -a query
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.10s
     Running `target/debug/lhs -s examples/ex1.rs -a query`
MIR for function: DefId(0:5 ~ ex1[6c4e]::write_to_file)
examples/ex1.rs:4:1: 7:2
Unsupported Type: ()
Unsupported Type: !
Unsupported Type: ()
Unsupported Type: ()
START: Path 0!
	bb0
	Found fs::write call
Model:
2 -> "/proc/self/./mem"

WARNING: potential write to `/proc/self/mem`
	examples/ex1.rs:5:5: 5:14 (#0)
```

## Examples
Please see the `inputoutput.md` file for input and output examples and expectations.
All code snippets used can also be found in the `examples/` directory.

## Credits 
This project was a result of Prof. Dr. Caleb Stanford's Davis PL research group.
The following members have made contributions to this project (names in alphabetical order):
- Anirudh Basu
- Audrey Gobaco
- Muhammad Hassnain
- Ethan Ng

The authors of this project wish to thank the following projects and papers that have influenced this work:
- MIRChecker [https://dl.acm.org/doi/10.1145/3460120.3484541](https://dl.acm.org/doi/10.1145/3460120.3484541)
    - In particular, this internal meme from the GitHub repository [https://github.com/lizhuohua/rust-mir-checker/issues/15](https://github.com/lizhuohua/rust-mir-checker/issues/15)
- MIRI [https://github.com/rust-lang/miri](https://github.com/rust-lang/miri)
- MIRAI [https://github.com/facebookexperimental/MIRAI](https://github.com/facebookexperimental/MIRAI)
- Cargo Scan [https://github.com/PLSysSec/cargo-scan](https://github.com/PLSysSec/cargo-scan)

A portion of this project was funded by the NSF.

