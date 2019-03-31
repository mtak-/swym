#!/bin/bash

set -ex

cd "$(dirname "$0")"/../swym-htm

export RUSTFLAGS="-D warnings"

cargo check --no-default-features
cargo check --benches --bins --examples --tests
cargo check --features rtm --benches --bins --examples --tests
cargo check --features htm --benches --bins --examples --tests
./x.py test

RUSTFLAGS="-Ctarget-cpu=native -Ctarget-feature=+rtm" \
./x.py bench \
    --target x86_64-unknown-linux-gnu \
    --features rtm
