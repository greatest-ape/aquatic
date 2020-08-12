#!/bin/sh 

. ./scripts/env-native-cpu-without-avx-512

cargo run --release --bin aquatic -- $@
