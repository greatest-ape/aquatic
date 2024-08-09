# aquatic_bencher

Automated benchmarking of aquatic and other BitTorrent trackers.

Requires Linux 6.0 or later.

Currently, only UDP BitTorrent tracker support is implemented.

## UDP

| Name              | Commit                |
|-------------------|-----------------------|
| [aquatic_udp]     | (use same as bencher) |
| [opentracker]     | 110868e               |
| [chihaya]         | 2f79440               |
| [torrust-tracker] | eaa86a7               |

The commits listed are ones known to work. It might be a good idea to first
test with the latest commits for each project, and if they don't seem to work,
revert to the listed commits.

Chihaya is known to crash under high load.

[aquatic_udp]: https://github.com/greatest-ape/aquatic/
[opentracker]: http://erdgeist.org/arts/software/opentracker/
[chihaya]: https://github.com/chihaya/chihaya
[torrust-tracker]: https://github.com/torrust/torrust-tracker

### Usage

Install dependencies. This is done differently for different Linux
distributions. On Debian 12, run:

```sh
sudo apt-get update
sudo apt-get install -y curl cmake build-essential pkg-config git screen cvs zlib1g zlib1g-dev golang
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
```

Optionally install latest Linux kernel. On Debian 12, you can do so from backports:

```sh
sudo echo "deb http://deb.debian.org/debian bookworm-backports main contrib" >> /etc/apt/sources.list
sudo apt-get update && sudo apt-get install -y linux-image-amd64/bookworm-backports
# You will have to restart to boot into the new kernel
```

Compile aquatic_udp, aquatic_udp_load_test and aquatic_udp_bencher:

```sh
git clone https://github.com/greatest-ape/aquatic.git && cd aquatic
# Optionally enable certain native platform optimizations
. ./scripts/env-native-cpu-without-avx-512
cargo build --profile "release-debug" -p aquatic_udp --features "io-uring"
cargo build --profile "release-debug" -p aquatic_udp_load_test
cargo build --profile "release-debug" -p aquatic_bencher --features udp
cd ..
```

Compile and install opentracker:

```sh
cvs -d :pserver:cvs@cvs.fefe.de:/cvs -z9 co libowfat
cd libowfat
make
cd ..
git clone git://erdgeist.org/opentracker
cd opentracker
# Optionally enable native platform optimizations
sed -i "s/^OPTS_production=-O3/OPTS_production=-O3 -march=native -mtune=native/g" Makefile
make
sudo cp ./opentracker /usr/local/bin/
cd ..
```

Compile and install chihaya:

```sh
git clone https://github.com/chihaya/chihaya.git
cd chihaya
go build ./cmd/chihaya
sudo cp ./chihaya /usr/local/bin/
```

Compile and install torrust-tracker:

```sh
git clone git@github.com:torrust/torrust-tracker.git
cd torrust-tracker
cargo build --release 
cp ./target/release/torrust-tracker /usr/local/bin/
```

You might need to raise locked memory limits:

```sh
ulimit -l 65536
```

Run the bencher:

```sh
cd aquatic
./target/release-debug/aquatic_bencher udp
# or print info on command line arguments
./target/release-debug/aquatic_bencher udp --help
```

If you're running the load test on a virtual machine / virtual server, consider
passing `--min-priority medium --cpu-mode subsequent-one-per-pair` for fairer
results.
