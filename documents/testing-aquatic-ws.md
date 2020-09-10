# Working procedure for testing file transfer with aquatic_ws

- On VPS, create identity (using real certificate), run tracker with TLS
- On VPS, create torrent with external url as announce. Edit file and put
  external url not only as announce, but in announce list too.
- On VPS, disallow traffic to other trackers by adding them to /etc/hosts
  or maybe with firewall, since webtorrent-hybrid adds its own trackers
  willy-nilly. To get a list of the tracker urls which are actually used,
  the node application under heading "Seed application" can be used as a
  starting point.
- I opened the listening port in the VPS firewall too (this might not be
  necessary if running both clients on the VPS, see below)
- On VPS, run webtorrent-hybrid download --keep-seeding ./abc.torrent
- On desktop/non-VPS computer, fetch torrent file, run
  webtorrent-hybrid download ./abc.torrent
  I actually got it to work running this client on the VPS too.

## Seed application

```js
// Start webtorrent seeder from data file, create torrent, write it to file,
// output info

var WebTorrent = require('webtorrent-hybrid')
var fs = require('fs')

// WebTorrent seems to use same peer id for different
// clients in some cases (I don't know how)
peerId = "ae61b6f4a5be4ada48333891512db5e90347d736"
announceUrl = 'ws://127.0.0.1:3000'
dataFile = './files-seed/ws-ipv4'
torrentFile = './torrents/ws-ipv4.torrent'

function createSeeder(){
    console.log('creating seeder..')

    var seeder = new WebTorrent({ dht: false, webSeeds: false, peerId: peerId })
    seeder.on('error', function(err) {
        console.log('seeder error: ' + err)
    })      

    var addOpts = {
        announceList: [[announceUrl]],
        announce: [announceUrl],
        private: true 
    }       

    seeder.seed(dataFile, addOpts, function(torrent){
        console.log("seeding")
        // Log torrent info, including actually used trackers
        console.log(torrent) 

        fs.writeFile(torrentFile, torrent.torrentFile, function(err){
            if (err){
                console.log(err)
            }       
        })      

        torrent.on('warning', function(err){
            console.log(err)
        });     

        torrent.on('error', function(err){
            console.log(err)
        });     

        torrent.on('download', function(bytes){
            console.log('downloaded bytes: ' + bytes)
        });     

        torrent.on('upload', function(bytes){
            console.log('uploaded bytes: ' + bytes)
        });     

        torrent.on('wire', function(wire, addr){
            console.log('connected to peer with addr: ' + addr)
        });     

        torrent.on('noPeers', function(announceType){
            console.log('no peers received with announce type: ' + announceType)
        })      

        torrent.on('done', function(){
            console.log('done')
        }); 

    })      
}

createSeeder()
```

## Simplifications to procedure that might work

- using fake certificate and routing certificate url to localhost in
  /etc/hosts, meaning all of this could maybe be run locally/in Docker (but I
  think sdp negotiations tend to fail in that case..)

## Issues with Docker implementation

- webtorrent-hybrid adds its own trackers when opening torrents, even if they
  have been removed from file! The really robust way to get around this would
  be to block all outgoing traffic with e.g. iptables before starting tests,
  but I couldn't get it to work

## Notes on testing locally

Official tracker does not successfully handle file transfer on localhost
on my machine between two instances of the official client (webtorrent-hybrid),
probably due to sdp negotiation issues. This was with plain `ws` protocol.

## Possibly useful collection of commands

```sh
npm install -g webtorrent-hybrid
npm install -g bittorrent-tracker # Reference tracker

bittorrent-tracker --ws -p 3000 & # Reference tracker

mkdir files-seed files-leech torrents

webtorrent create files-seed/ws-ipv4 --announce "wss://127.0.0.1:3000" > torrents/ws-ipv4.torrent

cd files-seed
webtorrent-hybrid seed torrents/ws-ipv4.torrent --keep-seeding -q &

cd ../files-leech
webtorrent-hybrid download torrents/ws-ipv4.torrent -q &
```