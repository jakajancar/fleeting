#!/bin/bash
set -eu -o pipefail

cd /tmp

openssl genrsa -out ca-key.pem 4096
openssl req -new -x509 -days 365 -key ca-key.pem -out ca.pem -subj "/"

openssl genrsa -out server-key.pem 4096
openssl req -subj "/CN=server" -sha256 -new -key server-key.pem -out server.csr
echo subjectAltName = IP:{{ip}} >> extfile-server.cnf
echo extendedKeyUsage = serverAuth >> extfile-server.cnf
openssl x509 -req -days 365 -sha256 -in server.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial -out server-cert.pem -extfile extfile-server.cnf

openssl genrsa -out client-key.pem 4096
openssl req -subj '/CN=client' -new -key client-key.pem -out client.csr
echo extendedKeyUsage = clientAuth > extfile-client.cnf
openssl x509 -req -days 365 -sha256 -in client.csr -CA ca.pem -CAkey ca-key.pem -CAcreateserial -out client-cert.pem -extfile extfile-client.cnf

rm -v client.csr server.csr extfile-server.cnf extfile-client.cnf
