#!/bin/bash
# Run benchmark, comparing against previous result.

set -e

# Compile with target-cpu=native but without AVX512 features, since they
# decrease performance.

DISABLE_AVX512=$(rustc --print target-features | grep "    avx512" |
    awk '{print $1}'  | sed 's/^/-C target-feature=-/' | xargs)

export RUSTFLAGS="-C target-cpu=native $DISABLE_AVX512"

cargo bench --bench bench_deserialize_announce_request -- --noplot --baseline latest

read -p "Replace previous benchmark result with this one (y/N)? " answer

case ${answer:0:1} in
    y|Y )
        cd aquatic_ws_protocol/target/criterion/deserialize-announce-request/ &&
        rm -r latest &&
        mv new latest &&
        echo "Replaced previous benchmark"
    ;;
esac