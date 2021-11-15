# TODO

* readme
  * document privilege dropping and cpu pinning
  * document access list log output levels and that it exits program
    on start if parsing fails

* config
  * add flag to print parsed config when starting
  * fail on unrecognized keys

* implement socket_recv_size and ipv6_only in glommio implementations

* CI
  * file transfer CI for all implementations
  * test access lists?
  * cargo-deny

* aquatic_udp
  * do ip mapping in socket worker, not in request worker
  * uring
    * ValidUntil periodic update
    * statistics
    * shared config keys such as poll interval
  * mio
    * stagger connection cleaning intervals?
  * glommio
    * consider sending local responses immediately
    * consider adding ConnectedScrapeRequest::Scrape(PendingScrapeRequest)
      containing TransactionId and BTreeMap<usize, InfoHash>, and same for
      response

* aquatic_http:
  * clean out connections regularly
    * Rc<RefCell<ValidUntil>> which get set on successful request parsing and
      successful response sending. Clone kept in connection slab which gets cleaned
      periodically (= cancel tasks). Means that task handle will need to be stored in slab.
      Config vars kill_idle_connections: bool, max_idle_connection_time. Remove keepalive.
    * handle panicked/cancelled tasks?
  * optimize?
    * get_peer_addr only once (takes 1.2% of runtime)
    * queue response: allocating takes 2.8% of runtime
  * use futures-rustls for load test
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")

* aquatic_ws
  * glommio
    * fix memory leak / huge growth

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
  and responses? Doesn't work on Vec etc
