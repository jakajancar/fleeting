#!/bin/bash
set -eu -o pipefail
trap "shutdown -h now" EXIT

mkdir /fleeting

# Write otp for readback upon connection
echo "{{otp}}" >/fleeting/otp

# Allow connections
mkdir -p /root/.ssh
echo "{{authorized_key}}" >/root/.ssh/authorized_keys
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAlQWouySQLhr+MfmROC7JN+lyPNyKd4x/lEP2efgC+L jaka@kubje.org" >>/root/.ssh/authorized_keys # TODO: remove

# Stay running while someone is extending the timeout
touch /fleeting/keepalive
while [ $(( $(date +%s) - $(stat --format %Y /fleeting/keepalive) )) -le {{keepalive_timeout}} ]
do
    sleep 1
done
