#!/bin/bash
# Run benchmark, comparing against previous result.

set -e

. ./scripts/env-native-cpu-without-avx-512

cargo bench --bench bench_request_from_bytes -- --noplot --baseline latest

read -p "Replace previous benchmark result with this one (y/N)? " answer

case ${answer:0:1} in
    y|Y )
        cd aquatic_http_protocol/target/criterion/request-from-bytes/ &&
        rm -r latest &&
        mv new latest &&
        echo "Replaced previous benchmark"
    ;;
esac