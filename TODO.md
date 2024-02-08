# TODO

## High priority

* aquatic_bencher
  * bench aquatic_udp with io_uring too
  * test with SubsequentOnePerPair
  * include chihaya with higher core counts?

## Medium priority

* stagger cleaning tasks?
* Run cargo-fuzz on protocol crates

* udp 
  * support link to arbitrary homepage as well as embedded tracker URL in statistics page
  * Non-trivial dependency updates
    * toml v0.7
    * syn v2.0

* Run cargo-deny in CI

* aquatic_ws
  * Add cleaning task for ConnectionHandle.announced_info_hashes?

* Performance hyperoptimization (receive interrupts on correct core)
  * If there is no network card RSS support, do eBPF XDP CpuMap redirect based on packet info, to
    cpus where socket workers run. Support is work in progress in the larger Rust eBPF
    implementations, but exists in rebpf
  * Pin socket workers
  * Set SO_INCOMING_CPU (which should be fixed in very recent Linux?) to currently pinned thread
  * How does this relate to (currently unused) so_attach_reuseport_cbpf code?

## Low priority

* aquatic_udp
  * udp uring
    * miri
    * thiserror?
    * CI
    * uring load test?
  * load test
      * move additional request sending to for each received response, maybe
        with probability 0.2

# Not important

* aquatic_http:
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")
  * test torrent transfer with real clients
    * scrape: does it work (serialization etc), and with multiple hashes?
    * 'left' optional in magnet requests? Probably not. Transmission sends huge
      positive number.

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
