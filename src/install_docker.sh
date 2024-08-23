#!/bin/bash
set -eu -o pipefail

cd /tmp
# TODO: multiarch
curl -fsSL -O https://download.docker.com/linux/static/stable/aarch64/docker-27.1.2.tgz
tar xzf docker-27.1.2.tgz
mv docker/* /usr/local/bin
