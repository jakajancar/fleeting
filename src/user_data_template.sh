#!/bin/bash
set -eu -o pipefail
trap "shutdown -h now" EXIT

mkdir /fleeting

# Write otp for readback upon connection
echo "{{otp}}" >/fleeting/otp

# Allow connections
mkdir -p /root/.ssh
echo "{{authorized_keys}}" >/root/.ssh/authorized_keys

# Stay running while someone is extending the timeout
touch /fleeting/keepalive
while [ $(( $(date +%s) - $(stat --format %Y /fleeting/keepalive) )) -le {{keepalive_timeout}} ]
do
    sleep 1
done
