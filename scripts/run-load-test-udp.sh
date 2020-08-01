#!/bin/sh 

# Compile with target-cpu=native but without AVX512 features, since they
# decrease performance.

DISABLE_AVX512=$(rustc --print target-features | grep "    avx512" |
    awk '{print $1}'  | sed 's/^/-C target-feature=-/' | xargs)

export RUSTFLAGS="-C target-cpu=native $DISABLE_AVX512"

cargo run --release --bin aquatic_udp_load_test -- $@
