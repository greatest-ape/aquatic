#!/bin/sh 

# Compile with target-cpu=native but without AVX512 features, since they
# decrease performance.

DISABLE_AVX512=$(rustc --print target-features | grep "    avx512" |
    awk '{print $1}'  | sed 's/^/-C target-feature=-/' | xargs)

export RUSTFLAGS="-C target-cpu=native $DISABLE_AVX512"

echo "Compiling with RUSTFLAGS=$RUSTFLAGS"

cargo run --release --bin aquatic_http_load_test -- $@