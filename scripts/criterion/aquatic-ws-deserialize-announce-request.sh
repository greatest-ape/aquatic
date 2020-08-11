#!/bin/bash
# Run benchmark, comparing against previous result.

set -e

. ./scripts/env-native-cpu-without-avx-512

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