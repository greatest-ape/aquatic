# aquatic

Blazingly fast, multi-threaded UDP BitTorrent tracker written in Rust.

Aims to implements the [UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html), except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

## Installation and usage

Install rust  (stable is fine) with rustup, as well as cmake. Then, compile and run aquatic:

```sh
./scripts/run-server.sh
```

To print default configuration as toml, pass the "-p" flag to the binary:

```sh
./scripts/run-server.sh -p
```

Example output:

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

To adjust the settings, save this text to a file and make your changes. The
values you will most likely want to adjust are `socket_workers` (number of
threads reading from and writing to sockets) and `network.address`. (Some
documentation of the various options is available in source code file
`aquatic/src/lib/config.rs`.) Then run aquatic with a "-c" argument pointing
to the file, e.g.:

```sh
./scripts/run-server.sh -c "tmp/aquatic.toml"
```

## Benchmarks

Performance was compared to [opentracker](http://erdgeist.org/arts/software/opentracker/) using `aquatic_load_test`.

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

## Copyright and license

Copyright (c) 2020 Joakim Frostegård

Distributed under Apache 2.0 license (details in `LICENSE` file.)

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
