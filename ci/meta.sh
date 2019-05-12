#!/bin/bash

set -ex

if rustup component add rustfmt-preview ; then
    cargo fmt --all -- --check
fi

export RUSTFLAGS="-D warnings"

cargo doc --all
