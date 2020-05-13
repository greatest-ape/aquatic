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

- Install rust with rustup (stable rust is recommended). 
- Install cmake with your package manager.
- Clone the git repository and refer to the next section.

## Run

The command line interfaces for `aquatic_udp` and `aquatic_ws` are identical.
To run the respective tracker, just run its binary. You can also run any of
the helper scripts, which will compile the binary for you and pass on any
command line parameters.

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
make your changes. The values you will most likely want to adjust are
`socket_workers` (number of threads reading from and writing to sockets) and
`address` under the `network` section (listening address). This goes for both
`aquatic_udp` and `aquatic_ws`. Some documentation of the various options is
available in source code files `aquatic_udp/src/lib/config.rs` and
`aquatic_ws/src/lib/config.rs`.

Then run the binaries with a "-c" argument pointing to the file, e.g.:

```sh
./scripts/run-aquatic-udp.sh -c "/path/to/aquatic-udp-config.toml"
```

```sh
./scripts/run-aquatic-ws.sh -c "/path/to/aquatic-ws-config.toml"
```

## aquatic_udp: UDP BitTorrent tracker

Aims to implements the
[UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html),
except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

Default configuration:

```toml
socket_workers = 1
request_workers = 1

[network]
address = '127.0.0.1:3000'
max_scrape_torrents = 255
max_response_peers = 255
peer_announce_interval = 900
socket_recv_buffer_size = 524288
poll_event_capacity = 4096

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

### Benchmarks

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

## aquatic_ws: WebTorrent tracker

Experimental [WebTorrent](https://github.com/webtorrent) tracker, not yet
recommended for production use.

Default configuration:

```toml
socket_workers = 1

[network]
address = '127.0.0.1:3000'
use_tls = false
tls_pkcs12_path = ''
tls_pkcs12_password = ''
max_scrape_torrents = 255
max_offers = 10
peer_announce_interval = 120
poll_event_capacity = 4096
poll_timeout_milliseconds = 50

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

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)