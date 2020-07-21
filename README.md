# aquatic

Blazingly fast, multi-threaded BitTorrent tracker written in Rust.

Consists of separate executables:
  * `aquatic_udp`: UDP BitTorrent tracker with double the throughput of
    opentracker (see benchmarks below)
  * `aquatic_ws`: WebTorrent tracker (experimental)
  * `aquatic_http`: HTTP BitTorrent tracker (experimental)

These are described in detail below, after the general information.

## Copyright and license

Copyright (c) 2020 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Technical overview

One or more socket workers open sockets, read and parse requests from peers and
send them through channels to request workers. They in turn go through the
requests, update internal state as appropriate and generate responses, which
are sent back to the socket workers, which serialize them and send them to
peers. This design means little waiting for locks on internal state occurs,
while network work can be efficiently distributed over multiple threads.

## Installation prerequisites

- Install Rust with [rustup](https://rustup.rs/) (stable is recommended)
- Install cmake with your package manager (e.g., `apt-get install cmake`)
- For `aquatic_ws` and `aquatic_http` on GNU/Linux, also install the OpenSSL
  components necessary for dynamic linking (e.g., `apt-get install libssl-dev`)
- Clone the git repository and refer to the next section.

## Compile and run

The command line interfaces for the tracker executables are identical. To run
the respective tracker, just run its binary. You can also run any of the helper
scripts, which will compile the binary for you and pass on any command line
parameters. (After compilation, the binaries are found in `target/release/`.)

To run with default settings:

```sh
./scripts/run-aquatic-udp.sh
```

```sh
./scripts/run-aquatic-ws.sh
```

```sh
./scripts/run-aquatic-http.sh
```

To print default settings to standard output, pass the "-p" flag to the binary:

```sh
./scripts/run-aquatic-udp.sh -p
```

```sh
./scripts/run-aquatic-ws.sh -p
```

```sh
./scripts/run-aquatic-http.sh -p
```

To adjust the settings, save the output of the previous command to a file and
make your changes. Then run the binaries with a "-c" argument pointing to the
file, e.g.:

```sh
./scripts/run-aquatic-udp.sh -c "/path/to/aquatic-udp-config.toml"
```

```sh
./scripts/run-aquatic-ws.sh -c "/path/to/aquatic-ws-config.toml"
```

```sh
./scripts/run-aquatic-http.sh -c "/path/to/aquatic-http-config.toml"
```

The configuration file values you will most likely want to adjust are
`socket_workers` (number of threads reading from and writing to sockets) and
`address` under the `network` section (listening address). This goes for all
three executables.

Some documentation of the various options might be available in source code
files `src/lib/config.rs` in the respective tracker crates.

## Details on protocol-specific executables

### aquatic_udp: UDP BitTorrent tracker

Aims to implements the
[UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html),
except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

#### Benchmarks

Performance was compared to
[opentracker](http://erdgeist.org/arts/software/opentracker/) using
`aquatic_udp_load_test`.

Server responses per second, best result in bold:

| workers |   aquatic   | opentracker |
| ------- | ----------- | ----------- |
|    1    |     n/a     |   __177k__  |
|    2    |  __168k__   |      98k    |
|    3    |  __187k__   |     118k    |
|    4    |  __216k__   |     127k    |
|    6    |  __309k__   |     109k    |
|    8    |  __408k__   |      96k    |

See `documents/aquatic-load-test-2020-04-19.pdf` for details on benchmark, and
end of README for more information about load testing.

### aquatic_ws: WebTorrent tracker

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients, including `wss` protocol support (WebSockets over TLS), with some
exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_ws` is not as well tested as `aquatic_udp`, but has been
successfully used as the tracker for a file transfer between two webtorrent
peers.

#### TLS

To run over TLS (wss protocol), a pkcs12 file (`.pkx`) is needed. It can be
generated from Let's Encrypt certificates as follows, assuming you are in the
directory where they are stored:

```sh
openssl pkcs12 -export -out identity.pfx -inkey privkey.pem -in cert.pem -certfile fullchain.pem
```

Enter a password when prompted. Then move `identity.pfx` somewhere suitable,
and enter the path into the tracker configuration field `tls_pkcs12_path`. Set
the password in the field `tls_pkcs12_password` and set `use_tls` to true.

### aquatic_http: HTTP BitTorrent tracker

Aims for compatibility with the HTTP BitTorrent protocol, as described
[here](https://wiki.theory.org/index.php/BitTorrentSpecification#Tracker_HTTP.2FHTTPS_Protocol),
including TLS and scrape request support. There are some exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_http` is a work in progress and hasn't been tested very much yet.

Please refer to the `aquatic_ws` section for information about setting up TLS.

## Load testing

There are two load test binaries. They use the same CLI structure as the
trackers, including configuration file generation and loading.

To load test `aquatic_udp`, start it and then run:

```sh
./scripts/run-load-test-udp.sh
```

To load test `aquatic_http`, start it and then run:

```sh
./scripts/run-load-test-http.sh
```

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
