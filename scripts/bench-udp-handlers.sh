#!/bin/sh

. ./scripts/env-native-cpu-without-avx-512

cargo run --release -p aquatic_udp_bench -- $@
