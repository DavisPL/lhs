# LRHS
### A command line utility for searching for analyzing a given Rust code's MIR and verifying proc/self/mem safety

## Installation

### Prerequisites
Before you begin, ensure you have the following installed:

- **Rust Nightly**: LRHS requires Rust's nightly to utilize the `rustc_private` library.

### Installing `rustc`

It's crucial to install `rustc` from the official Rust source. If you have installed Rust through third-party package managers like Homebrew, please uninstall it and reinstall using the official Rust installer. 

**Install Rust from the official source:**

1. Visit the official Rust installation page: [Rust Installation](https://www.rust-lang.org/tools/install)
2. Follow the provided instructions to install Rust and add `rustc` to your PATH.
3. Refresh your terminal to ensure the changes take effect. 

**Installing the required nightly version:**

Once Rust is installed, set the required nightly version:

`rustup default nightly-2024-07-21`

### Clone the Repository

To install LRHS, clone the repository and ensure you use the `--recursive` flag to include submodules:

`git clone --recursive https://github.com/DavisPL/lrhs.git`

## Usage

To run LRHS with an example file:

`cargo run -- -s [your_file.rs]`


Replace \`[your_file.rs]\` with the path to your Rust source file.
