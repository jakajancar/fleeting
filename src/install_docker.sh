#!/bin/bash
set -eu -o pipefail

cd /tmp
curl -fsSL -o docker.tgz "{{tarball_url}}"
tar xzf docker.tgz
mv docker/* /usr/local/bin
