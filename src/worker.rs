use crate::{
    docker_context::{self, DockerContext},
    vm_providers::{SomeVmProvider, VmProvider},
};
use anyhow::Context;
use async_trait::async_trait;
use clap::Args;
use core::str;
use futures::{
    future::{BoxFuture, LocalBoxFuture},
    stream::FuturesUnordered,
    FutureExt, StreamExt as _, TryFutureExt,
};
use indoc::indoc;
use rand::distributions::Alphanumeric;
use rand::Rng;
use russh::keys::PublicKeyBase64;
use std::{
    future::Future,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::io::AsyncWriteExt;
use tokio::{
    net::TcpStream,
    time::{sleep, timeout},
};

const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Args)]
#[command(next_help_heading = "VM/Docker options")]
pub struct WorkerConfig {
    #[command(flatten)]
    vm_provider: SomeVmProvider,

    /// Name of the ephemeral docker context [default: fleeting-<pid>]
    #[arg(long = "context_name", global = true)]
    pub custom_context_name: Option<String>,

    // [INTERNAL] Extra key to authorize. Not sure we'll use SSH in the future, so not public API.
    #[clap(long, short = 'k', value_parser = parse_ssh_public_key, hide = true, global = true)]
    authorize_ssh_key: Option<String>,
}

impl WorkerConfig {
    /// The process that "owns" the remote VM (= sends heartbeats).
    /// `task` receives a docker context name.
    #[tokio::main(flavor = "current_thread")]
    pub async fn run_task<F, FFut, FRet>(&self, task: F) -> anyhow::Result<FRet>
    where
        F: FnOnce(&str) -> FFut,
        FFut: Future<Output = anyhow::Result<FRet>>,
    {
        let context_name = self
            .custom_context_name
            .to_owned()
            .unwrap_or_else(|| format!("fleeting-{}", std::process::id()));

        let key_pair = russh::keys::key::KeyPair::generate_ed25519().expect("key generated");
        let authorized_key = format!("{} {} fleeting-ephemeral", key_pair.name(), key_pair.public_key_base64());
        println!("Generated key {authorized_key}");

        let otp = rand::thread_rng().sample_iter(&Alphanumeric).take(20).map(char::from).collect::<String>();

        // TODO: ensure timeout for every step

        // Launch instance
        let user_data = include_str!("user_data_template.sh")
            .replace("{{authorized_key}}", &authorized_key)
            .replace("{{keepalive_timeout}}", &KEEPALIVE_TIMEOUT.as_secs().to_string())
            .replace("{{otp}}", &otp);
        let ip = self.vm_provider.spawn(&user_data).await?;
        println!("Provider reported ip {ip}, polling with SSH...");

        // Attempt to connect
        let tcp_deadline = SystemTime::now() + Duration::from_secs(60); // TODO: make configurable
        let tcp_stream = loop {
            if SystemTime::now() > tcp_deadline {
                anyhow::bail!("Could not open tcp stream in the deadline");
            }

            match timeout(Duration::from_secs(3), TcpStream::connect((ip, 22))).await {
                Ok(Ok(stream)) => break stream,
                Ok(Err(e)) => {
                    println!("tcp connect failed: {e}");
                    sleep(Duration::from_secs(1)).await;
                }
                Err(_) => {
                    println!("tcp connect timeout out");
                    sleep(Duration::from_secs(1)).await;
                }
            }
        };

        let config = Arc::new(russh::client::Config {
            // inactivity_timeout: Some(Duration::from_secs(60)), // just in case smth goes wrong
            ..<_>::default()
        });

        let sh = Client {};
        let mut session = russh::client::connect_stream(config, tcp_stream, sh).await?;

        // Attempt to authenticate
        let key_pair = Arc::new(key_pair);
        let auth_deadline = SystemTime::now() + Duration::from_secs(60); // TODO: make configurable
        loop {
            if SystemTime::now() > auth_deadline {
                anyhow::bail!("Could not auth via SSH in XYZ seconds")
            }
            match session.authenticate_publickey("root", key_pair.clone()).await {
                Ok(true) => break,
                Ok(false) => {
                    println!("authentication failed, user_data probably still running");
                    sleep(Duration::from_secs(1)).await;
                }
                Err(e) => anyhow::bail!("failure while attempting auth: {e:?}"),
            }
        }

        println!("Connected via SSH!");

        log::debug!("Validating OTP...");
        let received_otp = session.call_get_output("cat /fleeting/otp").await?;
        let received_otp = str::from_utf8(&received_otp)?.trim();
        if received_otp != otp {
            anyhow::bail!("invalid otp, expected {otp} got {received_otp}");
        }

        // Start keepalive and dockerd
        let mut bg_jobs: FuturesUnordered<LocalBoxFuture<anyhow::Result<()>>> = [
            keepalive(&session).map(|r| r.context("keepalive")).boxed_local(),
            dockerd(&session).map(|r| r.context("dockerd")).boxed_local(),
        ]
        .into_iter()
        .collect();

        // Wait for docker to become reachable
        // Attempt to connect
        let docker_ready = async {
            let docker_deadline = SystemTime::now() + Duration::from_secs(60);
            loop {
                if SystemTime::now() > docker_deadline {
                    anyhow::bail!("Could not connect to docker in the deadline");
                }

                match timeout(Duration::from_secs(3), TcpStream::connect((ip, 2375))).await {
                    Ok(Ok(_stream)) => break,
                    Ok(Err(e)) => {
                        println!("tcp connect failed: {e}");
                        sleep(Duration::from_secs(1)).await;
                    }
                    Err(_) => {
                        println!("tcp connect timeout out");
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            Ok(())
        };

        tokio::select! {
            result = bg_jobs.next() => {
                result.unwrap()?;
                unreachable!();
            }
            result = docker_ready => {
                result?;
                // connectable
            }
        };

        let _docker_context = DockerContext::new(&context_name, &ip.to_string())?;

        // Start user task
        let task = task(&context_name);

        tokio::select! {
            result = bg_jobs.next() => {
                result.unwrap()?;
                unreachable!();
            }
            result = task => {
                result // return return value
            }
        }
    }
}

type Session = russh::client::Handle<Client>;

async fn keepalive(session: &Session) -> anyhow::Result<()> {
    loop {
        let exit_code = session.call("/fleeting/extend-timeout").await?;
        assert_eq!(exit_code, 0);
        sleep(KEEPALIVE_INTERVAL).await;
    }
}

// TODO: error handling
async fn dockerd(session: &Session) -> anyhow::Result<()> {
    session
        .call(indoc! {r###"
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

        echo "Starting dockerd..."
        exec dockerd -H tcp://0.0.0.0:2375
    "###})
        .await?;
    unreachable!();
}
struct Client {}

#[async_trait]
impl russh::client::Handler for Client {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _server_public_key: &russh::keys::key::PublicKey) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

#[async_trait]
trait HandleExt {
    async fn call(&self, command: &str) -> anyhow::Result<u32>;
    async fn call_get_output(&self, command: &str) -> anyhow::Result<Vec<u8>>;
}

#[async_trait]
impl<C: russh::client::Handler> HandleExt for russh::client::Handle<C> {
    async fn call(&self, command: &str) -> anyhow::Result<u32> {
        let mut channel = self.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut code = None;
        let mut stdout = tokio::io::stdout();
        let mut stderr = tokio::io::stderr();

        loop {
            // There's an event available on the session channel
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                // Write data to the terminal
                russh::ChannelMsg::Data { ref data } => {
                    stdout.write_all(data).await?;
                    stdout.flush().await?;
                }
                russh::ChannelMsg::ExtendedData { ref data, ext } if ext == 1 => {
                    stderr.write_all(data).await?;
                    stderr.flush().await?;
                }
                // The command has returned an exit code
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    code = Some(exit_status);
                    // cannot leave the loop immediately, there might still be more data to receive
                }
                _ => {}
            }
        }
        // channel.close().await?;
        Ok(code.expect("program did not exit cleanly"))
    }

    async fn call_get_output(&self, command: &str) -> anyhow::Result<Vec<u8>> {
        let mut channel = self.channel_open_session().await?;
        channel.exec(true, command).await?;

        let mut code = None;
        let mut stdout = Vec::new();
        let mut stderr = tokio::io::stderr();

        loop {
            // There's an event available on the session channel
            let Some(msg) = channel.wait().await else {
                break;
            };
            match msg {
                // Write data to the terminal
                russh::ChannelMsg::Data { ref data } => stdout.extend_from_slice(data),
                russh::ChannelMsg::ExtendedData { ref data, ext } if ext == 1 => {
                    stderr.write_all(data).await?;
                    stderr.flush().await?;
                }
                // The command has returned an exit code
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    code = Some(exit_status);
                    // cannot leave the loop immediately, there might still be more data to receive
                }
                _ => {}
            }
        }
        // channel.close().await?;
        let code = code.expect("program did not exit cleanly");
        assert_eq!(code, 0);
        Ok(stdout)
    }

    // async fn close(&mut self) -> Result<()> {
    //     self.session.disconnect(russh::Disconnect::ByApplication, "", "English").await?;
    //     Ok(())
    // }
}

fn parse_ssh_public_key(arg: &str) -> anyhow::Result<String> {
    if !arg.starts_with("ssh-") {
        anyhow::bail!("ssh public key should start with 'ssh-'")
    }
    Ok(arg.to_owned())
}

fn parse_duration(arg: &str) -> Result<std::time::Duration, std::num::ParseIntError> {
    let seconds = arg.parse()?;
    Ok(Duration::from_secs(seconds))
}
