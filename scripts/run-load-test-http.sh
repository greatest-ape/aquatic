#!/bin/sh 

. ./scripts/env-native-cpu-without-avx-512

cargo run --profile "release-debug" --bin aquatic_http_load_test -- $@