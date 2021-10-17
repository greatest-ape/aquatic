# aquatic

[![CargoBuildAndTest](https://github.com/greatest-ape/aquatic/actions/workflows/cargo-build-and-test.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/cargo-build-and-test.yml) [![Test HTTP, UDP and WSS file transfer](https://github.com/greatest-ape/aquatic/actions/workflows/test-transfer.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/test-transfer.yml)

Blazingly fast, multi-threaded BitTorrent tracker written in Rust.

Consists of three sub-implementations for different protocols:
  * `aquatic_udp`: BitTorrent over UDP. Implementation achieves 45% higher throughput
    than opentracker (see benchmarks below)
  * `aquatic_http`: BitTorrent over HTTP/TLS (slightly experimental)
  * `aquatic_ws`: WebTorrent (experimental)

## Copyright and license

Copyright (c) 2020-2021 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Installation prerequisites

- Install Rust with [rustup](https://rustup.rs/) (stable is recommended)
- Install cmake with your package manager (e.g., `apt-get install cmake`)
- On GNU/Linux, also install the OpenSSL components necessary for dynamic
  linking (e.g., `apt-get install libssl-dev`)
- Clone the git repository and refer to the next section.

## Compile and run

To compile the master executable for all protocols, run:

```sh
./scripts/build-aquatic.sh
```

To start the tracker for a protocol with default settings, run:

```sh
./target/release/aquatic udp
./target/release/aquatic http
./target/release/aquatic ws
```

To print default settings to standard output, pass the "-p" flag to the binary:

```sh
./target/release/aquatic udp -p
./target/release/aquatic http -p
./target/release/aquatic ws -p
```

Note that the configuration files differ between protocols.

To adjust the settings, save the output of the relevant previous command to a
file and make your changes. Then run `aquatic` with a "-c" argument pointing to
the file, e.g.:

```sh
./target/release/aquatic udp -c "/path/to/aquatic-udp-config.toml"
./target/release/aquatic http -c "/path/to/aquatic-http-config.toml"
./target/release/aquatic ws -c "/path/to/aquatic-ws-config.toml"
```

The configuration file values you will most likely want to adjust are
`socket_workers` (number of threads reading from and writing to sockets) and
`address` under the `network` section (listening address). This goes for all
three protocols.

Access control by info hash is supported for all protocols:

```toml
[access_list]
mode = 'off' # Change to 'black' (blacklist) or 'white' (whitelist)
path = '' # Path to text file with newline-delimited hex-encoded info hashes
```

Some documentation of the various options might be available under
`src/lib/config.rs` in crates `aquatic_udp`, `aquatic_http`, `aquatic_ws`.

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

### aquatic_http: HTTP BitTorrent tracker

Aims for compatibility with the HTTP BitTorrent protocol, as described
[here](https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol),
including TLS and scrape request support. There are some exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_http` has not been tested as much as `aquatic_udp` but likely works
fine.

#### TLS

To run over TLS, a pkcs12 file (`.pkx`) is needed. It can be generated from
Let's Encrypt certificates as follows, assuming you are in the directory where
they are stored:

```sh
openssl pkcs12 -export -out identity.pfx -inkey privkey.pem -in cert.pem -certfile fullchain.pem
```

Enter a password when prompted. Then move `identity.pfx` somewhere suitable,
and enter the path into the tracker configuration field `tls_pkcs12_path`. Set
the password in the field `tls_pkcs12_password` and set `use_tls` to true.

### aquatic_ws: WebTorrent tracker

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients, including `wss` protocol support (WebSockets over TLS), with some
exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

For information about running over TLS, please refer to the TLS subsection
of the `aquatic_http` section above.

#### Benchmarks

[wt-tracker]: https://github.com/Novage/wt-tracker
[bittorrent-tracker]: https://github.com/webtorrent/bittorrent-tracker

The following benchmark is not very realistic, as it simulates a small number of clients, each sending a large number of requests. Nonetheless, I think that it gives a useful indication of relative performance.

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

## Load testing

There are load test binaries for all protocols. They use a CLI structure
similar to `aquatic` and support generation and loading of configuration files.

To run, first start the tracker that you want to test. Then run the
corresponding load test binary:

```sh
./scripts/run-load-test-udp.sh
./scripts/run-load-test-http.sh
./scripts/run-load-test-ws.sh
```

To fairly compare HTTP performance to opentracker, set keepalive to false in
`aquatic_http` settings.

## Architectural overview

One or more socket workers open sockets, read and parse requests from peers and
send them through channels to request workers. The request workers go through
the requests, update shared internal tracker state as appropriate and generate
responses that are sent back to the socket workers. The responses are then
serialized and sent back to the peers.

This design means little waiting for locks on internal state occurs,
while network work can be efficiently distributed over multiple threads,
making use of SO_REUSEPORT setting.

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
