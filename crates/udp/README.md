# aquatic_udp: high-performance open UDP BitTorrent tracker

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

High-performance open UDP BitTorrent tracker for Unix-like operating systems.

Features at a glance:

- Multithreaded design for handling large amounts of traffic
- All data is stored in-memory (no database needed)
- IPv4 and IPv6 support
- Supports forbidding/allowing info hashes
- Prometheus metrics
- Automated CI testing of full file transfers

Known users:

- [explodie.org public tracker](https://explodie.org/opentracker.html) (`udp://explodie.org:6969`), typically [serving ~100,000 requests per second](https://explodie.org/tracker-stats.html)

This is the most mature implementation in the aquatic family. I consider it fully ready for production use.

## Performance

![UDP BitTorrent tracker throughput](../../documents/aquatic-udp-load-test-2024-02-10.png)

More benchmark details are available [here](../../documents/aquatic-udp-load-test-2024-02-10.md).

## Usage

### Compiling

- Install Rust with [rustup](https://rustup.rs/) (latest stable release is recommended)
- Install build dependencies with your package manager (e.g., `apt-get install cmake build-essential`)
- Clone this git repository and build the application:

```sh
git clone https://github.com/greatest-ape/aquatic.git && cd aquatic

# Recommended: tell Rust to enable support for all SIMD extensions present on
# current CPU except for those relating to AVX-512. (If you run a processor
# that doesn't clock down when using AVX-512, you can enable those instructions
# too.)
. ./scripts/env-native-cpu-without-avx-512

cargo build --release -p aquatic_udp
```

### Configuring and running

Generate the configuration file:

```sh
./target/release/aquatic_udp -p > "aquatic-udp-config.toml"
```

Make necessary adjustments to the file. You will likely want to adjust
listening addresses under the `network` section.

Once done, start the application:

```sh
./target/release/aquatic_udp -c "aquatic-udp-config.toml"
```

If your server is pointed to by domain `example.com` and you configured the
tracker to run on port 3000, people can now use it by adding the URL
`udp://example.com:3000` to their torrent files or magnet links.

### Load testing

A load test application is available. It supports generation and loading of
configuration files in a similar manner to the tracker application.

After starting the tracker, run the load tester:

```sh
. ./scripts/env-native-cpu-without-avx-512 # Optional

cargo run --release -p aquatic_udp_load_test -- --help
```

## Details

Implements [BEP 015](https://www.bittorrent.org/beps/bep_0015.html) ([more details](https://libtorrent.org/udp_tracker_protocol.html)) with the following exceptions:

- Ignores IP addresses sent in announce requests. The packet source IP is always used.
- Doesn't track the number of torrent downloads (0 is always sent). 

## Copyright and license

Copyright (c) Joakim Frostegård

Distributed under the terms of the Apache License, Version 2.0. Please refer to
the `LICENSE` file in the repository root directory for details.
