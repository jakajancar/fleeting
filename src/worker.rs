use crate::{
    arch::Arch,
    docker_context::DockerContext,
    docker_releases::get_docker_releases,
    docker_tls::DockerCA,
    ssh::{ChannelExt as _, StreamMode},
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
use semver::VersionReq;
use std::{
    fs,
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
    #[arg(long = "context-name", value_name = "NAME", global = true)]
    pub custom_context_name: Option<String>,

    /// Docker version to install on server, e.g. '=1.2.3' or '^1.2.3'.
    #[arg(long, default_value = "*", value_name = "SELECTOR", global = true)]
    pub dockerd_version: VersionReq,

    /// [INTERNAL] Authorize `~/.ssh/id_*.pub` for SSH connections
    #[clap(long, hide = true, global = true)]
    ssh: bool,
}

impl WorkerConfig {
    /// The process that "owns" the remote VM (= sends heartbeats).
    /// `task` receives a docker context name.
    pub async fn spawn(&self) -> anyhow::Result<DockerContext> {
        let step = steps::start();
        log::info!("Starting an ephemeral instance...");
        let (ip, key_pair, otp) = {
            log::debug!("Generating ephemeral ssh key...");
            let key_pair = russh::keys::key::KeyPair::generate_ed25519().expect("key generated");
            let authorized_key = format!("{} {} fleeting-ephemeral", key_pair.name(), key_pair.public_key_base64());
            log::debug!("{authorized_key}");

            let mut authorized_keys = vec![authorized_key];
            if self.ssh {
                log::debug!("Adding user's ssh keys:");
                let home_dir = dirs::home_dir().ok_or(anyhow::format_err!("cannot locate home dir"))?;
                let ssh_dir = home_dir.join(".ssh");
                if ssh_dir.exists() {
                    let pattern = ssh_dir.join("id_*.pub").to_str().expect("valid unicode").to_owned();
                    for entry in glob::glob(&pattern).expect("valid glob") {
                        let path = entry?;
                        let content = fs::read_to_string(&path)?;
                        let content = content.trim();
                        log::debug!("{path:?}: {content}");
                        authorized_keys.push(content.to_owned());
                    }
                } else {
                    log::warn!("{ssh_dir:?} does not exist");
                }
            }

            log::debug!("Generating otp...");
            let otp = rand::thread_rng().sample_iter(&Alphanumeric).take(20).map(char::from).collect::<String>();
            log::debug!("{otp}");

            let user_data = include_str!("user_data_template.sh")
                .replace("{{authorized_keys}}", &authorized_keys.join("\n"))
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
            let auth_deadline = SystemTime::now() + Duration::from_secs(60);
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
            log::debug!("Determining VM architecture...");
            let arch = session
                .channel_open_session()
                .await?
                .exec_to_completion(
                    "uname -m",
                    true,
                    None,
                    StreamMode::Capture,
                    StreamMode::Log { level: log::Level::Warn, prefix: "uname -m" },
                )
                .await?
                .stdout
                .unwrap();
            let arch: Arch = std::str::from_utf8(&arch).expect("valid utf-8").parse().expect("arch");

            log::debug!("Listing releases...");
            let releases = get_docker_releases(arch).await?;

            log::debug!("Selecting a release...");
            let release = releases.into_iter().rev().find(|(version, _)| self.dockerd_version.matches(version));
            let Some((version, tarball_url)) = release else {
                anyhow::bail!("No docker version matches requirement: {}", self.dockerd_version)
            };
            log::info!("{version}");

            log::debug!("Running install script...");
            let install_docker_script = include_str!("install_docker.sh").replace("{{tarball_url}}", tarball_url.as_str());
            session
                .channel_open_session()
                .await?
                .exec_passthru("install-dockerd", &install_docker_script)
                .await?;
        }

        let step: _ = step.next();
        log::info!("Setting up docker keys...");
        let ca = DockerCA::new()?;
        let server_tls = ca.create_server_cert(ip)?;
        let client_tls = ca.create_client_cert()?;
        session
            .channel_open_session()
            .await?
            .write_file("/tmp/ca.pem", ca.cert.pem().as_bytes())
            .await?;
        session
            .channel_open_session()
            .await?
            .write_file("/tmp/server-cert.pem", server_tls.cert.pem().as_bytes())
            .await?;
        session
            .channel_open_session()
            .await?
            .write_file("/tmp/server-key.pem", server_tls.key_pair.serialize_pem().as_bytes())
            .await?;

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
            DockerContext::new(context_name, ip, &ca.cert, &client_tls, keepalive_handle, dockerd_handle)?
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
