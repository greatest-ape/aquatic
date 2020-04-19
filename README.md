# aquatic

Fast, multi-threaded UDP BitTorrent tracker written in Rust.

Aims to implements the [UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html), except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

There is currently no support for a info hash black- or whilelist.

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

### Details

- System: Hetzner CCX41 VPS (16 dedicated vCPUs, Skylake)
- OS: Debian 10, kernel 4.19.0-8-amd64
- aquatic commit: 61841fff
- opentracker also has a single threaded event mode. It didn't perform as well (143k responses/second) as blocking single-threaded mode.

Default settings were used, except that:
- load test duration = 120 seconds
- load test multiple_client_ips = true
- load test worker settings were tuned for best results for each trackers
- aquatic request_workers was always = 1, only socket_workers setting was raised. Number in table corresponds to sum of both values

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
