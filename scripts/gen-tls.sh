#/bin/bash

set -e

mkdir -p tmp/tls

cd tmp/tls

openssl ecparam -genkey -name prime256v1 -out key.pem
openssl req -new -sha256 -key key.pem -out csr.csr -subj "/C=GB/ST=Test/L=Test/O=Test/OU=Test/CN=example.com"
openssl req -x509 -sha256 -nodes -days 365 -key key.pem -in csr.csr -out cert.crt

sudo cp cert.crt /usr/local/share/ca-certificates/snakeoil.crt
sudo update-ca-certificates

openssl pkcs8 -in key.pem -topk8 -nocrypt -out key.pk8
