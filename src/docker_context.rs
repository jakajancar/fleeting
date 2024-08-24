use anyhow::Context;
use core::str;
use futures::{future::RemoteHandle, FutureExt as _};
use russh::CryptoVec;
use serde_json::json;
use std::{fs, future::Future, net::Ipv4Addr, task::Poll};

pub struct DockerClientKeys {
    pub ca: CryptoVec,
    pub cert: CryptoVec,
    pub key: CryptoVec,
}

pub struct DockerContext {
    name: String,
    context_meta_dir: String,
    context_tls_dir: String,
    keepalive_handle: RemoteHandle<anyhow::Result<()>>,
    dockerd_handle: RemoteHandle<anyhow::Result<()>>,
}

impl DockerContext {
    pub fn new(
        name: impl Into<String>,
        ip: Ipv4Addr,
        keys: &DockerClientKeys,
        keepalive_handle: RemoteHandle<anyhow::Result<()>>,
        dockerd_handle: RemoteHandle<anyhow::Result<()>>,
    ) -> anyhow::Result<Self> {
        let name = name.into();
        log::debug!("Creating docker context '{}'...", name);
        let context_meta_json = json!({
            "Name": name,
            "Metadata": {},
            "Endpoints": {
                "docker": {
                    "Host": format!("tcp://{ip}:2376"),
                    "SkipTLSVerify": false
                }
            }
        });
        let home_dir = std::env::var("HOME")?;
        let context_name_hash = sha256(name.as_bytes());
        let context_meta_dir = format!("{home_dir}/.docker/contexts/meta/{context_name_hash}");
        let context_tls_dir = format!("{home_dir}/.docker/contexts/tls/{context_name_hash}");
        fs::create_dir_all(&context_meta_dir)?;
        fs::create_dir_all(&format!("{context_tls_dir}/docker"))?;
        fs::write(format!("{context_meta_dir}/meta.json"), serde_json::to_string(&context_meta_json)?)?;
        fs::write(format!("{context_tls_dir}/docker/ca.pem"), &keys.ca)?;
        fs::write(format!("{context_tls_dir}/docker/cert.pem"), &keys.cert)?;
        fs::write(format!("{context_tls_dir}/docker/key.pem"), &keys.key)?;
        Ok(Self { name, context_meta_dir, context_tls_dir, keepalive_handle, dockerd_handle })
    }

    pub fn name(&self) -> &str {
        &self.name
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
        if let Err(e) = fs::remove_dir_all(&self.context_meta_dir).context("deleting docker context meta dir") {
            log::error!("{e:#}");
        }
        if let Err(e) = fs::remove_dir_all(&self.context_tls_dir).context("deleting docker context tls dir") {
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
