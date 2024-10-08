use anyhow::Context;
use core::str;
use futures::{future::RemoteHandle, FutureExt as _};
use rcgen::{Certificate, CertifiedKey};
use serde_json::json;
use std::{fs, future::Future, net::Ipv4Addr, path::PathBuf, task::Poll};

pub struct DockerContext {
    name: String,
    meta_dir: PathBuf,
    tls_dir: PathBuf,
    keepalive_handle: RemoteHandle<anyhow::Result<()>>,
    dockerd_handle: RemoteHandle<anyhow::Result<()>>,
}

impl DockerContext {
    pub fn new(
        name: impl Into<String>,
        ip: Ipv4Addr,
        ca_cert: &Certificate,
        ckey: &CertifiedKey,
        keepalive_handle: RemoteHandle<anyhow::Result<()>>,
        dockerd_handle: RemoteHandle<anyhow::Result<()>>,
    ) -> anyhow::Result<Self> {
        let name = name.into();
        log::debug!("Creating docker context '{}'...", name);
        let meta_json = json!({
            "Name": name,
            "Metadata": {},
            "Endpoints": {
                "docker": {
                    "Host": format!("tcp://{ip}:2376"),
                    "SkipTLSVerify": false
                }
            }
        });
        let home_dir = dirs::home_dir().ok_or(anyhow::format_err!("cannot locate home dir"))?;
        let name_hash = sha256(name.as_bytes());
        let meta_dir = home_dir.join(".docker/contexts/meta").join(&name_hash);
        let tls_dir = home_dir.join(".docker/contexts/tls").join(&name_hash);
        if meta_dir.exists() {
            anyhow::bail!("Docker context '{name}' already exists")
        }
        fs::create_dir_all(&meta_dir)?;
        fs::create_dir_all(&tls_dir.join("docker"))?;
        fs::write(meta_dir.join("meta.json"), serde_json::to_string(&meta_json)?)?;
        fs::write(tls_dir.join("docker/ca.pem"), ca_cert.pem().as_bytes())?;
        fs::write(tls_dir.join("docker/cert.pem"), ckey.cert.pem().as_bytes())?;
        fs::write(tls_dir.join("docker/key.pem"), ckey.key_pair.serialize_pem().as_bytes())?;
        Ok(Self { name, meta_dir, tls_dir, keepalive_handle, dockerd_handle })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns when either `task` completes or the context fails.
    /// If the tasks completes first, its return value is returned.
    /// If either the task or the context fail, `Err` is returned.
    pub async fn wrap<F, FRet>(self, task: F) -> anyhow::Result<FRet>
    where
        F: Future<Output = anyhow::Result<FRet>>,
    {
        tokio::select! {
            result = self => {
                match result {
                    Ok(()) => unreachable!("should not complete cleanly"),
                    Err(e) => Err(anyhow::format_err!("docker context failed before task could be completed: {e:#}")),
                }
            }
            result = task => {
                result
            }
        }
    }
}

impl Future for DockerContext {
    type Output = anyhow::Result<()>;

    fn poll(mut self: std::pin::Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        if let Poll::Ready(result) = self.keepalive_handle.poll_unpin(cx) {
            Poll::Ready(result)
        } else if let Poll::Ready(result) = self.dockerd_handle.poll_unpin(cx) {
            Poll::Ready(result)
        } else {
            Poll::Pending
        }
    }
}

impl Drop for DockerContext {
    fn drop(&mut self) {
        log::debug!("Deleting docker context '{}'...", self.name);
        if let Err(e) = fs::remove_dir_all(&self.meta_dir).context("deleting docker context meta dir") {
            log::error!("{e:#}");
        }
        if let Err(e) = fs::remove_dir_all(&self.tls_dir).context("deleting docker context tls dir") {
            log::error!("{e:#}");
        }
    }
}

fn sha256(x: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(x);
    let result = hasher.finalize();
    hex::encode(result)
}
