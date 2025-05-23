#!/bin/bash

# Do not move this script or run it from a directory other than lhs/examples/crates
cd ../..
cargo build
cd target/debug
LHS=$(pwd)/lhs
cd ../../examples/crates

printf "Evaluation results:\n" > results.txt

cd safe
for safe in *; do
    printf "%s\n" $safe
    cd $safe
    printf "%s\n" "---------------------------" >> ../../results.txt
    printf "%s\n" "---------------------------" >> ../../results.txt
    printf "Evaluating crate: %s\n" $safe >> ../../results.txt
    printf "This crate should be safe\n" >> ../../results.txt
    rm -rf .cargo
    cargo build
    cargo clean -p $safe
    mkdir .cargo
    printf "[build]\nrustc-wrapper = \"%s\"" $LHS > .cargo/config.toml
    cargo build >> ../../results.txt 2>&1
    cd ..
done
cd ..

cd unsafe
for unsafe in *; do
    printf "%s\n" $unsafe
    cd $unsafe
    printf "%s\n" "---------------------------" >> ../../results.txt
    printf "%s\n" "---------------------------" >> ../../results.txt
    printf "Evaluating crate: %s\n" $unsafe >> ../../results.txt
    printf "This crate should be unsafe\n" >> ../../results.txt
    rm -rf .cargo
    cargo build
    cargo clean -p $unsafe
    mkdir .cargo
    printf "[build]\nrustc-wrapper = \"%s\"" $LHS > .cargo/config.toml
    cargo build >> ../../results.txt 2>&1
    cd ..
done
cd ..

