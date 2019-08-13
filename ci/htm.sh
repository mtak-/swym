#!/bin/bash

set -ex

cd "$(dirname "$0")"/../swym-htm

# the "+rtm" feature has to be set because the travis linux vm incorrectly thinks it doesn't support
# rtm
export RUSTFLAGS="-D warnings -Ctarget-feature=+rtm"

cargo check --no-default-features --benches --bins --examples --tests
cargo check --benches --bins --examples --tests
cargo check --features nightly --benches --bins --examples --tests
cargo check --features htm --benches --bins --examples --tests
./x.py test
./x.py test --release -- --nocapture
./x.py bench
