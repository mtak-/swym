#!/bin/bash

set -ex

cd "$(dirname "$0")"/../swym-rbtree

export RUSTFLAGS="-D warnings -Ctarget-cpu=native -Ctarget-feature=+rtm"
export ASAN_FLAG="-Z sanitizer=address"
export ASAN_OPTIONS="detect_odr_violation=0 detect_leaks=0"

# cheeck all combinations of features
cargo check --no-default-features --benches --bins --examples --tests
cargo check --benches --bins --examples --tests
cargo check --features "$RTM" --benches --bins --examples --tests
cargo check --features stats --benches --bins --examples --tests
cargo check --features nightly --benches --bins --examples --tests
cargo check --features stats,$RTM --benches --bins --examples --tests
cargo check --features nightly,$RTM --benches --bins --examples --tests
cargo check --features stats,nightly --benches --bins --examples --tests
cargo check --features nightly,stats,$RTM --benches --bins --examples --tests
# debug-alloc shouldn't change anything
cargo check --features debug-alloc,nightly,stats,$RTM --benches --bins --examples --tests

# run tests
./x.py test
RUST_TEST_THREADS=1 cargo test --features stats,nightly,$RTM --lib --tests

# TODO: address sanitizer doesn't work with criterion?
# RUSTFLAGS="${RUSTFLAGS} ${ASAN_FLAG}"
RUST_TEST_THREADS=1 \
    time cargo test --features debug-alloc,stats,$RTM

# benchmarks
./x.py bench --features nightly,$RTM
