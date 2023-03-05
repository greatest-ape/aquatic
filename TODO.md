# TODO

## High priority

* udp uring
  * test ipv6
  * pin?
  * thiserror
  * statistics
  * profile
  * CI

* ws: wait for crates release of glommio with membarrier fix (PR #558)
* Release new version
* More non-CI integration tests?
* Remove aquatic_http_private?

## Medium priority

* Run cargo-fuzz on protocol crates
* udp: support link to arbitrary homepage as well as embedded tracker URL in statistics page

* Consider storing torrents in separate IndexMaps. The amount should be a power
  of 2 and should be configurable. They could be stored in a Vec and the index
  could be calculated by taking the first N bits of the info hash. Each such map
  would also store when it was last cleaned. There would then be a small
  configurable random chance that when an announce request is being processed,
  the map will be cleaned. When doing the normal cleaning round, recently
  cleaned maps would be skipped.

* quit whole program if any thread panics
  * But it would be nice not to panic in workers, but to return errors instead.
    Once JoinHandle::is_finished is available in stable Rust (#90470), an
    option would be to
     * Save JoinHandles
     * When preparing to quit because of PanicSentinel sending SIGTERM, loop
       through them, extract error and log it

* Run cargo-deny in CI

* udp: add IP blocklist, which would be more flexible than just adding option
  for disallowing requests (claiming to be) from localhost

* stagger cleaning tasks?

* aquatic_ws
  * Add cleaning task for ConnectionHandle.announced_info_hashes?
  * RES memory still high after traffic stops, even if torrent maps and connection slabs go down to 0 len and capacity
    * replacing indexmap_amortized / simd_json with equivalents doesn't help
  * SinkExt::send maybe doesn't wake up properly?
    * related to https://github.com/sdroege/async-tungstenite/blob/master/src/compat.rs#L18 ?

* aquatic_http_private
  * Consider not setting Content-type: text/plain for responses and send vec as default octet stream instead
  * stored procedure
    * test ip format
    * check user token length
  * site will likely want num_seeders and num_leechers for all torrents..

* Performance hyperoptimization (receive interrupts on correct core)
  * If there is no network card RSS support, do eBPF XDP CpuMap redirect based on packet info, to
    cpus where socket workers run. Support is work in progress in the larger Rust eBPF
    implementations, but exists in rebpf
  * Pin socket workers
  * Set SO_INCOMING_CPU (which should be fixed in very recent Linux?) to currently pinned thread
  * How does this relate to (currently unused) so_attach_reuseport_cbpf code?

## Low priority

* aquatic_udp
  * what poll event capacity is actually needed?
  * load test
      * move additional request sending to for each received response, maybe
        with probability 0.2

* aquatic_ws
  * large amount of temporary allocations in serialize_20_bytes, pretty many in deserialize_20_bytes

* extract response peers: extract "one extra" to compensate for removal,
  of sender if present in selection?

# Not important

* aquatic_http:
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")
  * test torrent transfer with real clients
    * scrape: does it work (serialization etc), and with multiple hashes?
    * 'left' optional in magnet requests? Probably not. Transmission sends huge
      positive number.

* aquatic_ws
  * write new version of extract_response_peers which checks for equality with
    peer sending request???

# Don't do

* general: PGO didn't seem to help way back

## aquatic_http
* request from path:
  * deserialize 20 bytes: possibly rewrite (just check length of underlying
    bytes == 20 and then copy them), also maybe remove String from map for
    these cases too. doesn't really improve performance
  * crazy http parsing: check for newline with memchr, take slice until
    there. then iter over space newlines/just take relevant data. Not faster
    than httparse and a lot worse

## aquatic_udp_protocol
* Use `bytes` crate: seems to worsen performance somewhat
* Zerocopy (https://docs.rs/zerocopy/0.3.0/zerocopy/index.html) for requests
  and responses. Doesn't improve performance
