# LHS: Leveraging (not) HIR via Symbolic execution
### A command line utility for searching for analyzing a given Rust code's MIR and verifying proc/self/mem safety

## Installation

### Prerequisites
Before you begin, ensure you have the following installed:

- **Rust Nightly**: LHS requires Rust's nightly to utilize the `rustc_private` library.
- **Z3**: You must install Z3 on your system and ensure the Rust Z3 API can be accessed.

### Installing `rustc`

It's crucial to install `rustc` from the official Rust source. If you have installed Rust through third-party package managers like Homebrew, please uninstall it and reinstall using the official Rust installer. 

**Install Rust from the official source:**

1. Visit the official Rust installation page: [Rust Installation](https://www.rust-lang.org/tools/install)
2. Follow the provided instructions to install Rust and add `rustc` to your PATH.
3. Refresh your terminal to ensure the changes take effect. 

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
1) `Trace`: trace through function control flow
2) `Blocks`: print basic blocks
3) `Local`: print local declarations (variables)

To get trace for a file named example.rs you can run:
```bash
cargo run -- -s example.rs -a trace
```

