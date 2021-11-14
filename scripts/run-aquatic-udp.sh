#!/bin/bash

. ./scripts/env-native-cpu-without-avx-512

if [ "$1" != "mio" ] && [ "$1" != "glommio" ] && [ "$1" != "io-uring" ]; then
    echo "Usage: $0 [mio|glommio|io-uring] [ARGS]"
else
    if [ "$1" = "mio" ]; then
        cargo run --release --bin aquatic_udp -- "${@:2}"
    elif [ "$1" = "io-uring" ]; then
        cargo run --release --features "with-io-uring" --no-default-features --bin aquatic_udp -- "${@:2}"
    else
        cargo run --release --features "with-glommio" --no-default-features --bin aquatic_udp -- "${@:2}"
    fi
fi
