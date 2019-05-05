#!/bin/bash

set -ex

cd "$(dirname "$0")"/..

export RUSTFLAGS="-D warnings -Ctarget-cpu=native -Ctarget-feature=+rtm"
export RTM="rtm"
export ASAN_FLAG="-Z sanitizer=address"
export ASAN_OPTIONS="detect_odr_violation=0 detect_leaks=0"

if [[ "$TRAVIS_OS_NAME" == "osx" ]]; then
    # no rtm support
    export RTM=""
fi

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

# examples
RUSTFLAGS="${RUSTFLAGS} ${ASAN_FLAG}" \
    time cargo run \
        --release \
        --features debug-alloc,stats,$RTM \
        --example stack

time cargo run \
    --features stats,$RTM \
    --example dining_philosophers

# benchmarks
if [[ -z $RTM ]]; then
    ./x.py bench --features nightly
else
    ./x.py bench --features rtm,nightly
fi
