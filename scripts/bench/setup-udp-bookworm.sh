#!/bin/bash
# Prepare for running aquatic_bench for UDP on Debian 12

# Install dependencies
sudo apt-get update && apt-get upgrade -y
sudo apt-get install -y curl vim htop screen cmake build-essential pkg-config git screen cvs zlib1g zlib1g-dev golang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Build aquatic
. ./scripts/env-native-cpu-without-avx-512
cargo build --profile "release-debug" -p aquatic_udp
cargo build --profile "release-debug" -p aquatic_udp_load_test
cargo build --profile "release-debug" -p aquatic_bencher --features udp

cd $HOME
mkdir -p projects
cd projects

# Install opentracker
cvs -d :pserver:cvs@cvs.fefe.de:/cvs -z9 co libowfat
cd libowfat
make
cd ..
git clone git://erdgeist.org/opentracker
cd opentracker
sed -i "s/^OPTS_production=-O3/OPTS_production=-O3 -march=native -mtune=native/g" Makefile
sed -i "s/if \(numwant > 200\) numwant = 200/if (numwant > 50) numwant = 50/g" ot_udp.c
make
sudo cp ./opentracker /usr/local/bin/
cd ..

# Install chihaya
git clone https://github.com/chihaya/chihaya.git
cd chihaya
go build ./cmd/chihaya
sudo cp ./chihaya /usr/local/bin/
cd ..