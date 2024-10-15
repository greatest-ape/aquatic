# aquatic_ws: high-performance open WebTorrent tracker

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

High-performance WebTorrent tracker for Linux 5.8 or later.

Features at a glance:

- Multithreaded design for handling large amounts of traffic
- All data is stored in-memory (no database needed)
- IPv4 and IPv6 support
- Supports forbidding/allowing info hashes
- Prometheus metrics
- Automated CI testing of full file transfers

Known users:

- [tracker.webtorrent.dev](https://tracker.webtorrent.dev) (`wss://tracker.webtorrent.dev`)

## Performance

![WebTorrent tracker throughput comparison](../../documents/aquatic-ws-load-test-illustration-2023-01-25.png)

More details are available [here](../../documents/aquatic-ws-load-test-2023-01-25.pdf).

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

cargo build --release -p aquatic_ws
```

### Configuring

Generate the configuration file:

```sh
./target/release/aquatic_ws -p > "aquatic-ws-config.toml"
```

Make necessary adjustments to the file. You will likely want to adjust `address`
(listening address) under the `network` section.

To run over TLS, configure certificate and private key files.

Running behind a reverse proxy is supported, as long as IPv4 requests are
proxied to IPv4 requests, and IPv6 requests to IPv6 requests.

### Running

Make sure locked memory limits are sufficient:
- If you're using a systemd service file, add `LimitMEMLOCK=65536000` to it
- If you're using openrc, make sure you have `rc_ulimit='-l 65536'` in your init script
- Otherwise, add the following lines to
`/etc/security/limits.conf`, and then log out and back in:

```
*    hard    memlock    65536
*    soft    memlock    65536
```

In Alpine Linux you will likely need a [higher limit](https://github.com/greatest-ape/aquatic/issues/211).

Once done, start the application:

```sh
./target/release/aquatic_ws -c "aquatic-ws-config.toml"
```

If your server is pointed to by domain `example.com` and you configured the
tracker to run on port 3000, people can now use it by adding the URL
`wss://example.com:3000` to their torrent files or magnet links.

### Load testing

A load test application is available. It supports generation and loading of
configuration files in a similar manner to the tracker application.

After starting the tracker, run the load tester:

```sh
. ./scripts/env-native-cpu-without-avx-512 # Optional

cargo run --release -p aquatic_ws_load_test -- --help
```

## Details

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients. Notes:

  * Doesn't track the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_ws` has not been tested as much as `aquatic_udp`, but likely works
fine in production.

## Architectural overview

![Architectural overview of aquatic](../../documents/aquatic-architecture-2024.svg)

## Copyright and license

Copyright (c) Joakim Frostegård

Distributed under the terms of the Apache License, Version 2.0. Please refer to
the `LICENSE` file in the repository root directory for details.

