#!/bin/bash

set -ex

cd "$(dirname "$0")"/../swym-rbtree

# the "+rtm" feature has to be set because the travis linux vm incorrectly thinks it doesn't support
# rtm
export RTM="-Ctarget-feature=+rtm"
if [[ "$TRAVIS_OS_NAME" == "osx" ]]; then
    # no rtm support
    export RTM=""
fi

export RUSTFLAGS="-D warnings -Ctarget-cpu=native ${RTM}"
export ASAN_FLAG="-Z sanitizer=address"
export ASAN_OPTIONS="detect_odr_violation=0 detect_leaks=0"

# cheeck all combinations of features
cargo check --no-default-features --benches --bins --examples --tests
cargo check --benches --bins --examples --tests
cargo check --benches --bins --examples --tests
cargo check --features stats --benches --bins --examples --tests
cargo check --features nightly --benches --bins --examples --tests
cargo check --features stats --benches --bins --examples --tests
cargo check --features nightly --benches --bins --examples --tests
cargo check --features stats,nightly --benches --bins --examples --tests
cargo check --features nightly,stats --benches --bins --examples --tests
# debug-alloc shouldn't change anything
cargo check --features debug-alloc,nightly,stats --benches --bins --examples --tests

# run tests
./x.py test
RUST_TEST_THREADS=1 cargo test --features stats,nightly --lib --tests

# TODO: address sanitizer doesn't work with criterion?
# RUSTFLAGS="${RUSTFLAGS} ${ASAN_FLAG}"
RUST_TEST_THREADS=1 \
    time cargo test --features debug-alloc,stats

# benchmarks
./x.py bench --features nightly
