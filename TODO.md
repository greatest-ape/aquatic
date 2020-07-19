# TODO

## General

* use ipv4-mapped address functions, but I should check that they really
  work as they really work as they should. All announces over ipv4 should
  go to ipv4 map, all over ipv6 to ipv6 map
* avx-512 should be avoided, maybe this should be mentioned in README
  and maybe run scripts should be adjusted

## aquatic_http
* request parsing:
  * tests of main function and the various helper functions
  * hashmap needs ddos protecting hash function, or keys could be checked
    against list before insertion
  * deserialize 20 bytes: possibly rewrite (just check length of underlying
    bytes == 20 and then copy them), also maybe remove String from map for
    these cases too
* test torrent transfer with real clients
  * test tls
  * current serialized byte strings valid
  * scrape: does it work (serialization etc), and with multiple hashes?
* compact=0 should result in error response
* tests of response serialization (against data known to be good would be nice)
* Connection.send_response: handle case when all bytes are not written: can
  write actually block here? And what action should be taken then?

### less important
* use fastrand instead of rand? (also for ws and udp then I guess because of
  shared function)
* use smartstring crate for announce request key and failure response reason?
* request parsing in protocol module instead of in network, maybe from byte
  buffer? Not obvious what error return type to use then (anyhow maybe?)
* log more info for all log modes (function location etc)? also for aquatic_ws
* move stuff to common crate with ws?
* serialization for future load tester: write custom non-serde serializer
  for url encode support maybe, or possibly existing crates handle byte
  serialization well (then there would still be the question of multiple
  values per key for scrape requests)
* 'left' optional in magnet requests? Probably not. Transmission sends huge
  positive number.
* handle_connection_read_event: this is an ugly function, but I don't know
  how to improve it
* Support supportcrypto/requirecrypto keys? Official extension according to
  https://wiki.theory.org/index.php/BitTorrentSpecification#Connection_Obfuscation.
  More info: http://wiki.vuze.com/w/Message_Stream_Encryption. The tricky part
  is finding supportcrypto peers (and even better requirecrypto peers) to send
  back to requirecrypto peers. Doesn't really work according to reference in
  https://en.wikipedia.org/wiki/BitTorrent_protocol_encryption

### don't do

* request from path: only urldecode peer_id and info_hash: doesn't really
  improve performance

## aquatic_ws
* is 'key' sent in announce request? if so, maybe handle it like in
  aquatic_http (including ip uniqueness part of peer map key)
* established connections do not get valid_until updated, I think?
* tests
* use enum as return type for handshake machine
* ipv4 and ipv6 state split: think about this more..

## aquatic_udp
* mio: set oneshot for epoll and kqueue? otherwise, stop reregistering?
* handle errors similarily to aquatic_ws, including errors in socket workers
* Handle Ipv4 and Ipv6 peers. Probably split torrent state. Ipv4 peers
  can't make use of Ipv6 ones. Ipv6 ones may or may note be able to make
  use of Ipv4 ones, I have to check.
* More tests?

## aquatic_udp_protocol
* Tests with good known byte sequences (requests and responses)

# Not important

## aquatic_ws
* copyless for vec pushes in request handler, instead of stack and then heap?
* config
  * send/recv buffer size?
  * tcp backlog?
  * some config.network fields are actually used in handler. maybe they should
    be checked while parsing? not completely clear
* "close connection" message from handler on peer_id and socket_addr mismatch?
  Probably not really necessary. If it is an honest mistake, peer will just
  keep announcing and after a few minutes, the peer in the map will be cleaned
  out and everything will start working
* stack-allocated vectors for announce request offers and scrape request info
  hashes?
* write new version of extract_response_peers which checks for equality with
  peer sending request? It could return an arrayvec or smallvec by the way
  (but then the size needs to be adjusted together with the corresponding
  config var, or the config var needs to be removed)

## aquatic_udp

* Does it really make sense to include peer address in peer map key? I have
  to think about why I included it in the first place.
* if socket workers panic while binding, don't sit around and wait for them
  in privdrop function. Maybe wait some maximum amount of time?
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

## aquatic_udp_protocol
* Avoid heap allocation in general if it can be avoided?
    * request from bytes for scrape: use arrayvec with some max size for
      torrents? With Vec, allocation takes quite a bit of CPU time
    * Optimize bytes to scrape request: Vec::with_capacity or other solution (SmallVec?)
* Don't do endian conversion where unnecessary, such as for connection id and
  transaction id?

## aquatic_cli_helpers

* Include config field comments in exported toml (likely quite a bit of work)

# Don't do

## General - profile-guided optimization

Doesn't seem to improve performance, possibly because I only got it to compile
with thin LTO which could have impacted performance. Running non-pgo version
without AVX-512 seems to be the fastest, although the presence of a ctrl-c handler
(meaning the addition of a thread) might have worsed performance in pgo version
(unlikely).

Benchmarks of aquatic_udp with and without PGO. On hetzer 16x vCPU. 8 workers
just like best results in last benchmark, multiple client ips=true:

### target-cpu=native (probably with avx512 since such features are listed in /proc/cpuinfo), all with thin lto
*   With PGO on aquatic_udp: 370k, without 363k responses per second
*   With PGO on both aquatic_udp and aquatic_udp_load_test: 368k

### with target-cpu=skylake, all with thin lto
*   with pgo on aquatic_udp: 400k
*   with no pgo: 394k

### checkout master (no pgo, no thin lto, no ctrlc handler)

* target-cpu=native: 394k
* target-cpu=skylake: 439k
* no target-cpu set: 388k

## aquatic_http / aquatic_ws
* Shared state for HTTP with and without TLS. Peers who announce over TLS
  should be able to expect that someone snooping on the connection can't
  connect them to a info hash. If someone receives their IP in a response
  while announcing without TLS, this expectation would be broken.

## aquatic_udp

* Other HashMap hashers (such as SeaHash): seemingly not worthwhile, see
  `https://github.com/tkaitchuck/aHash`
* `sendmmsg`: can't send to multiple socket addresses, so doesn't help
* Config behind Arc in state: it is likely better to be able to pass it around
  without state
* Responses: make vectors iterator references so we dont have run .collect().
  Doesn't work since it means conversion to bytes must be done while holding
  readable reference to entry in torrent map, hurting concurrency.

## aquatic_udp_protocol

* Use `bytes` crate: seems to worsen performance somewhat
* Zerocopy (https://docs.rs/zerocopy/0.3.0/zerocopy/index.html) for requests
  and responses? Doesn't work on Vec etc
* New array buffer each time in response_to_bytes: doesn't help performance
