#!/bin/bash

# Constants
ROOT_DIR="$(cd "$(dirname $0)/../.." && pwd)"
EXAMPLES_DIR="$ROOT_DIR/examples/crates"
RESULTS_FILE="$EXAMPLES_DIR/results.txt"
LHS="$ROOT_DIR/target/debug/lhs"
# WARNING_STRING="WARNING: potential write to \`/proc/self/mem\`"
WARNING_STRING="WARNING: Potential call to sink function:"

# Counters
SAFE_COUNT=0
SAFE_TOTAL=0
UNSAFE_COUNT=0
UNSAFE_TOTAL=0

# Build lhs binary
printf "[INFO] Building LHS...\n"
cargo build --manifest-path "$ROOT_DIR/Cargo.toml" > /dev/null 2>&1

# Prepare results file
printf "Evaluation results:\n" > "$RESULTS_FILE"

# Function to evaluate crates
evaluate_crates() {
    local dir="$EXAMPLES_DIR/$1"

    pushd "$dir" > /dev/null
    for crate in *; do
        printf "[INFO] Evaluating %s crate: %s\n" "$1" "$crate"
        {
            printf "%s\n" "---------------------------"
            printf "Evaluating crate: %s\n" "$crate"
            printf "This crate should be %s\n" "$1"
        } >> "$RESULTS_FILE"

        pushd "$crate" > /dev/null

        rm -rf .cargo
        cargo build > /dev/null 2>&1
        cargo clean -p "$crate" > /dev/null 2>&1

        mkdir -p .cargo
        printf "[build]\nrustc-wrapper = \"%s\"\n" "$LHS" > .cargo/config.toml

        # Capture build output
        BUILD_OUTPUT="$(cargo build 2>&1)"

        # Check for the WARNING string
        if [ "$1" = "safe" ]; then
            if printf "%s\n" "$BUILD_OUTPUT" | grep -q "$WARNING_STRING"; then
                printf "[ALERT] Found unexpected warning in SAFE crate.\n" | tee -a "$RESULTS_FILE"
            else
                printf "[RESULT] Results for the SAFE crate are as expected.\n" | tee -a "$RESULTS_FILE"
                SAFE_COUNT=$((SAFE_COUNT + 1))
            fi
            SAFE_TOTAL=$((SAFE_TOTAL + 1))
        else
            if printf "%s\n" "$BUILD_OUTPUT" | grep -q "$WARNING_STRING"; then
                printf "[RESULT] Results for the UNSAFE crate are as expected.\n" | tee -a "$RESULTS_FILE"
                UNSAFE_COUNT=$((UNSAFE_COUNT + 1))
            else
                printf "[ALERT] Missing expected warning in UNSAFE crate.\n" | tee -a "$RESULTS_FILE"
            fi
            UNSAFE_TOTAL=$((UNSAFE_TOTAL + 1))
        fi

        {
            printf "%s\n" "---------------------------"
            printf "%s\n" "$BUILD_OUTPUT"
        } >> "$RESULTS_FILE"

        popd > /dev/null
    done
    popd > /dev/null
}

pushd "$EXAMPLES_DIR" > /dev/null

evaluate_crates "safe"
evaluate_crates "unsafe"

# Print summary
{
    printf "\nSummary:\n"
    printf "Safe crates accuracy: %d/%d\n" "$SAFE_COUNT" "$SAFE_TOTAL"
    printf "Unsafe crates accuracy: %d/%d\n" "$UNSAFE_COUNT" "$UNSAFE_TOTAL"
} | tee -a "$RESULTS_FILE"


printf "[DONE] Results saved to %s\n" "$RESULTS_FILE"
