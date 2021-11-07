#!/bin/bash

. ./scripts/env-native-cpu-without-avx-512

if [ "$1" != "mio" ] && [ "$1" != "glommio" ]; then
    echo "Usage: $0 [mio|glommio] [ARGS]"
else
    if [ "$1" = "mio" ]; then
        cargo run --release --bin aquatic_udp -- "${@:2}"
    else
        cargo run --release --features "with-glommio" --no-default-features --bin aquatic_udp -- "${@:2}"
    fi
fi
