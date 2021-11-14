#!/bin/bash

. ./scripts/env-native-cpu-without-avx-512

USAGE="Usage: $0 [mio|io-uring] [ARGS]"

if [ "$1" != "mio" ] && [ "$1" != "glommio" ] && [ "$1" != "io-uring" ]; then
    echo "$USAGE"
else
    if [ "$1" = "mio" ]; then
        cargo run --release --bin aquatic_udp -- "${@:2}"
    elif [ "$1" = "io-uring" ]; then
        cargo run --release --features "with-io-uring" --no-default-features --bin aquatic_udp -- "${@:2}"
    else
        echo "$USAGE"
    fi
fi
