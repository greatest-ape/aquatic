#!/bin/sh 

. ./scripts/env-native-cpu-without-avx-512

cargo run --profile "release-debug" -p aquatic_ws_load_test -- $@
