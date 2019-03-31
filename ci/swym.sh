#!/bin/bash

set -ex

cd "$(dirname "$0")"/..

export RUSTFLAGS="-D warnings -Ctarget-cpu=native -Ctarget-feature=+rtm"

cargo check --no-default-features
cargo check --benches --bins --examples --tests
./x.py test

cargo check --features stats,rtm --benches --bins --examples --tests
./x.py test --features rtm
./x.py test --features stats

ASAN_OPTIONS="detect_odr_violation=0 detect_leaks=0" \
RUSTFLAGS="-Ctarget-cpu=native -Ctarget-feature=+rtm -Z sanitizer=address" \
cargo run \
    --release \
    --target x86_64-unknown-linux-gnu \
    --features stats,rtm \
    --example stack

./x.py bench \
    --target x86_64-unknown-linux-gnu \
    --features rtm
