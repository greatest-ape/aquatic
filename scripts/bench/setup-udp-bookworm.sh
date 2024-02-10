#!/bin/bash
# Prepare for running aquatic_bench for UDP on Debian 12

# Install dependencies
sudo apt-get update && sudo apt-get upgrade -y
sudo apt-get install -y curl vim htop screen cmake build-essential pkg-config git screen cvs zlib1g zlib1g-dev golang
sudo echo "deb http://deb.debian.org/debian bookworm-backports main contrib" >> /etc/apt/sources.list
sudo apt-get update && sudo apt-get install -y linux-image-amd64/bookworm-backports
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

# Build aquatic
. ./scripts/env-native-cpu-without-avx-512
# export RUSTFLAGS="-C target-cpu=native"
cargo build --profile "release-debug" -p aquatic_udp --features "io-uring"
cargo build --profile "release-debug" -p aquatic_udp_load_test
cargo build --profile "release-debug" -p aquatic_bencher --features udp
git log --oneline | head -n 1

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
make
sudo cp ./opentracker /usr/local/bin/
git log --oneline | head -n 1
cd ..

# Install chihaya
git clone https://github.com/chihaya/chihaya.git
cd chihaya
go build ./cmd/chihaya
sudo cp ./chihaya /usr/local/bin/
git log --oneline | head -n 1
cd ..

rustc --version
gcc --version
go version
lscpu

echo "Finished. Reboot before running aquatic_bencher"