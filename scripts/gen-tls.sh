#/bin/bash
# Generate self-signed TLS cert and private key for local testing

set -e

TLS_DIR="./tmp/tls"

mkdir -p "$TLS_DIR"
cd "$TLS_DIR"

openssl ecparam -genkey -name prime256v1 -out key.pem
openssl req -new -sha256 -key key.pem -out csr.csr -subj "/C=GB/ST=Test/L=Test/O=Test/OU=Test/CN=example.com"
openssl req -x509 -sha256 -nodes -days 365 -key key.pem -in csr.csr -out cert.crt
openssl pkcs8 -in key.pem -topk8 -nocrypt -out key.pk8

echo "tls_certificate_path = \"$TLS_DIR/cert.crt\""
echo "tls_private_key_path = \"$TLS_DIR/key.pk8\""
