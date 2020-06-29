#!/bin/bash
# Run this to setup benchmark prerequisites on debian

set -e

echo "This script installs various dependencies for benchmarking aquatic."
echo "It is meant for be run on a Debian buster system after OS install."

read -p "Setup benchmarking prerequisites? [y/N]" -n 1 -r
if [[ ! $REPLY =~ ^[Yy]$ ]]
then
    [[ "$0" = "$BASH_SOURCE" ]] && exit 1 || return 1
fi

apt-get update && apt-get upgrade -y
apt-get install -y vim screen build-essential libz-dev cvs htop curl linux-perf cmake c++filt numactl git nginx

curl https://sh.rustup.rs -sSf | sh

echo 'export RUSTFLAGS="-Ctarget-cpu=native"' >> ~/.profile
echo 'export EDITOR=vim' >> ~/.profile
echo 'net.core.rmem_max=104857600' >> /etc/sysctl.d/local.conf
echo 'net.core.rmem_default=104857600' >> /etc/sysctl.d/local.conf

sysctl -p

source ~/.profile

cd ~
mkdir -p projects
cd projects

# Download and compile opentracker and dependencies

wget https://www.fefe.de/libowfat/libowfat-0.32.tar.xz
tar -xf libowfat-0.32.tar.xz
rm libowfat-0.32.tar.xz
mv libowfat-0.32 libowfat
cd libowfat
make
cd ..

git clone git://erdgeist.org/opentracker
cd opentracker
sed -i "s/^OPTS_production=-O3/OPTS_production=-O3 -march=native -mtune=native/g" Makefile
make
cp opentracker.conf.example config
cd ..

# Download and compile aquatic

git clone https://github.com/greatest-ape/aquatic.git
cd aquatic
export RUSTFLAGS="-C target-cpu=native"
cargo build --release --bin aquatic_udp
cargo build --release --bin aquatic_udp_load_test
cd ..

# Download flamegraph stuff

git clone https://github.com/brendangregg/FlameGraph
cd FlameGraph
echo 'export PATH="$HOME/projects/FlameGraph:$PATH"' >> ~/.profile
source ~/.profile
