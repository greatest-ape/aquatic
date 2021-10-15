# TODO

* access lists:
  * test functionality
  * implement for aquatic_ws

* Don't unwrap peer_address
* Consider turning on safety and override flags in mimalloc (mostly for
  simd-json)
* Use Cow for error response messages?

## General
* extract response peers: extract "one extra" to compensate for removal,
  of sender if present in selection? maybe make criterion benchmark,
  optimize

## aquatic_http_load_test
* how handle large number of peers for "popular" torrents in keepalive mode?
  maybe it is ok
* multiple workers combined with closing connections immediately in tracker
  results in very few responses, why?
* why is cpu usage in load test client so much higher than in aquatic_http
  and in opentracker (when closing connections immediately in tracker)
* maybe check performance against other well-known implementations than
  opentracker (which hopefully do keep-alive)
* try creating sockets with different ports (and also local ips if setting
  enabled), then converting them to mio tcp streams? (so that so_reuseport
  can distribute them to different workers)

## aquatic_http
* test torrent transfer with real clients
  * scrape: does it work (serialization etc), and with multiple hashes?
  * 'left' optional in magnet requests? Probably not. Transmission sends huge
    positive number.
* compact=0 should result in error response

## aquatic_ws_load_test
* very small amount of connections means very small number of peers per
  torrents, so tracker handling of large number is not really assessed
* too many responses received with aquatic_ws?
* wt-tracker: why lots of responses with no (new) requests?
* count number of announce requests sent, total number of offers sent,
  and answers sent. compare these with responses received. same for
  scrape requests I suppose.

## aquatic_ws
* panic when unwrapping peer_address after peer closes connection:

```
thread 'socket-01' panicked at 'called `Result::unwrap()` on an `Err` value: Os { code: 22, kind: InvalidInput, message: "Invalid argument" }', aquatic_ws/src/lib/network/connection.rs:28:59
```

* websocket_max_frame_size should be at least something like 64 * 1024,
  maybe put it and message size at 128k just to be sure
* test transfer, specifically ipv6/ipv4 mapping
* is 'key' sent in announce request? if so, maybe handle it like in
  aquatic_http (including ip uniqueness part of peer map key)

## aquatic_udp
* handle errors similarily to aquatic_ws, including errors in socket workers
  and using log crate
* use key from request as part of peer map key like in aquatic_http? need to
  check protocol specification

# Not important

## General
* mio oneshot setting: could it be beneficial? I think not, I think it is
  only useful when there are multiple references to a fd, which shouldn't
  be the case?
* peer extractor: extract one extra, remove peer if same as sender, otherwise
  just remove one?
* ctrl-c handlers
* have main thread wait for any of the threads returning, quit
  if that is the since since it means a panic occured

## aquatic_http
* array buffer for EstablishedConnection.send_response? there is a lot of
  allocating and deallocating now. Doesn't seem to help performance a lot.
* request parsing:
  * smartstring: maybe use for keys? maybe use less? needs benchmarking
* use fastrand instead of rand? (also for ws and udp then I guess because of
  shared function)
* use smartstring for failure response reason?
* log more info for all log modes (function location etc)? also for aquatic_ws
* Support supportcrypto/requirecrypto keys? Official extension according to
  https://wiki.theory.org/index.php/BitTorrentSpecification#Connection_Obfuscation.
  More info: http://wiki.vuze.com/w/Message_Stream_Encryption. The tricky part
  is finding supportcrypto peers (and even better requirecrypto peers) to send
  back to requirecrypto peers. Doesn't really work according to reference in
  https://en.wikipedia.org/wiki/BitTorrent_protocol_encryption

## aquatic_ws
* use enum as return type for handshake machine
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
* Performance
    * mialloc good?

## aquatic_udp_protocol
* Tests with good known byte sequences (requests and responses)
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

## aquatic_http
* request from path:
  * deserialize 20 bytes: possibly rewrite (just check length of underlying
    bytes == 20 and then copy them), also maybe remove String from map for
    these cases too. doesn't really improve performance
  * crazy http parsing: check for newline with memchr, take slice until
    there. then iter over space newlines/just take relevant data. Not faster
    than httparse and a lot worse

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
