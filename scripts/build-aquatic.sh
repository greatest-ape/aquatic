#!/bin/sh 

. ./scripts/env-native-cpu-without-avx-512

cargo build --release --bin aquatic
