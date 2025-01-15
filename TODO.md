# TODO

## High priority

* Change network address handling to accept separate IPv4 and IPv6
  addresses. Open a socket for each one, setting ipv6_only flag on
  the IPv6 one (unless user opts out).
* update zerocopy version (will likely require minor rewrite)

* udp (uring)
  * run tests under valgrind
    * hangs for integration tests, possibly related to https://bugs.kde.org/show_bug.cgi?id=463859
  * run tests with AddressSanitizer
    * `RUSTFLAGS=-Zsanitizer=address cargo +nightly test -Zbuild-std --target x86_64-unknown-linux-gnu --verbose --profile "test-fast" -p aquatic_udp --features "io-uring"`
      * build fails with `undefined reference to __asan_init`, currently unclear why

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
