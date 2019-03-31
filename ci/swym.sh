#!/bin/bash

set -e

cd "$(dirname "$0")"/..

export RUSTFLAGS="-D warnings -Ctarget-cpu=skylake -Ctarget-feature=+rtm"

cargo check --no-default-features
cargo check --benches --bins --examples --tests
./x.py test

cargo check --features stats,rtm --benches --bins --examples --tests

RUST_TEST_THREADS=1 \
    RUSTFLAGS="-Ctarget-cpu=skylake -Ctarget-feature=+rtm" \
    cargo test --features debug-alloc,stats,rtm --lib --tests

ASAN_OPTIONS="detect_odr_violation=0 detect_leaks=0" \
RUSTFLAGS="-Ctarget-cpu=skylake -Ctarget-feature=+rtm -Z sanitizer=address" \
    cargo run \
        --release \
        --target x86_64-unknown-linux-gnu \
        --features stats,rtm \
        --example stack

./x.py bench \
    --target x86_64-unknown-linux-gnu \
    --features rtm
