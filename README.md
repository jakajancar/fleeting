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

### General

<pre>
The simplest way to &quot;docker run&quot; or &quot;docker build&quot; in the cloud

<b><u>Usage:</u></b> <b>fleeting</b> &lt;PROVIDER&gt; [OPTIONS] [COMMAND]...

Run a single docker command on an ephemeral host:

    fleeting ec2 docker run debian:bookworm echo hello world

Run multiple commands on the same ephemeral host:

    fleeting ec2 --while $$ --context-name greeter
    docker --context greeter run debian:bookworm echo hello world
    docker --context greeter run debian:bookworm echo hello again

<b><u>Providers:</u></b>
  <b>ec2</b>        AWS Elastic Compute Cloud
  <b>gce</b>        Google Compute Engine
  <b>multipass</b>  Canonical Multipass (local)

<b><u>Options:</u></b>
  <b>-h</b>, <b>--help</b>
          Print help (see a summary with &#39;-h&#39;)

<b><u>Task (mutually exclusive):</u></b>
      <b>--while</b> &lt;PID&gt;
          Keep the VM/Docker context alive in background while PID is running.
          
          When started with &#39;--while&#39;, fleeting does the following:
          
          1. Starts a detached worker in background and prints its PID to stdout
          so it can be captured (VM_PID=$(fleeting ...)) and killed explicitly,
          if desired.
          
          2. Waits for the worker to finish launching a Docker context and
          exits. The exit code is 0 is the VM started successfully or 1 if not.
          This ensures the following commands have a fully-functioning Docker
          context.
          
          3. The worker monitors PID and exits when it is no longer running.
          Consider using $$, $PPID or 1 as PID.

  [COMMAND]...
          The subprocess to run

<b><u>Logging options:</u></b>
  <b>-q</b>, <b>--quiet</b>
          Output only warnings and errors, no progress

  <b>-v</b>, <b>--verbose</b>
          Output additional debugging information

      <b>--log-file</b> &lt;PATH&gt;
          Log file for the background worker.
          
          Applicable only when using &#39;--while&#39;. Helps debugging docker context
          failures after the foreground launcher has exited.

<b><u>VM/Docker options:</u></b>
      <b>--context-name</b> &lt;NAME&gt;
          Name of the ephemeral docker context [default: fleeting-&lt;pid&gt;]

      <b>--dockerd-version</b> &lt;SELECTOR&gt;
          Docker version to install on server, e.g. &#39;=1.2.3&#39; or &#39;^1.2.3&#39;
          
          [default: *]
</pre>

### AWS Elastic Compute Cloud

<pre>
<b><u>Usage:</u></b> <b>fleeting</b> <b>ec2</b> [OPTIONS] [COMMAND]...

<b><u>Authentication:</u></b>
  - Environment variables (AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY)
  - Shared config (~/.aws/config, ~/.aws/credentials)
  - Web Identity Tokens
  - ECS (IAM Roles for Tasks) &amp; General HTTP credentials
  - EC2 IMDSv2

More info:
https://docs.rs/aws-config/1.5.5/aws_config/default_provider/credentials/struct.DefaultCredentialsChain.html

<b><u>Options:</u></b>
      <b>--region</b> &lt;REGION&gt;
          [default: $AWS[_DEFAULT]_REGION &gt; profile &gt; EC2 IMDSv2 &gt; us-east-1]

      <b>--instance-type</b> &lt;INSTANCE_TYPE&gt;
          [default: t4g.nano]

      <b>--disk</b> &lt;DISK&gt;
          Disk size, in GiBs
</pre>

### Google Compute Engine

<pre>
<b><u>Usage:</u></b> <b>fleeting</b> <b>gce</b> [OPTIONS] [COMMAND]...

<b><u>Authentication:</u></b>
  - GOOGLE_APPLICATION_CREDENTIALS (pointing to JSON file)
  - gcloud auth application-default login
  - Metadata server, if running on GCE

<b><u>Setup:</u></b>
  - Create a project
  - Enable the Compute Engine API for it
  - Create a service account and download credentials JSON

<b><u>Limitations:</u></b>
While GCE instances will automatically stop, they will not be automatically
deleted. fleeting collects garbage at the beginning of the run, but you will
be left with a small number of stopped instances and will continue to pay for
their associated disks. Hopefully, this will be resolved in the future with
termination_time / max_run_duration, once GCE client libraries support it.

<b><u>Options:</u></b>
      <b>--project</b> &lt;PROJECT&gt;
          Project in which to create instances [required]

      <b>--zone</b> &lt;ZONE&gt;
          [default: us-central1-a]

      <b>--machine-type</b> &lt;MACHINE_TYPE&gt;
          [default: e2-micro]

      <b>--disk</b> &lt;DISK&gt;
          Disk size, in GiBs
</pre>

### Canonical Multipass (local)

<pre>
<b><u>Usage:</u></b> <b>fleeting</b> <b>multipass</b> [OPTIONS] [COMMAND]...

This provider is primarily intended for developing and testing fleeting
itself. To get started, install multipass as described on:

    https://multipass.run/install

<b><u>Options:</u></b>
      <b>--cpus</b> &lt;CPUS&gt;
          CPUs

      <b>--memory</b> &lt;MEMORY&gt;
          Memory, in GBs

      <b>--disk</b> &lt;DISK&gt;
          Disk size, in GiBs
</pre>



## License

Licensed under the MIT license.
