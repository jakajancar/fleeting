use crate::{
    docker_context::{DockerClientKeys, DockerContext},
    ssh::{self, ChannelExt as _},
    steps,
    vm_providers::{SomeVmProvider, VmProvider},
};
use async_trait::async_trait;
use clap::Args;
use core::str;
use futures::FutureExt as _;
use rand::distributions::Alphanumeric;
use rand::Rng;
use russh::keys::PublicKeyBase64;
use std::{
    net::Ipv4Addr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::{
    io::AsyncWriteExt,
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
    // TODO: make filename
    #[clap(long, short = 'k', value_parser = ssh::parse_public_key, hide = true, global = true)]
    authorize_ssh_key: Option<String>,
}

impl WorkerConfig {
    /// The process that "owns" the remote VM (= sends heartbeats).
    /// `task` receives a docker context name.
    pub async fn spawn(&self) -> anyhow::Result<DockerContext> {
        let step = steps::start();
        log::info!("Starting an ephemeral instance...");
        let (ip, key_pair, otp) = {
            log::debug!("Generating keys...");
            let key_pair = russh::keys::key::KeyPair::generate_ed25519().expect("key generated");
            let authorized_key = format!("{} {} fleeting-ephemeral", key_pair.name(), key_pair.public_key_base64());
            log::debug!("Generated ssh key {authorized_key}");
            let otp = rand::thread_rng().sample_iter(&Alphanumeric).take(20).map(char::from).collect::<String>();
            log::debug!("Generated otp {otp}");
            let user_data = include_str!("user_data_template.sh")
                .replace("{{authorized_key}}", &authorized_key)
                .replace("{{keepalive_timeout}}", &KEEPALIVE_TIMEOUT.as_secs().to_string())
                .replace("{{otp}}", &otp);
            let ip = self.vm_provider.spawn(&user_data).await?;
            (ip, key_pair, otp)
        };
        log::info!("{ip}");

        let step: _ = step.next();
        log::info!("Attempting to connect to instance...");
        let ssh_tcp_stream = wait_for_tcp_stream(ip, 22).await?;

        let step: _ = step.next();
        log::info!("Waiting for instance setup to complete..."); // == ssh can authenticate
        let (session, mut keepalive_handle) = {
            let config = Arc::new(russh::client::Config {
                // inactivity_timeout: Some(Duration::from_secs(60)), // needed?
                ..<_>::default()
            });

            log::debug!("Establishing SSH connection...");
            let sh = ClientHandler {};
            let mut session = russh::client::connect_stream(config, ssh_tcp_stream, sh).await?;

            log::debug!("Attempting to authenticate...");
            let key_pair = Arc::new(key_pair);
            let auth_deadline = SystemTime::now() + Duration::from_secs(60); // TODO: make configurable
            loop {
                if SystemTime::now() > auth_deadline {
                    anyhow::bail!("Could not auth via SSH in time limit")
                }
                match session.authenticate_publickey("root", key_pair.clone()).await {
                    Ok(true) => break,
                    Ok(false) => {
                        log::debug!("Authentication failed, user_data probably still running");
                        sleep(Duration::from_secs(1)).await;
                    }
                    Err(e) => anyhow::bail!("failure while attempting auth: {e:?}"),
                }
            }

            log::debug!("Validating OTP...");
            let received_otp = session.channel_open_session().await?.read_file("/fleeting/otp").await?;
            let received_otp = str::from_utf8(&received_otp)?.trim();
            if received_otp != otp {
                anyhow::bail!("invalid otp, expected {otp} got {received_otp}");
            }

            log::debug!("Starting keepalive...");
            let keepalive_channel = session.channel_open_session().await?;
            let (keepalive, keepalive_handle) = async move {
                // Keeps the VM while running
                keepalive_channel.exec(true, "while read; do touch /fleeting/keepalive; done").await?;
                let mut stream = keepalive_channel.into_stream();
                loop {
                    stream.write_all(b"\n").await?;
                    sleep(KEEPALIVE_INTERVAL).await;
                }
                #[allow(unreachable_code)]
                Ok(())
            }
            .remote_handle();
            tokio::spawn(keepalive);

            (session, keepalive_handle)
        };

        let step: _ = step.next();
        log::info!("Installing dockerd...");
        {
            let install_docker_script = include_str!("install_docker.sh");
            session
                .channel_open_session()
                .await?
                .exec_passthru("install-dockerd", &install_docker_script)
                .await?;
        }
        let step: _ = step.next();
        log::info!("Generating certs...");
        {
            let prepare_certs_script = include_str!("prepare_certs.sh").replace("{{ip}}", &ip.to_string());
            session
                .channel_open_session()
                .await?
                .exec_passthru("prepare-certs", &prepare_certs_script)
                .await?;
        }

        let step: _ = step.next();
        log::info!("Downloading client keys...");
        let docker_client_keys = DockerClientKeys {
            ca: session.channel_open_session().await?.read_file("/tmp/ca.pem").await?,
            cert: session.channel_open_session().await?.read_file("/tmp/client-cert.pem").await?,
            key: session.channel_open_session().await?.read_file("/tmp/client-key.pem").await?,
        };

        let step: _ = step.next();
        log::info!("Waiting for dockerd to start...");
        let docker_context = {
            log::debug!("Starting dockerd...");
            let mut dockerd_session = session.channel_open_session().await?;
            let (dockerd, dockerd_handle) = async move {
                let command = "dockerd -H tcp://0.0.0.0:2376 --tlsverify --tlscacert=/tmp/ca.pem --tlscert=/tmp/server-cert.pem --tlskey=/tmp/server-key.pem";
                dockerd_session.exec_passthru("dockerd", command).await
            }
            .remote_handle();
            tokio::spawn(dockerd);

            log::debug!("Waiting for port to become reachable...");
            let tcp_stream_future = wait_for_tcp_stream(ip, 2376);
            tokio::select! {
                result = tcp_stream_future => {
                    result?;
                },
                result = &mut keepalive_handle => {
                    anyhow::bail!("Keepalive failed while waiting for dockerd to start: {:#}", result.unwrap_err());
                }
            }

            let context_name = self
                .custom_context_name
                .to_owned()
                .unwrap_or_else(|| format!("fleeting-{}", std::process::id()));
            DockerContext::new(context_name, ip, &docker_client_keys, keepalive_handle, dockerd_handle)?
        };
        log::info!("Docker context '{}' ready.", docker_context.name());

        steps::end(step);
        Ok(docker_context)
    }
}

struct ClientHandler {}

#[async_trait]
impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;
    async fn check_server_key(&mut self, _server_public_key: &russh::keys::key::PublicKey) -> Result<bool, Self::Error> {
        Ok(true) // will check otp instead
    }
}

/// Tries to connect for 60 seconds
async fn wait_for_tcp_stream(ip: Ipv4Addr, port: u16) -> anyhow::Result<TcpStream> {
    let deadline = SystemTime::now() + Duration::from_secs(60);
    loop {
        if SystemTime::now() > deadline {
            anyhow::bail!("Could not open tcp stream in the deadline");
        }
        match timeout(Duration::from_secs(3), TcpStream::connect((ip, port))).await {
            Ok(Ok(stream)) => break Ok(stream),
            Ok(Err(e)) => {
                log::debug!("TCP connect failed: {e}");
                sleep(Duration::from_secs(1)).await;
            }
            Err(_) => {
                log::debug!("TCP connect timeout out");
                sleep(Duration::from_secs(1)).await;
            }
        }
    }
}
