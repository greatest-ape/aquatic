# aquatic

Blazingly fast, multi-threaded BitTorrent tracker written in Rust.

Consists of separate executables:
  * `aquatic_udp`: UDP BitTorrent tracker
  * `aquatic_ws`: WebTorrent tracker (experimental)

These are described in detail below, after the general information.

## Copyright and license

Copyright (c) 2020 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Installation prerequisites

- Install Rust with [rustup](https://rustup.rs/) (stable is recommended)
- Install cmake with your package manager (e.g., `apt-get install cmake`)
- For `aquatic_ws` on GNU/Linux, also install the OpenSSL components necessary
  for dynamic linking (e.g., `apt-get install libssl-dev`)
- Clone the git repository and refer to the next section.

## Compile and run

The command line interfaces for `aquatic_udp` and `aquatic_ws` are identical.
To run the respective tracker, just run its binary. You can also run any of
the helper scripts, which will compile the binary for you and pass on any
command line parameters. (After compilation, the binaries are found in
directory `target/release/`.)

To run with default settings:

```sh
./scripts/run-aquatic-udp.sh
```

```sh
./scripts/run-aquatic-ws.sh
```

To print default settings to standard output, pass the "-p" flag to the binary:

```sh
./scripts/run-aquatic-udp.sh -p
```

```sh
./scripts/run-aquatic-ws.sh -p
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

The configuration file values you will most likely want to adjust are
`socket_workers` (number of threads reading from and writing to sockets) and
`address` under the `network` section (listening address). This goes for both
`aquatic_udp` and `aquatic_ws`.

Some documentation of the various options is available in source code files
`aquatic_udp/src/lib/config.rs` and `aquatic_ws/src/lib/config.rs`. The
default settings are also included in in this document, under the section for
each executable below.

## Details on protocol-specific executables

### aquatic_udp: UDP BitTorrent tracker

Aims to implements the
[UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html),
except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

#### Default configuration:

```toml
socket_workers = 1
request_workers = 1

[network]
address = '0.0.0.0:3000'
socket_recv_buffer_size = 524288
poll_event_capacity = 4096

[protocol]
max_scrape_torrents = 255
max_response_peers = 255
peer_announce_interval = 900

[handlers]
max_requests_per_iter = 10000
channel_recv_timeout_microseconds = 200

[statistics]
interval = 5

[cleaning]
interval = 30
max_peer_age = 1200
max_connection_age = 300

[privileges]
drop_privileges = false
chroot_path = '.'
user = 'nobody'
```

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

(See `documents/aquatic-load-test-2020-04-19.pdf` for details.)

### aquatic_ws: WebTorrent tracker

Aims for compatibility with [WebTorrent](https://github.com/webtorrent)
clients, including `wss` protocol support (WebSockets over TLS), with some
exceptions:

  * Doesn't track of the number of torrent downloads (0 is always sent). 
  * Doesn't allow full scrapes, i.e. of all registered info hashes

`aquatic_ws` is not as well tested as `aquatic_udp`, but has been
successfully used as the tracker for a file transfer between two webtorrent
peers.

#### Default configuration

```toml
socket_workers = 1
log_level = 'error'

[network]
address = '0.0.0.0:3000'
ipv6_only = false
use_tls = false
tls_pkcs12_path = ''
tls_pkcs12_password = ''
poll_event_capacity = 4096
poll_timeout_milliseconds = 50
websocket_max_message_size = 65536
websocket_max_frame_size = 16384

[protocol]
max_scrape_torrents = 255
max_offers = 10
peer_announce_interval = 120

[handlers]
max_requests_per_iter = 10000
channel_recv_timeout_microseconds = 200

[cleaning]
interval = 30
max_peer_age = 180
max_connection_age = 180

[privileges]
drop_privileges = false
chroot_path = '.'
user = 'nobody'
```

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

## Architectural overview

One or more socket workers open sockets, read and parse requests from peers and
send them through channels to request workers. They in turn go through the
requests, update internal state as appropriate and generate responses, which
are sent back to the socket workers, which serialize them and send them to
peers. This design means less waiting for locks on internal state has to occur,
while network work can be efficiently distributed over multiple threads.

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)