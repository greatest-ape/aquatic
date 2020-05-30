#!/bin/bash 

# Build aquatic_udp to generate profile guided optimization data. If using
# lto, build errors are avoided with lto = "thin".
#
# Afterwards, run the executable. No PDO output will be generated if it is
# forced to quit, so something like a ctrl-c handler will need to be
# implemented. I just modified aquatic_udp to quit after 10 minutes when
# testing.
#
# More info about PGO:
# https://doc.rust-lang.org/rustc/profile-guided-optimization.html

HERE=$(pwd)
PGO_DATA_PATH="$HERE/tmp/pgo-data"

rm -r "$PGO_DATA_PATH"

export RUSTFLAGS="-C target-cpu=native -Cprofile-generate=$PGO_DATA_PATH"

cargo build --release --target=x86_64-apple-darwin --bin aquatic_udp