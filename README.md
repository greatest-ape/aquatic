# aquatic: high-performance open BitTorrent tracker

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

High-performance open BitTorrent tracker, consisting
of sub-implementations for different protocols:

[BitTorrent over UDP]: https://libtorrent.org/udp_tracker_protocol.html
[BitTorrent over HTTP]: https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol
[WebTorrent]: https://github.com/webtorrent
[rustls]: https://github.com/rustls/rustls
[native-tls]: https://github.com/sfackler/rust-native-tls
[mio]: https://github.com/tokio-rs/mio
[glommio]: https://github.com/DataDog/glommio

| Name         | Protocol                                     | OS requirements              |
|--------------|----------------------------------------------|------------------------------|
| aquatic_udp  | [BitTorrent over UDP]                        | Unix-like (using [mio])      |
| aquatic_http | [BitTorrent over HTTP] over TLS ([rustls])   | Linux 5.8+ (using [glommio]) |
| aquatic_ws   | [WebTorrent], optionally over TLS ([rustls]) | Linux 5.8+ (using [glommio]) |

Features at a glance:

- Multithreaded design for handling large amounts of traffic
- All data is stored in-memory (no database needed)
- IPv4 and IPv6 support
- Supports forbidding/allowing info hashes
- Built-in TLS support (no reverse proxy needed)
- Automated CI testing of full file transfers

Known users:

