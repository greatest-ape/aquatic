# TODO

## aquatic_ws
* check protocol todo's etc
* network
  * open socket with so_reuseport and nonblocking
* test
* torrent state cleaning
* config

## aquatic
* mio: set oneshot for epoll and kqueue? otherwise, stop reregistering?
* Handle Ipv4 and Ipv6 peers. Probably split torrent state. Ipv4 peers
  can't make use of Ipv6 ones. Ipv6 ones may or may note be able to make
  use of Ipv4 ones, I have to check.
* More tests?
* chroot / drop privileges? Crates: privdrop, daemonize

## bittorrent_udp
* Tests with good known byte sequences (requests and responses)

# Not important

## aquatic

* No overflow on instant + duration arithmetic now, hopefully? Possibly,
  checked_add should be used.
* extract_response_peers
    * Cleaner code
    * Stack-allocated vector?
* Use log crate for errors, including logging thread names? I could probably
  use code from old rs_news project for that.
* Performance
    * cpu-target=native good?
    * mialloc good?
    * Try using flume (MPSC) or multiqueue2 (MPMC) instead of crossbeam channel
    * Use less bytes from PeerId for hashing? (If yes, only save half of them
      or so in PeerMapKey). Might improve performance, but probably not worth
      it.

## bittorrent_udp
* Avoid heap allocation in general if it can be avoided?
    * request from bytes for scrape: use arrayvec with some max size for
      torrents? With Vec, allocation takes quite a bit of CPU time
    * Optimize bytes to scrape request: Vec::with_capacity or other solution (SmallVec?)
* Don't do endian conversion where unnecessary, such as for connection id and
  transaction id?

## cli_helpers

* Include config field comments in exported toml (likely quite a bit of work)

# Don't do

## aquatic

* Other HashMap hashers (such as SeaHash): seemingly not worthwhile (might be
  with AVX though)
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help
* Config behind Arc in state: it is likely better to be able to pass it around
  without state
* Responses: make vecors iteretor references so we dont have run .collect().
  Doesn't work since it means conversion to bytes must be done while holding
  readable reference to entry in torrent map, hurting concurrency.

## bittorrent_udp

* Use `bytes` crate for bittorrent_udp: seems to worsen performance somewhat
* Zerocopy (https://docs.rs/zerocopy/0.3.0/zerocopy/index.html) for requests
  and responses? Doesn't work on Vec etc
* New array buffer each time in response_to_bytes: doesn't help performance
