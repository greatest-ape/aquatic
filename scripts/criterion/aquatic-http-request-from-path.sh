#!/bin/sh

export RUSTFLAGS="-C target-cpu=native"

cargo bench --bench bench_request_from_path -- --noplot --baseline no-memchr --load-baseline latest