- [explodie.org public tracker](https://explodie.org/opentracker.html) (`udp://explodie.org:6969`), typically [serving ~80,000 requests per second](https://explodie.org/tracker-stats.html)

## Usage

### Compiling

- Install Rust with [rustup](https://rustup.rs/) (latest stable release is recommended)
- Install cmake with your package manager (e.g., `apt-get install cmake`)
- Clone this git repository and enter the directory
- Build the implementations that you are interested in:

```sh
# Tell Rust to enable support for all SIMD extensions present on current CPU
# except for those relating to AVX-512. SIMD is required for aquatic_ws and
# recommended for the other implementations. If you run a processor that
# doesn't clock down when using AVX-512, you can enable those instructions
# too.
. ./scripts/env-native-cpu-without-avx-512

cargo build --release -p aquatic_udp
cargo build --release -p aquatic_http
cargo build --release -p aquatic_ws
```

### Configuring

Generate configuration files. They come with comments and differ between protocols.

```sh
./target/release/aquatic_udp -p > "aquatic-udp-config.toml"
./target/release/aquatic_http -p > "aquatic-http-config.toml"
./target/release/aquatic_ws -p > "aquatic-ws-config.toml"
```

Make adjustments to the files. You will likely want to adjust `address`
(listening address) under the `network` section.

Note that both `aquatic_http` and `aquatic_ws` require configuring certificate
and private key files to run over TLS. `aquatic_http` __only__ runs over TLS.
More details are available in the respective configuration files.

#### Workers

To increase performance, number of worker threads can be increased.
Recommended proportions based on number of physical CPU cores:

<table>
 <tr>
  <td></td>
  <th colspan="1">udp</th>
  <th colspan="1">http</th>
  <th colspan="2">ws</th>
 </tr>
 <tr>
  <th scope="row">CPU cores (N)</th>
  <td>N</td>
  <td>N</td>
  <td>1-7</td>
  <td>>=8</td>
 </tr>
 <tr>
  <th scope="row">Swarm workers</th>
  <td>1</td>
  <td>1</td>
  <td>1</td>
  <td>2</td>
 </tr>
 <tr>
  <th scope="row">Socket workers</th>
  <td>N</td>
  <td>N</td>
  <td>N</td>
  <td>N-2</td>
 </tr>
</table>

#### Access control

Access control by info hash is supported for all protocols. The relevant part
of configuration is:

```toml
[access_list]
# Access list mode. Available modes are allow, deny and off.
mode = "off"
# Path to access list file consisting of newline-separated hex-encoded info hashes.
path = ""
```

The file is read on start and when the program receives `SIGUSR1`. If initial
parsing fails, the program exits. Later failures result in in emitting of
an error-level log message, while successful updates of the access list result
in emitting of an info-level log message.

#### Prometheus

Exporting [Prometheus](https://prometheus.io/) metrics is supported. Activate
the endpoint in the configuration file:

##### aquatic_udp

```toml
[statistics]
run_prometheus_endpoint = true
prometheus_endpoint_address = "0.0.0.0:9000"
```

##### aquatic_http / aquatic_ws

```toml
[metrics]
run_prometheus_endpoint = true
prometheus_endpoint_address = "0.0.0.0:9000"
```

### Running

If you're running `aquatic_http` or `aquatic_ws`, please make sure locked memory
limits are sufficient:
- If you're using a systemd service file, add `LimitMEMLOCK=65536000` to it
- Otherwise, add the following lines to
`/etc/security/limits.conf`, and then log out and back in:

```
*    hard    memlock    65536
*    soft    memlock    65536
```

Once done, start the application:

```sh
./target/release/aquatic_udp -c "aquatic-udp-config.toml"
./target/release/aquatic_http -c "aquatic-http-config.toml"
./target/release/aquatic_ws -c "aquatic-ws-config.toml"
```

If your server is pointed to by domain `example.com` and you configured the
tracker to run on port 3000, people can now use it by adding its URL to their
torrent files or magnet links:

| Implementation | Announce URL                        |
|----------------|-------------------------------------|
| aquatic_udp    | `udp://example.com:3000`            |
| aquatic_http   | `https://example.com:3000/announce` |
| aquatic_ws     | `wss://example.com:3000`            |

## Details on implementations

### aquatic_udp: UDP BitTorrent tracker

[BEP 015]: https://www.bittorrent.org/beps/bep_0015.html

Implements:
  * [BEP 015]: UDP BitTorrent tracker protocol ([more details](https://libtorrent.org/udp_tracker_protocol.html)). Exceptions:
    * Doesn't care about IP addresses sent in announce requests. The packet
      source IP is always used.
    * Doesn't track the number of torrent downloads (0 is always sent). 

This is the most mature of the implementations. I consider it ready for production use.

#### Performance

![UDP BitTorrent tracker throughput comparison](./documents/aquatic-udp-load-test-illustration-2023-01-11.png)

More details are available [here](./documents/aquatic-udp-load-test-2023-01-11.pdf).

#### io_uring

An experimental io_uring backend can be compiled in by passing the `io-uring`
feature. Currently, Linux 6.0 or later is required. The application will
attempt to fall back to the mio backend if your kernel is not supported.

```sh
cargo build --release -p aquatic_udp --features "io-uring"
```

### aquatic_http: HTTP BitTorrent tracker

[BEP 003]: https://www.bittorrent.org/beps/bep_0003.html
[BEP 007]: https://www.bittorrent.org/beps/bep_0007.html
[BEP 023]: https://www.bittorrent.org/beps/bep_0023.html
[BEP 048]: https://www.bittorrent.org/beps/bep_0048.html

Implements:
  * [BEP 003]: HTTP BitTorrent protocol ([more details](https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol)). Exceptions:
    * Only runs over TLS
    * Doesn't track the number of torrent downloads (0 is always sent)
    * Only compact responses are supported
  * [BEP 023]: Compact HTTP responses
  * [BEP 007]: IPv6 support
  * [BEP 048]: HTTP scrape support. Notes:
    * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_http` has not been tested as much as `aquatic_udp` but likely works
fine in production.

Running behind a reverse proxy is currently not supported due to the
[difficulties of determining the originating IP address](https://adam-p.ca/blog/2022/03/x-forwarded-for/)
without knowing the exact setup.

#### Performance

![HTTP BitTorrent tracker throughput comparison](./documents/aquatic-http-load-test-illustration-2023-01-25.png)

More details are available [here](./documents/aquatic-http-load-test-2023-01-25.pdf).

### aquatic_ws: WebTorrent tracker

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients. Notes:

  * Doesn't track the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_ws` has not been tested as much as `aquatic_udp` but likely works
fine in production.

Running behind a reverse proxy is supported, as long as IPv4 requests are
proxied to IPv4 requests, and IPv6 requests to IPv6 requests.

#### Performance

![WebTorrent tracker throughput comparison](./documents/aquatic-ws-load-test-illustration-2023-01-25.png)

More details are available [here](./documents/aquatic-ws-load-test-2023-01-25.pdf).

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

To fairly compare HTTP performance to opentracker, set `keep_alive` to false in
`aquatic_http` settings.

## Architectural overview

![Architectural overview of aquatic](./documents/aquatic-architecture-2022-02-02.svg)

## Copyright and license

Copyright (c) 2020-2023 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
