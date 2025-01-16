#!/bin/bash
# 
# Test that file transfers work over all protocols.
#
# IPv6 is unfortunately disabled by default in Docker
# (see sysctl net.ipv6.conf.lo.disable_ipv6)

set -e

# Install programs and build dependencies

if command -v sudo; then
    SUDO="sudo "
else
    SUDO=""
fi

ulimit -a

$SUDO apt-get update
$SUDO apt-get install -y cmake libssl-dev screen rtorrent mktorrent ssl-cert ca-certificates curl golang libhwloc-dev

git clone https://github.com/anacrolix/torrent.git gotorrent
cd gotorrent
# Use commit known to work
git checkout 16176b762e4a840fc5dfe3b1dfd2d6fa853b68d7
go build -o $HOME/gotorrent ./cmd/torrent
cd ..
file $HOME/gotorrent

# Go to repository directory

if [[ -z "${GITHUB_WORKSPACE}" ]]; then
    exit 1
else
    cd "$GITHUB_WORKSPACE"
fi

# Setup bogus TLS certificate

$SUDO echo "127.0.0.1    example.com" >> /etc/hosts

openssl genrsa -out ca.key 2048
openssl req -new -x509 -days 365 -key ca.key -subj "/C=CN/ST=GD/L=SZ/O=Acme, Inc./CN=Acme Root CA" -out ca.crt
openssl req -newkey rsa:2048 -nodes -keyout server.key -subj "/C=CN/ST=GD/L=SZ/O=Acme, Inc./CN=*.example.com" -out server.csr
openssl x509 -req -extfile <(printf "subjectAltName=DNS:example.com,DNS:www.example.com") -days 365 -in server.csr -CA ca.crt -CAkey ca.key -CAcreateserial -out server.crt
openssl pkcs8 -in server.key -topk8 -nocrypt -out key.pk8

$SUDO cp ca.crt /usr/local/share/ca-certificates/snakeoil-ca.crt
$SUDO cp server.crt /usr/local/share/ca-certificates/snakeoil-server.crt
$SUDO update-ca-certificates

# Build and start tracker

cargo build --bin aquatic

# UDP
echo "
log_level = 'debug'

[network]
address_ipv4 = '127.0.0.1:3000'" > udp.toml
./target/debug/aquatic udp -c udp.toml > "$HOME/udp.log" 2>&1 &

# HTTP
echo "log_level = 'debug'

[network]
address_ipv4 = '127.0.0.1:3004'" > http.toml
./target/debug/aquatic http -c http.toml > "$HOME/http.log" 2>&1 &

# HTTP with TLS
echo "log_level = 'debug'

[network]
address_ipv4 = '127.0.0.1:3001'
enable_tls = true
tls_certificate_path = './server.crt'
tls_private_key_path = './key.pk8'
" > tls.toml
./target/debug/aquatic http -c tls.toml > "$HOME/tls.log" 2>&1 &

# WebTorrent
echo "log_level = 'debug'

[network]
address = '127.0.0.1:3003'
enable_http_health_checks = true
" > ws.toml
./target/debug/aquatic ws -c ws.toml > "$HOME/ws.log" 2>&1 &

# WebTorrent with TLS
echo "log_level = 'debug'

[network]
address = '127.0.0.1:3002'
enable_tls = true
tls_certificate_path = './server.crt'
tls_private_key_path = './key.pk8'
" > ws-tls.toml
./target/debug/aquatic ws -c ws-tls.toml > "$HOME/ws-tls.log" 2>&1 &

# Setup directories

cd "$HOME"

mkdir seed
mkdir leech
mkdir torrents

# Create torrents

echo "udp-test-ipv4" > seed/udp-test-ipv4
echo "http-test-ipv4" > seed/http-test-ipv4
echo "tls-test-ipv4" > seed/tls-test-ipv4
echo "ws-test-ipv4" > seed/ws-test-ipv4
echo "ws-tls-test-ipv4" > seed/ws-tls-test-ipv4

mktorrent -p -o "torrents/udp-ipv4.torrent" -a "udp://127.0.0.1:3000" "seed/udp-test-ipv4"
mktorrent -p -o "torrents/http-ipv4.torrent" -a "http://127.0.0.1:3004/announce" "seed/http-test-ipv4"
mktorrent -p -o "torrents/tls-ipv4.torrent" -a "https://example.com:3001/announce" "seed/tls-test-ipv4"
mktorrent -p -o "torrents/ws-ipv4.torrent" -a "ws://example.com:3003" "seed/ws-test-ipv4"
mktorrent -p -o "torrents/ws-tls-ipv4.torrent" -a "wss://example.com:3002" "seed/ws-tls-test-ipv4"

cp -r torrents torrents-seed
cp -r torrents torrents-leech

# Setup ws-tls seeding client

echo "Starting seeding ws-tls (wss) client"
cd seed
GOPPROF=http $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --seed ../torrents/ws-tls-ipv4.torrent > "$HOME/ws-tls-seed.log" 2>&1 &
cd ..

# Setup ws seeding client

echo "Starting seeding ws client"
cd seed
GOPPROF=http $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --seed ../torrents/ws-ipv4.torrent > "$HOME/ws-seed.log" 2>&1 &
cd ..

# Start seeding rtorrent client

