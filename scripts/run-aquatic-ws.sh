#!/bin/bash

. ./scripts/env-native-cpu-without-avx-512

if [[ -z $1 ]]; then
    echo "Usage: $0 [mio|glommio]"
else
    if [ "$1" = "mio" ]; then
        cargo run --release --bin aquatic_ws -- "${@:2}"
    else
        cargo run --release --features "with-glommio" --no-default-features --bin aquatic_ws -- "${@:2}"
    fi
fi

