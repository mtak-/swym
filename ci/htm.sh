#!/bin/bash

set -e

cd "$(dirname "$0")"/../swym-htm

export RUSTFLAGS="-D warnings"

cargo check --no-default-features
cargo check --benches --bins --examples --tests
cargo check --features rtm --benches --bins --examples --tests
cargo check --features htm --benches --bins --examples --tests
./x.py test
./x.py bench
