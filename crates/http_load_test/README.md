# aquatic_http_load_test: HTTP BitTorrent tracker load tester

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

Load tester for HTTP BitTorrent trackers. Requires Linux 5.8 or later.

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

cargo build --release -p aquatic_http_load_test
```

### Configuring and running

Generate the configuration file:

```sh
./target/release/aquatic_http_load_test -p > "load-test-config.toml"
```

Make necessary adjustments to the file.

Make sure locked memory limits are sufficient:

```sh
ulimit -l 65536
```

First, start the tracker application that you want to test. Then
start the load tester:

```sh
./target/release/aquatic_http_load_test -c "load-test-config.toml"
```

## Copyright and license

Copyright (c) Joakim Frostegård

Distributed under the terms of the Apache License, Version 2.0. Please refer to
the `LICENSE` file in the repository root directory for details.
