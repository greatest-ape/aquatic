# aquatic

Fast, multi-threaded UDP BitTorrent tracker written in Rust.

Aims to implements the [UDP BitTorrent protocol](https://libtorrent.org/udp_tracker_protocol.html), except that it:

  * Doesn't care about IP addresses sent in announce requests. The packet
    source IP is always used.
  * Doesn't track of the number of torrent downloads (0 is always sent). 

Supports IPv4 and IPv6.

There is currently no support for a info hash black- or whilelist.

## Trivia

The tracker is called aquatic because it thrives under a torrent of bits ;-)
