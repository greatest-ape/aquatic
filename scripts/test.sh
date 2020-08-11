#!/bin/sh

# Test in release mode to avoid quickcheck tests taking forever

# Compile with target-cpu=native but without AVX512 features, since they
# decrease performance.

DISABLE_AVX512=$(rustc --print target-features | grep "    avx512" |
    awk '{print $1}'  | sed 's/^/-C target-feature=-/' | xargs)

export RUSTFLAGS="-C target-cpu=native $DISABLE_AVX512"

# Not chosen for exact values, only to be larger than defaults
export QUICKCHECK_TESTS=2000
export QUICKCHECK_GENERATOR_SIZE=1000

cargo test --all
