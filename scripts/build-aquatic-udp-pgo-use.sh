#!/bin/bash

# Build aquatic_udp with profile guided optimization, based on previously
# generated data. In very limited benchmarking with aquatic_udp_load_test
# using one socket worker on macOS, performance is improved by 10%.

set -e

HERE=$(pwd)
PGO_DATA_PATH="$HERE/tmp/pgo-data"

# Path to llvm-profdata might not work if you have multiple toolchains
# installed. If you don't have llvm-profdata installed at all, run
# `rustup component add llvm-tools-preview`
~/.rustup/toolchains/stable-x86_64-*/lib/rustlib/x86_64-*/bin/llvm-profdata merge --output "$PGO_DATA_PATH/merged.profdata" "$PGO_DATA_PATH";

export RUSTFLAGS="-C target-cpu=native -Cprofile-use=$PGO_DATA_PATH/merged.profdata -Cllvm-args=-pgo-warn-missing-function"

cargo build --release --target=x86_64-apple-darwin --bin aquatic_udp