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

Currently supports Amazon Web Services (EC2). Google Cloud and Hetzner are planned.

## Installation

fleeting is distributed as a single binary and available for Linux and macOS.

To install on Linux:

    curl -fsSL -o fleeting.gz https://github.com/jakajancar/fleeting/releases/latest/download/fleeting-$(uname -m)-unknown-linux-musl.gz
    gunzip fleeting.gz
    chmod +x fleeting
    mv fleeting /usr/local/bin

Windows builds are also [available](https://github.com/jakajancar/fleeting/releases), but currently untested. Feedback and contributions welcome.

## Usage

<pre>
The simplest way to &quot;docker run&quot; or &quot;docker build&quot; in the cloud

<b><u>Usage:</u></b> <b>fleeting</b> &lt;PROVIDER&gt; [OPTIONS] [COMMAND]...

Run a single docker command on an ephemeral host:

    fleeting ec2 docker run debian:bookworm echo hello world

Run multiple commands on the same ephemeral host:

    EC2_MACHINE=$(fleeting ec2 --bg)
    docker --context &quot;fleeting-$EC2_MACHINE&quot; run debian:bookworm echo hello world
    docker --context &quot;fleeting-$EC2_MACHINE&quot; run debian:bookworm echo hello again
    kill $EC2_MACHINE

<b><u>Options:</u></b>
  <b>-h</b>, <b>--help</b>
          Print help

<b><u>Task (mutually exclusive):</u></b>
      <b>--bg</b>
          Start a worker in background, print its pid, and wait until VM is up

  [COMMAND]...
          The subprocess to run

<b><u>Logging options:</u></b>
  <b>-q</b>, <b>--quiet</b>
          Output only warnings and errors, no progress

  <b>-v</b>, <b>--verbose</b>
          Output additional debugging information

<b><u>VM/Docker options:</u></b>
      <b>--context_name</b> &lt;NAME&gt;
          Name of the ephemeral docker context [default: fleeting-&lt;pid&gt;]

      <b>--dockerd-version</b> &lt;SELECTOR&gt;
          Docker version to install on server, e.g. &#39;=1.2.3&#39; or &#39;^1.2.3&#39;
          
          [default: *]

<b><u>fleeting ec2:</u></b>
AWS Elastic Compute Cloud
      <b>--region</b> &lt;REGION&gt;
          [default: $AWS[_DEFAULT]_REGION &gt; profile &gt; EC2 IMDSv2 &gt; us-east-1]

      <b>--instance-type</b> &lt;INSTANCE_TYPE&gt;
          [default: t4g.nano]

  <b>-h</b>, <b>--help</b>
          Print help

</pre>

## License

Licensed under the MIT license.
