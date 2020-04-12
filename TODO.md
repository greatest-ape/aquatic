# TODO

## aquatic
* `thread 'main' panicked at 'overflow when subtracting duration from instant', src/libstd/time.rs:374:9`
* Handler: put responses in vector and send them all together after releasing
  lock?
* Use bounded request channel?
* Handle Ipv4 and Ipv6 peers. Probably split state. Ipv4 peers can't make
  use of Ipv6 ones. Ipv6 ones may or may note be able to make use of Ipv4
  ones, have to check.
* More tests

## aquatic_bench
* Fix issues since switch to socket and handler workers, removal of dashmap
* Iterate over whole returned buffer and run e.g. xor on it (.iter().fold())
* Generic bench function since current functions are almost identical
* Show percentile stats for peers per torrent

## bittorrent_udp
* Tests with good known byte sequences

# Not important

* extract_response_peers
    * Cleaner code
    * Stack-allocated vector?
* Benchmarks
    * num_rounds command line argument
    * Send in connect reponse ids to other functions as integration test
    * Save last results, check if difference is significant?
    * ProgressBar: `[{elapsed_precise}]` and eta_precise?
    * Test server over udp socket instead?
* Performance
    * cpu-target=native good?
    * mialloc good?
    * Use less bytes from PeerId for hashing? Would need to implement
      "faulty" PartialEq too (on PeerMapKey, which would be OK)
* bittorrent_udp
    * Avoid heap allocation in general if it can be avoided?
      * request from bytes for scrape: use arrayvec with some max size for
        torrents? With Vec, allocation takes quite a bit of CPU time
      * Optimize bytes to scrape request: Vec::with_capacity or other solution (SmallVec?)

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
