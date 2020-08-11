#!/bin/sh 

. scripts/env-native-cpu-wihout-avx-512

cargo run --release --bin aquatic_http -- $@
