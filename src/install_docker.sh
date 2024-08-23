#!/bin/bash
set -eu -o pipefail

echo "Installing docker..."
cd /tmp
# TODO: multiarch
curl -fsSL -O https://download.docker.com/linux/static/stable/aarch64/docker-27.1.2.tgz
tar xzf docker-27.1.2.tgz
mv docker/* /usr/local/bin

echo "Configuring dockerd..."
#openssl genrsa -out ca-key.pem 4096
#openssl req -new -x509 -days 365 -key ca-key.pem -out ca.pem -subj "/"
#
#openssl genrsa -out server-key.pem 4096
#openssl req -subj "/CN=54.145.43.148" -sha256 -new -key server-key.pem -out server.csr
#echo subjectAltName = IP:10.10.10.20 >> extfile.cnf
#echo extendedKeyUsage = serverAuth >> extfile.cnf
#openssl x509 -req -days 365 -sha256 -in server.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial -out server-cert.pem -extfile extfile.cnf
#
#openssl genrsa -out key.pem 4096
#openssl req -subj '/CN=client' -new -key key.pem -out client.csr
#echo extendedKeyUsage = clientAuth > extfile-client.cnf
#openssl x509 -req -days 365 -sha256 -in client.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial -out cert.pem -extfile extfile-client.cnf
#
#rm -v client.csr server.csr extfile.cnf extfile-client.cnf
