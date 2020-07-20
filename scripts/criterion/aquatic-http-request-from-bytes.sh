#!/bin/sh
# Compare against latest. If you commit changes, replace "latest" directory
# with "new" directory after running benchmark.

export RUSTFLAGS="-C target-cpu=native"

cargo bench --bench bench_request_from_bytes -- --noplot --baseline latest