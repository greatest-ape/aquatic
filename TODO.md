# TODO

## High priority

* udp
  * consider ways of avoiding response peer allocations
  * make ConnectionValidator faster by avoiding calling time functions so often

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

## Low priority

* aquatic_udp
  * udp uring
    * miri
    * thiserror?
    * CI
    * uring load test?

* Performance hyperoptimization (receive interrupts on correct core)
  * If there is no network card RSS support, do eBPF XDP CpuMap redirect based on packet info, to
    cpus where socket workers run. Support is work in progress in the larger Rust eBPF
    implementations, but exists in rebpf
  * Pin socket workers
  * Set SO_INCOMING_CPU (which should be fixed in very recent Linux?) to currently pinned thread
  * How does this relate to (currently unused) so_attach_reuseport_cbpf code?

# Not important

* aquatic_http:
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")
  * test torrent transfer with real clients
    * scrape: does it work (serialization etc), and with multiple hashes?
    * 'left' optional in magnet requests? Probably not. Transmission sends huge
      positive number.
