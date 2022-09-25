# Changelog

## Unreleased

### Added

* Add cli flag for printing parsed config
* Add `aquatic_http_private`, an experiment for integrating with private trackers
* _aquatic_udp_: implement optional response resend buffer
* _aquatic_udp_: add optional extended statistics
* _aquatic_udp_: add Dockerfile to make it easier to get started
* _aquatic_ws_: add HTTP health check route when running without TLS

### Changed

* Rename request workers to swarm workers
* Switch to thin LTO
* Use proper workspace path declarations and remove workspace patch section
* Use [Rust 1.64 workspace inheritance](https://blog.rust-lang.org/2022/09/22/Rust-1.64.0.html)
* Reduce space taken by ValidUntil struct from 128 to 32 bits
* Use regular (non-amortized) IndexMap for peer and pending scrape response maps (but not for torrent maps)
* Improve privilege dropping
* Quit whole program if any thread panics
* Update dependencies
* _aquatic_udp_: replace ConnectionMap with BLAKE3-based connection validator
* _aquatic_udp_: ignore requests with source port value of zero
* _aquatic_ws_: reduce size of various structs
* _aquatic_ws_: make TLS optional
* _aquatic_ws_: support reverse proxies

### Fixed

* Fail on unrecognized config keys
* _aquatic_http_protocol_: explicity check for /scrape path
* _aquatic_http_protocol_: return NeedMoreData until headers are fully parsed
* _aquatic_http_protocol_: fix issues with ScrapeRequest::write and AnnounceRequest::write
* _aquatic_http_protocol_: expose write and parse methods for subtypes
* _aquatic_http_load_test_: exclusively use TLS 1.3
* _aquatic_ws_: remove peer from swarms immediately when connection is closed
