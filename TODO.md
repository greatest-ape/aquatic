# TODO

* readme
  * document privilige dropping, cpu pinning

* socket_recv_size and ipv6_only in glommio implementations

* config: fail on unrecognized keys

* access lists:
  * use signals to reload, use arcswap everywhere
  * use arc-swap Cache?
  * add CI tests

* aquatic_udp
  * CI for both implementations
  * glommio
    * consider sending local responses immediately
    * consider adding ConnectedScrapeRequest::Scrape(PendingScrapeRequest)
      containing TransactionId and BTreeMap<usize, InfoHash>, and same for
      response

* aquatic_http:
  * optimize?
    * get_peer_addr only once (takes 1.2% of runtime)
    * queue response: allocating takes 2.8% of runtime
  * clean out connections regularly
    * Rc<RefCell<ValidUntil>> which get set on successful request parsing and
      successful response sending. Clone kept in connection slab which gets cleaned
      periodically (= cancel tasks). Means that task handle will need to be stored in slab.
      Config vars kill_idle_connections: bool, max_idle_connection_time. Remove keepalive.
    * handle panicked/cancelled tasks?
  * load test: use futures-rustls
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")
  * Scrape: should stats with only zeroes be sent back for non-registered info hashes?
    Relevant for mio implementation too.

* aquatic_ws
  * load test cpu pinning
  * test with multiple socket and request workers
  * should it send back error on message parse error, or does that
    just indicate that not enough data has been received yet?

## Less important

* extract response peers: extract "one extra" to compensate for removal,
  of sender if present in selection? maybe make criterion benchmark,
  optimize

## aquatic_http_load_test
* how handle large number of peers for "popular" torrents in keepalive mode?
  maybe it is ok

## aquatic_http
* test torrent transfer with real clients
  * scrape: does it work (serialization etc), and with multiple hashes?
  * 'left' optional in magnet requests? Probably not. Transmission sends huge
    positive number.
* compact=0 should result in error response

## aquatic_ws_load_test
* very small amount of connections means very small number of peers per
  torrents, so tracker handling of large number is not really assessed

## aquatic_udp
* use key from request as part of peer map key like in aquatic_http? need to
  check protocol specification

# Not important

## General
* have main thread wait for any of the threads returning, quit
  if that is the since since it means a panic occured

## aquatic_ws
* write new version of extract_response_peers which checks for equality with
  peer sending request? It could return an arrayvec or smallvec by the way
  (but then the size needs to be adjusted together with the corresponding
  config var, or the config var needs to be removed)

## aquatic_udp
* Does it really make sense to include peer address in peer map key? I have
  to think about why I included it in the first place.

## aquatic_udp_protocol
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
