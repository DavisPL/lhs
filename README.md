# LHS: Leveraging (not) HIR via Symbolic execution
### A command line utility for analyzing a given Rust code's MIR and verifying `std::fs::write` calls' safeness, in particular, writes to `/proc/self/mem`

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

**IMPROVEMENT!?**

(yes. Definitely an improvement)

To run LHS, you need to have a downloaded project within a `cargo` directory. The following
instructions will run LHS on ONLY the contents of the directory and NOT the dependencies.

1. Compile and build LHS with `cargo build` (this repositiory).
2. Compile and build the target project (the one for analyzing) within that project's directory with
   `cargo build`.
3. Run `cargo clean -p <package/crate name>` in that directory to remove all compiled objects except
   for the dependencies.
4. Add the following lines to the target project's `.cargo/config.toml` (the `.cargo` directory
   should exist on the target project's root directory.
   Make sure you put in your absolute path to the LHS project's `target/debug/lhs` binary.
```toml
[build]
rustc-wrapper = "/absolute/path/to/this/repo/slash/target/debug/lhs"
```
5. Run `cargo build` at the project root directory. You should see output.

Expected results from running LHS on this crate (LHS):
```
❯ cargo build
   Compiling lhs v0.1.0 ([local path to repository])
MIR for function: DefId(0:26 ~ lhs[c37b]::operand::get_operand_def_id)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: *const rustc_middle::mir::ConstOperand<'_>
Unsupported Type: *const ()
START: Path 0!
        bb0
        bb1
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:28 ~ lhs[c37b]::operand::get_operand_const_string)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: *const rustc_middle::mir::ConstOperand<'_>
Unsupported Type: *const ()
START: Path 0!
        bb0
        bb1
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:30 ~ lhs[c37b]::operand::get_operand_local)
START: Path 0!
        bb0
        bb1
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:32 ~ lhs[c37b]::operand::extract_string_from_const)
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
        bb7
        bb8
        bb9
        bb10
        bb11
        bb12
        bb13
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:34 ~ lhs[c37b]::operand::get_operand_span)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: *const rustc_middle::mir::ConstOperand<'_>
Unsupported Type: *const ()
START: Path 0!
        bb0
        bb1
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:179 ~ lhs[c37b]::callback::trace_mir_body)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: [core::fmt::rt::Placeholder; 1]
Unsupported Type: ()
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
        bb7
        bb8
        bb9
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:51 ~ lhs[7528]::main)
Unsupported Type: ()
Unsupported Type: ()
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:52 ~ lhs[7528]::get_callback_mir)
Unsupported Type: ()
Unsupported Type: ()
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:53 ~ lhs[7528]::get_mir_body)
Unsupported Type: ()
Unsupported Type: [&str; 3]
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: ()
Unsupported Type: {closure@src/main.rs:172:43: 172:53}
START: Path 0!
        bb0
        bb1
        Data: ConstAllocation { .. }
        Meta: 15
        Data: ConstAllocation { .. }
        Meta: 19
        bb2
        bb3
        bb4
        bb5
        bb6
        bb7
        bb8
        bb9
        bb10
        bb11
        bb12
        bb13
        bb14
        bb15
        bb16
        bb17
        bb18
        bb19
        bb20
        bb21
        bb45
        bb22
        bb23
        bb24
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:57 ~ lhs[7528]::trace_mir_body)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 1]
Unsupported Type: [core::fmt::rt::Placeholder; 1]
Unsupported Type: ()
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
        bb7
        bb8
        bb9
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:60 ~ lhs[7528]::print_basic_blocks)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 2]
Unsupported Type: [core::fmt::rt::Placeholder; 2]
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
MIR for function: DefId(0:62 ~ lhs[7528]::print_local_decls)
Unsupported Type: ()
Unsupported Type: ()
Unsupported Type: [core::fmt::rt::Argument<'_>; 2]
START: Path 0!
        bb0
        bb1
        bb2
        bb3
        bb4
        bb5
        bb6
Encountered Unknown Terminator. Results may be incorrect.
No potential writes to `/proc/self/mem` detected!
```

*Note*: This is currently under development. It is *extremely* unstable and does not work on large
crates with a lot of dependencies. (NO LONGER THE CASE, hopefully)

**Old instructions**

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

## FAQ
1. What is with the funny name `LHS`?

This came about during project selection when our principle investigator proposed two project directions, one listed on the left hand side of the board, and one on the right.
LHS, short for Left Hand Side, became the *de facto* name of the project.
At the time of release, one of our team members came up with a more applicable name that would also shorten to LHS, now known as "Leveraging (not) HIR via Symbolic execution",
ensuring that LHS continues to live on gloriously as part of the repository.

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

