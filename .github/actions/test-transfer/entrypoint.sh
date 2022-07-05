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
git checkout 16176b762e4a840fc5dfe3b1dfd2d6fa853b68d7
go build -o $HOME/gotorrent ./cmd/torrent
cd ..
file $HOME/gotorrent

# Clone repository if necessary, go to repository directory

if [[ -z "${GITHUB_WORKSPACE}" ]]; then
    cd "$HOME"

    git clone https://github.com/greatest-ape/aquatic.git

    cd aquatic
else
    cd "$GITHUB_WORKSPACE"
fi

echo "last aquatic commits:"
git log --oneline -3

# Setup bogus TLS certificate

$SUDO echo "127.0.0.1    example.com" >> /etc/hosts

openssl ecparam -genkey -name prime256v1 -out key.pem
openssl req -new -sha256 -key key.pem -out csr.csr -subj "/C=GB/ST=Test/L=Test/O=Test/OU=Test/CN=example.com"
openssl req -x509 -sha256 -nodes -days 365 -key key.pem -in csr.csr -out cert.crt
openssl pkcs8 -in key.pem -topk8 -nocrypt -out key.pk8 # rustls
openssl pkcs12 -export -passout "pass:p" -out identity.pfx -inkey key.pem -in cert.crt

$SUDO cp cert.crt /usr/local/share/ca-certificates/snakeoil.crt
$SUDO update-ca-certificates

# Build and start tracker

cargo build --bin aquatic

# echo "log_level = 'debug'
# 
# [network]
# address = '127.0.0.1:3000'" > http.toml
# ./target/debug/aquatic http -c http.toml > "$HOME/http.log" 2>&1 &

echo "log_level = 'debug'

[network]
address = '127.0.0.1:3001'
tls_certificate_path = './cert.crt'
tls_private_key_path = './key.pk8'
" > tls.toml
./target/debug/aquatic http -c tls.toml > "$HOME/tls.log" 2>&1 &

echo "[network]
address = '127.0.0.1:3000'" > udp.toml
./target/debug/aquatic udp -c udp.toml > "$HOME/udp.log" 2>&1 &

echo "log_level = 'trace'

[network]
address = '127.0.0.1:3002'
tls_certificate_path = './cert.crt'
tls_private_key_path = './key.pk8'
" > ws.toml
./target/debug/aquatic ws -c ws.toml > "$HOME/wss.log" 2>&1 &

# Setup directories

cd "$HOME"

mkdir seed
mkdir leech
mkdir torrents

# Create torrents

# echo "http-test-ipv4" > seed/http-test-ipv4
echo "tls-test-ipv4" > seed/tls-test-ipv4
echo "udp-test-ipv4" > seed/udp-test-ipv4
echo "wss-test-ipv4" > seed/wss-test-ipv4

# mktorrent -p -o "torrents/http-ipv4.torrent" -a "http://127.0.0.1:3000/announce" "seed/http-test-ipv4"
mktorrent -p -o "torrents/tls-ipv4.torrent" -a "https://example.com:3001/announce" "seed/tls-test-ipv4"
mktorrent -p -o "torrents/udp-ipv4.torrent" -a "udp://127.0.0.1:3000" "seed/udp-test-ipv4"
mktorrent -p -o "torrents/wss-ipv4.torrent" -a "wss://example.com:3002" "seed/wss-test-ipv4"

cp -r torrents torrents-seed
cp -r torrents torrents-leech

# Setup wss seeding client

echo "Starting seeding wss client"
cd seed
GOPPROF=http GODEBUG=x509ignoreCN=0 $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --seed ../torrents/wss-ipv4.torrent > "$HOME/wss-seed.log" 2>&1 &
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

echo "Starting leeching wss client"
cd leech
GOPPROF=http GODEBUG=x509ignoreCN=0 $HOME/gotorrent download --dht=false --tcppeers=false --utppeers=false --pex=false --stats --addr ":43000" ../torrents/wss-ipv4.torrent > "$HOME/wss-leech.log" 2>&1 &
cd ..

# Check for completion

# HTTP_IPv4="Ok"
TLS_IPv4="Failed"
UDP_IPv4="Failed"
WSS_IPv4="Failed"

i="0"

echo "Watching for finished files.."

while [ $i -lt 60 ]
do
    # if test -f "leech/http-test-ipv4"; then
    #     if grep -q "http-test-ipv4" "leech/http-test-ipv4"; then
    #         if [ "$HTTP_IPv4" != "Ok" ]; then
    #             HTTP_IPv4="Ok"
    #             echo "HTTP_IPv4 is Ok"
    #         fi
    #     fi
    # fi
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
    if test -f "leech/wss-test-ipv4"; then
        if grep -q "wss-test-ipv4" "leech/wss-test-ipv4"; then
            if [ "$WSS_IPv4" != "Ok" ]; then
                WSS_IPv4="Ok"
                echo "WSS_IPv4 is Ok"
            fi
        fi
    fi

    # if [ "$HTTP_IPv4" = "Ok" ] && [ "$TLS_IPv4" = "Ok" ] && [ "$UDP_IPv4" = "Ok" ] && [ "$WSS_IPv4" = "Ok" ]; then
    if [ "$TLS_IPv4" = "Ok" ] && [ "$UDP_IPv4" = "Ok" ] && [ "$WSS_IPv4" = "Ok" ]; then
        break
    fi

    sleep 1

    i=$[$i+1]
done

echo "Waited for $i seconds"

# echo "::set-output name=http_ipv4::$HTTP_IPv4"
echo "::set-output name=http_tls_ipv4::$TLS_IPv4"
echo "::set-output name=udp_ipv4::$UDP_IPv4"
echo "::set-output name=wss_ipv4::$WSS_IPv4"

# echo ""
# echo "# --- HTTP log --- #"
# cat "http.log"

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
echo "# --- WSS tracker log --- #"
cat "wss.log"

sleep 1

echo ""
echo "# --- WSS seed log --- #"
cat "wss-seed.log"

sleep 1

echo ""
echo "# --- WSS leech log --- #"
cat "wss-leech.log"

sleep 1

echo ""
echo "# --- Test results --- #"
# echo "HTTP (IPv4):          $HTTP_IPv4"
echo "HTTP over TLS (IPv4): $TLS_IPv4"
echo "UDP (IPv4):           $UDP_IPv4"
echo "WSS (IPv4):           $WSS_IPv4"

# if [ "$HTTP_IPv4" != "Ok" ] || [ "$TLS_IPv4" != "Ok" ] || [ "$UDP_IPv4" != "Ok" ] || [ "$WSS_IPv4" != "Ok" ]; then
if [ "$TLS_IPv4" != "Ok" ] || [ "$UDP_IPv4" != "Ok" ] || [ "$WSS_IPv4" != "Ok" ]; then
    exit 1
fi
