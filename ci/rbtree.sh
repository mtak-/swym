#!/bin/bash

set -ex

cd "$(dirname "$0")"/../swym-rbtree

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
RUST_TEST_THREADS=1 RUSTFLAGS="${RUSTFLAGS} ${ASAN_FLAG}" \
    time cargo test \
        --features debug-alloc,stats,$RTM

# benchmarks
./x.py bench insert:: --features nightly,$RTM 

# these benchmarks are run one at a time due to high memory usage
./x.py bench --features nightly,$RTM rbtree::contains_key_01
./x.py bench --features nightly,$RTM rbtree::contains_key_02
./x.py bench --features nightly,$RTM rbtree::contains_key_03
./x.py bench --features nightly,$RTM rbtree::contains_key_04
./x.py bench --features nightly,$RTM rbtree::contains_key_05
./x.py bench --features nightly,$RTM rbtree::contains_key_06
./x.py bench --features nightly,$RTM rbtree::contains_key_07
./x.py bench --features nightly,$RTM rbtree::contains_key_08
./x.py bench --features nightly,$RTM rbtree::entry_01
./x.py bench --features nightly,$RTM rbtree::entry_02
./x.py bench --features nightly,$RTM rbtree::entry_03
./x.py bench --features nightly,$RTM rbtree::entry_04
./x.py bench --features nightly,$RTM rbtree::entry_05
./x.py bench --features nightly,$RTM rbtree::entry_06
./x.py bench --features nightly,$RTM rbtree::entry_07
./x.py bench --features nightly,$RTM rbtree::entry_08
./x.py bench --features nightly,$RTM rbtree::get_01
./x.py bench --features nightly,$RTM rbtree::get_02
./x.py bench --features nightly,$RTM rbtree::get_03
./x.py bench --features nightly,$RTM rbtree::get_04
./x.py bench --features nightly,$RTM rbtree::get_05
./x.py bench --features nightly,$RTM rbtree::get_06
./x.py bench --features nightly,$RTM rbtree::get_07
./x.py bench --features nightly,$RTM rbtree::get_08
./x.py bench --features nightly,$RTM rbtree::insert_01
./x.py bench --features nightly,$RTM rbtree::insert_02
./x.py bench --features nightly,$RTM rbtree::insert_03
./x.py bench --features nightly,$RTM rbtree::insert_04
./x.py bench --features nightly,$RTM rbtree::insert_05
./x.py bench --features nightly,$RTM rbtree::insert_06
./x.py bench --features nightly,$RTM rbtree::insert_07
./x.py bench --features nightly,$RTM rbtree::insert_08
