# aquatic

Blazingly fast, multi-threaded UDP BitTorrent tracker written in Rust.

Aims to implements the [UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html), except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

## Usage

Install rust compiler (stable is fine) and cmake. Then, compile and run aquatic:

```sh
./scripts/run-server.sh
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

## License

Apache 2.0 (see `LICENSE` file)

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
