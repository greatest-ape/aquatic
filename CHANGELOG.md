# Changelog

## Unreleased

### General

#### Added

* Add cli flag for printing parsed config
* Add `aquatic_http_private`, an experiment for integrating with private trackers

#### Changed

* Rename request workers to swarm workers
* Switch to thin LTO for faster compile times
* Use proper workspace path declarations instead of workspace patch section
* Use [Rust 1.64 workspace inheritance](https://blog.rust-lang.org/2022/09/22/Rust-1.64.0.html)
* Reduce space taken by ValidUntil struct from 128 to 32 bits, reducing memory
  consumption for each stored peer by same amount
* Use regular indexmap instead of amortized-indexmap. This goes for torrent,
  peer and pending scrape response maps 
* Improve privilege dropping
* Quit whole program if any thread panics
* Update dependencies

#### Fixed

* Forbid unrecognized keys when parsing config files
* Stop including invalid avx512 key in `./scripts/env-native-cpu-without-avx-512`

### aquatic_udp

#### Added

* Add optional response resend buffer for use on on operating systems that
  don't buffer outgoing UDP traffic
* Add optional extended statistics (peers per torrent histogram)
* Add Dockerfile to make it easier to get started

#### Changed

* Replace ConnectionMap with BLAKE3-based connection validator, greatly
  decreasing memory consumtion
* Don't return any response peers if announce event is stopped
* Ignore requests with source port value of zero

#### Fixed

* When calculating bandwidth statistics, include size of protocol headers

### aquatic_http

#### Added

* Support exposing a Prometheus endpoint for metrics

#### Changed

* Don't return any response peers if announce event is stopped

### aquatic_http_protocol

#### Fixed

* Explicity check for /scrape path
* Return NeedMoreData until headers are fully parsed
* Fix issues with ScrapeRequest::write and AnnounceRequest::write
* Expose write and parse methods for subtypes

### aquatic_http_load_test

#### Changed

* Exclusively use TLS 1.3

### aquatic_ws

#### Added

* Add HTTP health check route when running without TLS
* Support exposing a Prometheus endpoint for metrics

#### Changed

* Make TLS optional
* Support reverse proxies
* Reduce size of various structs

#### Fixed

* Remove peer from swarms immediately when connection is closed
* Allow peers to use multiple peer IDs, as long as they only use one per info hash

### aquatic_ws_load_test

#### Changed

* Exclusively use TLS 1.3
