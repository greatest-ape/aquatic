# aquatic

[![CargoBuildAndTest](https://github.com/greatest-ape/aquatic/actions/workflows/cargo-build-and-test.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/cargo-build-and-test.yml) [![Test HTTP, UDP and WSS file transfer](https://github.com/greatest-ape/aquatic/actions/workflows/test-transfer.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/test-transfer.yml)

Blazingly fast, multi-threaded BitTorrent tracker written in Rust, consisting
of sub-implementations for different protocols:

[BitTorrent over UDP]: https://libtorrent.org/udp_tracker_protocol.html
[BitTorrent over HTTP]: https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol
[WebTorrent]: https://github.com/webtorrent
[rustls]: https://github.com/rustls/rustls
[native-tls]: https://github.com/sfackler/rust-native-tls
[mio]: https://github.com/tokio-rs/mio
[glommio]: https://github.com/DataDog/glommio

| Name         | Protocol                                   | OS requirements                                            |
|--------------|--------------------------------------------|------------------------------------------------------------|
| aquatic_udp  | [BitTorrent over UDP]                      | Unix-like with [mio] (default) / Linux 5.8+ with [glommio] |
| aquatic_http | [BitTorrent over HTTP] with TLS ([rustls]) | Linux 5.8+                                                 |
| aquatic_ws   | [WebTorrent]                               | Unix-like with [mio] (default) / Linux 5.8+ with [glommio] |

## Usage

### Prerequisites

- Install Rust with [rustup](https://rustup.rs/) (stable is recommended)
- Install cmake with your package manager (e.g., `apt-get install cmake`)
- Unless you're planning to only run aquatic_udp and only the cross-platform,
  mio based implementation, make sure locked memory limits are sufficient.
  You can do this by adding the following lines to `/etc/security/limits.conf`,
  and then logging out and back in:

```
*    hard    memlock    512
*    soft    memlock    512
```

- Clone this git repository and enter it

### Compiling

Compile the implementations that you are interested in:

```sh
# Tell Rust to enable support for all CPU extensions present on current CPU
# except for those relating to AVX-512. This is necessary for aquatic_ws and
# recommended for the other implementations.
. ./scripts/env-native-cpu-without-avx-512

cargo build --release -p aquatic_udp
cargo build --release -p aquatic_udp --features "with-glommio" --no-default-features
cargo build --release -p aquatic_http
cargo build --release -p aquatic_ws
```

### Running

Begin by generating configuration files. They differ between protocols.

```sh
./target/release/aquatic_udp -p > "aquatic-udp-config.toml"
./target/release/aquatic_http -p > "aquatic-http-config.toml"
./target/release/aquatic_ws -p > "aquatic-ws-config.toml"
```

Make adjustments to the files. You will likely want to adjust `address`
(listening address) under the `network` section.

Both `aquatic_http` and `aquatic_ws` requires configuring a TLS certificate
file as well as a private key file to run. More information is available
in the `aquatic_http` subsection of this document.

Once done, run the tracker:

```sh
./target/release/aquatic_udp -c "aquatic-udp-config.toml"
./target/release/aquatic_http -c "aquatic-http-config.toml"
./target/release/aquatic_ws -c "aquatic-ws-config.toml"
```

### Configuration values

Starting a lot more socket workers than request workers is recommended. All
implementations are heavily IO-bound and spend most of their time reading from
and writing to sockets. This part is handled by the `socket_workers`, which
also do parsing, serialisation and access control. They pass announce and
scrape requests to the `request_workers`, which update internal tracker state
and pass back responses.

#### Access control

Access control by info hash is supported for all protocols. The relevant part
of configuration is:

```toml
[access_list]
mode = 'off' # Change to 'black' (blacklist) or 'white' (whitelist)
path = '' # Path to text file with newline-delimited hex-encoded info hashes
```

The file is read on start and when the program receives `SIGUSR1`.

#### More information

More documentation of the various configuration options might be available
under `src/lib/config.rs` in directories `aquatic_udp`, `aquatic_http` and
`aquatic_ws`.

## Details on implementations

### aquatic_udp: UDP BitTorrent tracker

Aims to implements the
[UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html),
except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6 (BitTorrent UDP protocol doesn't support IPv6 very well,
however.)

For optimal performance, enable setting of core affinities in configuration.

#### Benchmarks

[opentracker]: http://erdgeist.org/arts/software/opentracker/

Server responses per second, best result in bold:

| workers | aquatic   | [opentracker] |
|---------|-----------|---------------|
| 1       | n/a       | __232k__      |
| 2       | __309k__  | 293k          |
| 3       | __597k__  | 397k          |
| 4       | __603k__  | 481k          |
| 6       | __757k__  | 587k          |
| 8       | __850k__  | 431k          |
| 10      | __826k__  | 165k          |
| 16      | __785k__  | 139k          |

Please refer to `documents/aquatic-udp-load-test-2021-08-19.pdf` for more details.

#### Alternative implementation using io_uring

[io_uring]: https://en.wikipedia.org/wiki/Io_uring
[glommio]: https://github.com/DataDog/glommio

There is an alternative implementation that utilizes [io_uring] by running on
[glommio]. It only runs on Linux and requires a recent kernel (version 5.8 or later).
In some cases, it performs even better than the cross-platform implementation.

### aquatic_http: HTTP BitTorrent tracker

[HTTP BitTorrent protocol]: https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol

Aims for compatibility with the [HTTP BitTorrent protocol], with some exceptions:

  * Only runs over TLS
  * Doesn't track of the number of torrent downloads (0 is always sent)
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_http` has not been tested as much as `aquatic_udp` but likely works
fine.

#### TLS

A TLS certificate file (DER-encoded X.509) and a corresponding private key file
(DER-encoded ASN.1 in either PKCS#8 or PKCS#1 format) are required. Set their
paths in the configuration file, e.g.:

```toml
[network]
address = '0.0.0.0:3000'
tls_certificate_path = './cert.pem'
tls_private_key_path = './key.pem'
```

### aquatic_ws: WebTorrent tracker

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients, with some exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes


#### TLS: mio version

To run over TLS, a pkcs12 file (`.pkx`) is needed. It can be generated from
Let's Encrypt certificates as follows, assuming you are in the directory where
they are stored:

```sh
openssl pkcs12 -export -out identity.pfx -inkey privkey.pem -in cert.pem -certfile fullchain.pem
```

Enter a password when prompted. Then move `identity.pfx` somewhere suitable,
and enter the path into the tracker configuration field `tls_pkcs12_path`. Set
the password in the field `tls_pkcs12_password` and set `use_tls` to true.

#### TLS: glommio version

The glommio version only runs over TLS. For setup instructions, please see
`aquatic_http` TLS section above.

#### Benchmarks

[wt-tracker]: https://github.com/Novage/wt-tracker
[bittorrent-tracker]: https://github.com/webtorrent/bittorrent-tracker

The following benchmark is not very realistic, as it simulates a small number
of clients, each sending a large number of requests. Nonetheless, I think that
it gives a useful indication of relative performance.

Server responses per second, best result in bold:

| workers | aquatic    | [wt-tracker] | [bittorrent-tracker] |
|---------|------------|--------------|----------------------|
| 1       | n/a        | __117k__     | 45k                  |
| 2       | __225k__   | n/a          | n/a                  |
| 4       | __627k__   | n/a          | n/a                  |
| 6       | __831k__*  | n/a          | n/a                  |
| 8       | __1209k__* | n/a          | n/a                  |
| 10      | __1455k__* | n/a          | n/a                  |
| 12      | __1650k__* | n/a          | n/a                  |
| 14      | __1804k__* | n/a          | n/a                  |
| 16      | __1789k__* | n/a          | n/a                  |

\* Using a VPS with 32 vCPUs. The other measurements were made using a 16 vCPU VPS.

Please refer to `documents/aquatic-ws-load-test-2021-08-18.pdf` for more details.

__Note__: these benchmarks were made with the mio-based implementation.

## Load testing

There are load test binaries for all protocols. They use a CLI structure
similar to the trackers and support generation and loading of configuration
files.

To run, first start the tracker that you want to test. Then run the
corresponding load test binary:

```sh
./scripts/run-load-test-udp.sh
./scripts/run-load-test-http.sh
./scripts/run-load-test-ws.sh
```

To fairly compare HTTP performance to opentracker, set keepalive to false in
`aquatic_http` settings.

## Copyright and license

Copyright (c) 2020-2021 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
