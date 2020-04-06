#!/bin/sh

export RUSTFLAGS="-C target-cpu=native"

cargo run --release --bin bench_handlers