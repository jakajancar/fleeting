# fleeting

[![Releases](https://img.shields.io/github/v/release/jakajancar/fleeting)](https://github.com/jakajancar/fleeting/releases)
[![CI status](https://img.shields.io/github/actions/workflow/status/jakajancar/fleeting/ci.yml?branch=master&logo=github&label=ci)](https://github.com/jakajancar/fleeting/actions/workflows/ci.yml?query=branch%3Amaster)
[![MIT license](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

fleeting is a command-line utility that launches an ephemeral VM instance with Docker Engine (dockerd) in your cloud provider account and configures a docker context so you can use the Docker CLI (docker) against it. The instance will automatically terminate once fleeting exits. It is aimed at running one-off tasks, for example image builds or test runs during CI.

    % fleeting ec2 docker run debian:bookworm echo hello world
    [... progress omitted ...]
    hello world

Priorities are:

 1. **Security**: Ephemeral keys are created automatically for every VM.
 2. **Cost control**: The VM instance will auto-terminate unless fleeting sends keepalives.
 3. **Performance**: On AWS EC2, a docker host is typically ready in 30-60 seconds.

## Installation

fleeting is distributed as a single binary and available for Linux and macOS.

To install on Linux:

    curl -fsSL -o fleeting.gz https://github.com/jakajancar/fleeting/releases/latest/download/fleeting-$(uname -m)-unknown-linux-musl.gz
    gunzip fleeting.gz
    chmod +x fleeting
    mv fleeting /usr/local/bin

Windows builds are also [available](https://github.com/jakajancar/fleeting/releases), but currently untested. Feedback and contributions welcome.

## Usage

{usage_markdown}

## License

Licensed under the MIT license.
