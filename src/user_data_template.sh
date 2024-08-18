#!/bin/bash
set -eu -o pipefail
trap "shutdown -h now" EXIT

mkdir /fleeting

# The otp...
echo "{{otp}}" >/fleeting/otp

# Prepare the /fleeting/extend-timeout script
cat <<'EOF' >/fleeting/extend-timeout
#!/bin/bash
set -eu -o pipefail
echo $(( $(date +%s) + {{keepalive_timeout}} )) >/fleeting/timeout.new
mv -f /fleeting/timeout.new /fleeting/timeout
EOF
chmod +x /fleeting/extend-timeout

# Allow connections
mkdir -p /root/.ssh
echo "{{authorized_key}}" >/root/.ssh/authorized_keys
echo "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAlQWouySQLhr+MfmROC7JN+lyPNyKd4x/lEP2efgC+L jaka@kubje.org" >>/root/.ssh/authorized_keys # TODO: remove

# Stay running while someone is extending the timeout
/fleeting/extend-timeout
while [ $(date +%s) -le $(</fleeting/timeout) ]
do
    sleep 1
done
