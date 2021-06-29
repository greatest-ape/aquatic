#!/bin/bash
# 
# Test that file transfers work with aquatic_http (with and without TLS)
# and aquatic_udp.
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

$SUDO apt-get update
$SUDO apt-get install -y cmake libssl-dev screen rtorrent mktorrent ssl-cert ca-certificates

rtorrent -h

# Clone repository if necessary, go to repository directory

if [[ -z "${GITHUB_WORKSPACE}" ]]; then
    cd "$HOME"

    git clone https://github.com/greatest-ape/aquatic.git

    cd aquatic
else
    cd "$GITHUB_WORKSPACE"
fi

# Setup bogus TLS certificate

$SUDO echo "127.0.0.1    example.com" >> /etc/hosts

openssl ecparam -genkey -name prime256v1 -out key.pem
openssl req -new -sha256 -key key.pem -out csr.csr -subj "/C=GB/ST=Test/L=Test/O=Test/OU=Test/CN=example.com"
openssl req -x509 -sha256 -nodes -days 365 -key key.pem -in csr.csr -out cert.crt

$SUDO cp cert.crt /usr/local/share/ca-certificates/snakeoil.crt
$SUDO update-ca-certificates

openssl pkcs12 -export -passout "pass:p" -out identity.pfx -inkey key.pem -in cert.crt

# Build and start tracker

cargo build --bin aquatic

echo "log_level = 'debug'

[network]
address = '127.0.0.1:3000'" > http.toml
./target/debug/aquatic http -c http.toml > "$HOME/http.log" 2>&1 &

echo "log_level = 'debug'

[network]
address = '127.0.0.1:3001'
use_tls = true
tls_certificate_path = './cert.crt'
tls_private_key_path = './key.pem'
" > tls.toml
./target/debug/aquatic http -c tls.toml > "$HOME/tls.log" 2>&1 &

echo "[network]
address = '127.0.0.1:3000'" > udp.toml
./target/debug/aquatic udp -c udp.toml > "$HOME/udp.log" 2>&1 &

# Setup directories

cd "$HOME"

mkdir seed
mkdir leech
mkdir torrents

# Create torrents

echo "http-test-ipv4" > seed/http-test-ipv4
echo "tls-test-ipv4" > seed/tls-test-ipv4
echo "udp-test-ipv4" > seed/udp-test-ipv4

mktorrent -p -o "torrents/http-ipv4.torrent" -a "http://127.0.0.1:3000/announce" "seed/http-test-ipv4"
mktorrent -p -o "torrents/tls-ipv4.torrent" -a "https://example.com:3001/announce" "seed/tls-test-ipv4"
mktorrent -p -o "torrents/udp-ipv4.torrent" -a "udp://127.0.0.1:3000" "seed/udp-test-ipv4"

cp -r torrents torrents-seed
cp -r torrents torrents-leech

# Start seeding client

echo "directory.default.set = $HOME/seed
schedule2 = watch_directory,5,5,load.start=$HOME/torrents-seed/*.torrent" > ~/.rtorrent.rc

echo "Starting seeding client"
screen -dmS rtorrent-seed rtorrent

sleep 10 # Give seeding rtorrent time to load its config file

# Start leeching client

echo "directory.default.set = $HOME/leech
schedule2 = watch_directory,5,5,load.start=$HOME/torrents-leech/*.torrent" > ~/.rtorrent.rc

echo "Starting leeching client.."
screen -dmS rtorrent-leech rtorrent

# Check for completion

HTTP_IPv4="Failed"
TLS_IPv4="Failed"
UDP_IPv4="Failed"

i="0"

echo "Watching for finished files.."

while [ $i -lt 300 ]
do
    if test -f "leech/http-test-ipv4"; then
        if grep -q "http-test-ipv4" "leech/http-test-ipv4"; then
            HTTP_IPv4="Ok"
        fi
    fi
    if test -f "leech/tls-test-ipv4"; then
        if grep -q "tls-test-ipv4" "leech/tls-test-ipv4"; then
            TLS_IPv4="Ok"
        fi
    fi
    if test -f "leech/udp-test-ipv4"; then
        if grep -q "udp-test-ipv4" "leech/udp-test-ipv4"; then
            UDP_IPv4="Ok"
        fi
    fi

    if [ "$HTTP_IPv4" = "Ok" ] && [ "$TLS_IPv4" = "Ok" ] && [ "$UDP_IPv4" = "Ok" ]; then
        break
    fi

    sleep 1

    i=$[$i+1]
done

echo "Waited for $i seconds"

echo "::set-output name=http_ipv4::$HTTP_IPv4"
echo "::set-output name=http_tls_ipv4::$TLS_IPv4"
echo "::set-output name=udp_ipv4::$UDP_IPv4"

echo ""
echo "# --- HTTP log --- #"
cat "http.log"

echo ""
echo "# --- HTTP over TLS log --- #"
cat "tls.log"

echo ""
echo "# --- UDP log --- #"
cat "udp.log"

echo ""
echo "# --- Test results --- #"
echo "HTTP (IPv4):          $HTTP_IPv4"
echo "HTTP over TLS (IPv4): $TLS_IPv4"
echo "UDP (IPv4):           $UDP_IPv4"

if [ "$HTTP_IPv4" != "Ok" ] || [ "$TLS_IPv4" != "Ok" ] || [ "$UDP_IPv4" != "Ok" ]; then
    exit 1
fi