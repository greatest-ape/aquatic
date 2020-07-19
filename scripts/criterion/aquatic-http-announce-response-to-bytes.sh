#!/bin/sh
# Compare against latest. If you commit changes, replace "latest" directory
# with "new" directory after running benchmark.

export RUSTFLAGS="-C target-cpu=native"

cargo bench --bench bench_announce_response_to_bytes -- --noplot --baseline latest