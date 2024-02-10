# aquatic: high-performance open BitTorrent tracker

[![CI](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml/badge.svg)](https://github.com/greatest-ape/aquatic/actions/workflows/ci.yml)

High-performance open BitTorrent tracker, consisting
of sub-implementations for different protocols:

[aquatic_udp]: ./crates/udp
[aquatic_http]: ./crates/http
[aquatic_ws]: ./crates/ws

| Name           | Protocol                                  | OS requirements    |
|----------------|-------------------------------------------|--------------------|
| [aquatic_udp]  | BitTorrent over UDP                       | Unix-like          |
| [aquatic_http] | BitTorrent over HTTP, optionally over TLS | Linux 5.8 or later |
| [aquatic_ws]   | WebTorrent, optionally over TLS           | Linux 5.8 or later |

Features at a glance:

- Multithreaded design for handling large amounts of traffic
- All data is stored in-memory (no database needed)
- IPv4 and IPv6 support
- Supports forbidding/allowing info hashes
- Prometheus metrics
- Automated CI testing of full file transfers

Known users:

- [explodie.org public tracker](https://explodie.org/opentracker.html) (`udp://explodie.org:6969`), typically [serving ~100,000 requests per second](https://explodie.org/tracker-stats.html)
- [tracker.webtorrent.dev](https://tracker.webtorrent.dev) (`wss://tracker.webtorrent.dev`)

## Performance of the UDP implementation

![UDP BitTorrent tracker throughput](./documents/aquatic-udp-load-test-2024-02-10.png)

More benchmark details are available [here](./documents/aquatic-udp-load-test-2024-02-10.md).

## Usage

Please refer to the README pages for the respective implementations listed in
the table above.

## Copyright and license

Copyright (c) Joakim Frostegård

Distributed under the terms of the Apache License, Version 2.0. Please refer to
the `LICENSE` file in the repository root directory for details.

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
