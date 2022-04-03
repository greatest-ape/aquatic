# TODO

## High priority

* aquatic_http_private
  * Test that responses are actually valid UTF-8
  * stored procedure
    * handle/document lack of NOT NULL
    * event can be null
    * test ip format
  * site will likely want num_seeders and num_leechers for all torrents..

* aquatic_http_protocol
  * don't require compact=1?

## Medium priority

* Use thin LTO?
* Add release-debug profile?
* quit whole program if any thread panics
* config: fail on unrecognized keys?
* Run cargo-deny in CI

* aquatic_ws
  * remove peer from all torrent maps when connection is closed
  * RES memory still high after traffic stops, even if torrent maps and connection slabs go down to 0 len and capacity
    * replacing indexmap_amortized / simd_json with equivalents doesn't help
  * SinkExt::send maybe doesn't wake up properly?
    * related to https://github.com/sdroege/async-tungstenite/blob/master/src/compat.rs#L18 ?

* extract_response_peers
  * don't assume requesting peer is in list?

## Low priority

* config
  * add flag to print parsed config when starting

* aquatic_udp
  * what poll event capacity is actually needed?
  * stagger connection cleaning intervals?
  * load test
      * move additional request sending to for each received response, maybe
        with probability 0.2

* aquatic_ws
  * large amount of temporary allocations in serialize_20_bytes, pretty many in deserialize_20_bytes

* so_attach_reuseport_cbpf
* extract response peers: extract "one extra" to compensate for removal,
  of sender if present in selection?

# Not important

* aquatic_http:
  * optimize?
    * get_peer_addr only once (takes 1.2% of runtime)
    * queue response: allocating takes 2.8% of runtime
  * consider better error type for request parsing, so that better error
    messages can be sent back (e.g., "full scrapes are not supported")
  * test torrent transfer with real clients
    * scrape: does it work (serialization etc), and with multiple hashes?
    * 'left' optional in magnet requests? Probably not. Transmission sends huge
      positive number.

* aquatic_ws
  * mio
    * shard torrent state. this could decrease dropped messages too, since
      request handlers won't send large batches of them
    * connection cleaning interval
    * use access list cache
    * use write event interest for handshakes too
    * deregistering before closing is required by mio, but it hurts performance
      * blocked on https://github.com/snapview/tungstenite-rs/issues/51
    * connection closing: send tls close message etc?
  * write new version of extract_response_peers which checks for equality with
    peer sending request???

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
  and responses. Doesn't improve performance
