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

cargo check --no-default-features
cargo check --benches --bins --examples --tests
./x.py test

cargo check --features stats,$RTM --benches --bins --examples --tests

RUST_TEST_THREADS=1 \
    cargo test --features debug-alloc,stats,$RTM --lib --tests

RUSTFLAGS="${RUSTFLAGS} ${ASAN_FLAG}" \
    cargo run \
        --release \
        --features stats,$RTM \
        --example stack

cargo run \
    --features stats,$RTM \
    --example dining_philosophers

if [[ -z $RTM ]]; then
    ./x.py bench
else
    ./x.py bench --features rtm
fi