echo "directory.default.set = $HOME/seed
schedule2 = watch_directory,5,5,load.start=$HOME/torrents-seed/*.torrent" > ~/.rtorrent.rc

echo "Starting seeding rtorrent client"
screen -dmS rtorrent-seed rtorrent

# Give seeding clients time to load config files etc

echo "Waiting for a while"
sleep 30

# Start leeching clients

echo "directory.default.set = $HOME/leech
schedule2 = watch_directory,5,5,load.start=$HOME/torrents-leech/*.torrent" > ~/.rtorrent.rc

echo "Starting leeching client.."
screen -dmS rtorrent-leech rtorrent

echo "Starting leeching ws-tls (wss) client"
cd leech
GOPPROF=http $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --addr ":43000" ../torrents/ws-tls-ipv4.torrent > "$HOME/ws-tls-leech.log" 2>&1 &
cd ..

echo "Starting leeching ws client"
cd leech
GOPPROF=http $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --addr ":43001" ../torrents/ws-ipv4.torrent > "$HOME/ws-leech.log" 2>&1 &
cd ..

# Check for completion

HTTP_IPv4="Failed"
TLS_IPv4="Failed"
UDP_IPv4="Failed"
WS_TLS_IPv4="Failed"
WS_IPv4="Failed"

i="0"

echo "Watching for finished files.."

while [ $i -lt 60 ]
do
    if test -f "leech/http-test-ipv4"; then
        if grep -q "http-test-ipv4" "leech/http-test-ipv4"; then
            if [ "$HTTP_IPv4" != "Ok" ]; then
                HTTP_IPv4="Ok"
                echo "HTTP_IPv4 is Ok"
            fi
        fi
    fi
    if test -f "leech/tls-test-ipv4"; then
        if grep -q "tls-test-ipv4" "leech/tls-test-ipv4"; then
            if [ "$TLS_IPv4" != "Ok" ]; then
                TLS_IPv4="Ok"
                echo "TLS_IPv4 is Ok"
            fi
        fi
    fi
    if test -f "leech/udp-test-ipv4"; then
        if grep -q "udp-test-ipv4" "leech/udp-test-ipv4"; then
            if [ "$UDP_IPv4" != "Ok" ]; then
                UDP_IPv4="Ok"
                echo "UDP_IPv4 is Ok"
            fi
        fi
    fi
    if test -f "leech/ws-tls-test-ipv4"; then
        if grep -q "ws-tls-test-ipv4" "leech/ws-tls-test-ipv4"; then
            if [ "$WS_TLS_IPv4" != "Ok" ]; then
                WS_TLS_IPv4="Ok"
                echo "WS_TLS_IPv4 is Ok"
            fi
        fi
    fi
    if test -f "leech/ws-test-ipv4"; then
        if grep -q "ws-test-ipv4" "leech/ws-test-ipv4"; then
            if [ "$WS_IPv4" != "Ok" ]; then
                WS_IPv4="Ok"
                echo "WS_IPv4 is Ok"
            fi
        fi
    fi

    if [ "$HTTP_IPv4" = "Ok" ] && [ "$TLS_IPv4" = "Ok" ] && [ "$UDP_IPv4" = "Ok" ] && [ "$WS_TLS_IPv4" = "Ok" ] && [ "$WS_IPv4" = "Ok" ]; then
        break
    fi

    sleep 1

    i=$[$i+1]
done

echo "Waited for $i seconds"

echo "::set-output name=http_ipv4::$HTTP_IPv4"
echo "::set-output name=http_tls_ipv4::$TLS_IPv4"
echo "::set-output name=udp_ipv4::$UDP_IPv4"
echo "::set-output name=ws_tls_ipv4::$WS_TLS_IPv4"
echo "::set-output name=ws_ipv4::$WS_IPv4"

echo ""
echo "# --- HTTP log --- #"
cat "http.log"

sleep 1

echo ""
echo "# --- HTTP over TLS log --- #"
cat "tls.log"

sleep 1

echo ""
echo "# --- UDP log --- #"
cat "udp.log"

sleep 1

echo ""
echo "# --- WS over TLS tracker log --- #"
cat "ws-tls.log"

sleep 1

echo ""
echo "# --- WS tracker log --- #"
cat "ws.log"

sleep 1

echo ""
echo "# --- WS over TLS seed log --- #"
cat "ws-tls-seed.log"

sleep 1

echo ""
echo "# --- WS over TLS leech log --- #"
cat "ws-tls-leech.log"

sleep 1

echo ""
echo "# --- WS seed log --- #"
cat "ws-seed.log"

sleep 1

echo ""
echo "# --- WS leech log --- #"
cat "ws-leech.log"

sleep 1

echo ""
echo "# --- Test results --- #"
echo "HTTP:             $HTTP_IPv4"
echo "HTTP (TLS):       $TLS_IPv4"
echo "UDP:              $UDP_IPv4"
echo "WebTorrent (TLS): $WS_TLS_IPv4"
echo "WebTorrent:       $WS_IPv4"

if [ "$HTTP_IPv4" != "Ok" ] || [ "$TLS_IPv4" != "Ok" ] || [ "$UDP_IPv4" != "Ok" ] || [ "$WS_TLS_IPv4" != "Ok" ] || [ "$WS_IPv4" != "Ok" ]; then
    exit 1
fi
