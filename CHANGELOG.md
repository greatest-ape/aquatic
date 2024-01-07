# Changelog

## Unreleased

### General

#### Added

* Add `aquatic_peer_id` crate with peer client information logic
* Add `aquatic_bencher` crate for automated benchmarking of aquatic and other
  BitTorrent trackers

### aquatic_udp

#### Added

* Add support for reporting peer client information

#### Changed

* Index peers by packet source IP and provided port, instead of by peer_id.
  This prevents users from impersonating others and is likely also slightly
  faster for IPv4 peers.
* Remove support for unbounded worker channels
* Add backpressure in socket workers. They will postpone reading from the
  socket if sending a request to a swarm worker failed
* Reuse allocations in swarm response channel
* Remove config key `network.poll_event_capacity`
* Harden ConnectionValidator to make IP spoofing even more costly
* Distribute announce responses from swarm workers over socket workers to
  decrease performance loss due to underutilized threads

### aquatic_http

#### Added

* Reload TLS certificate (and key) on SIGUSR1

#### Changed

* Allow running without TLS
* Allow running behind reverse proxy

#### Fixed

* Fix bug where clean up after closing connections wasn't always done

### aquatic_ws

#### Added

* Add support for reporting peer client information
* Reload TLS certificate (and key) on SIGUSR1
* Keep track of which offers peers have sent and only allow matching answers

#### Changed

* A response is no longer generated when peers announce with AnnounceEvent::Stopped
* Compiling with SIMD extensions enabled is no longer required, due to the
  addition of runtime detection to simd-json
* Only consider announce and scrape responses as signs of connection still
  being alive. Previously, all messages sent to peer were considered.
* Decrease default max_peer_age and max_connection_idle config values

#### Fixed

* Fix bug where clean up after closing connections wasn't always done
* Fix double counting of error responses
* Actually close connections that are too slow to send responses to
* If peers announce with AnnounceEvent::Stopped, allow them to later announce on
  same torrent with different peer_id

## 0.8.0 - 2023-03-17

### General

#### Added

* Support exposing a Prometheus endpoint for metrics
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

* Add experimental io_uring backend with higher throughput
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
