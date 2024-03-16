# aquatic_udp_load_test: UDP BitTorrent tracker load tester

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

High-performance load tester for UDP BitTorrent trackers, for Unix-like operating systems.

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

cargo build --release -p aquatic_udp_load_test
```

### Configuring and running

Generate the configuration file:

```sh
./target/release/aquatic_udp_load_test -p > "load-test-config.toml"
```

Make necessary adjustments to the file.

Once done, first start the tracker application that you want to test. Then,
start the load tester:

```sh
./target/release/aquatic_udp_load_test -c "load-test-config.toml"
```

## Copyright and license

Copyright (c) Joakim Frostegård

Distributed under the terms of the Apache License, Version 2.0. Please refer to
the `LICENSE` file in the repository root directory for details.
