#!/bin/bash
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
$SUDO apt-get install -y cmake libssl-dev screen rtorrent mktorrent

rtorrent -h

# Build and start tracker

if [[ -z "${GITHUB_WORKSPACE}" ]]; then
    cd "$HOME"

    git clone https://github.com/greatest-ape/aquatic.git

    cd aquatic
else
    cd "$GITHUB_WORKSPACE"
fi

cargo build --bin aquatic

echo "log_level = 'debug'

[network]
address = '127.0.0.1:3000'" > http.toml
./target/debug/aquatic http -c http.toml &

echo "[network]
address = '127.0.0.1:3000'" > udp.toml
screen -dmS aquatic-udp ./target/debug/aquatic udp -c udp.toml

# Setup directories

cd "$HOME"

mkdir seed
mkdir leech
mkdir torrents

# Create torrents

echo "http-test-ipv4" > seed/http-test-ipv4
echo "udp-test-ipv4" > seed/udp-test-ipv4

mktorrent -p -o "torrents/http-ipv4.torrent" -a "http://127.0.0.1:3000/announce" "seed/http-test-ipv4"
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
    if test -f "leech/udp-test-ipv4"; then
        if grep -q "udp-test-ipv4" "leech/udp-test-ipv4"; then
            UDP_IPv4="Ok"
        fi
    fi

    if [ "$HTTP_IPv4" = "Ok" ] && [ "$UDP_IPv4" = "Ok" ]; then
        break
    fi

    sleep 1

    i=$[$i+1]
done

echo "Waited for $i seconds"

echo "::set-output name=http_ipv4::$HTTP_IPv4"
echo "::set-output name=udp_ipv4::$UDP_IPv4"

if [ "$HTTP_IPv4" != "Ok" ] || [ "$UDP_IPv4" != "Ok" ]; then
    exit 1
fi