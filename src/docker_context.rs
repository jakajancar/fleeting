use serde_json::json;
use std::fs;

pub struct DockerContext {
    name: String,
    context_dir: String,
}

impl DockerContext {
    // TODO: change to write(), to allow creation before being written

    pub fn new(name: &str, host: &str) -> anyhow::Result<Self> {
        // Sha256 of name
        // TODO: windows
        let home = std::env::var("HOME")?;
        let name_hash = sha256(name.as_bytes());
        let context_dir = format!("{home}/.docker/contexts/meta/{name_hash}");

        let meta_json = json!({
            "Name": name,
            "Metadata": {},
            "Endpoints": {
                "docker": {
                    "Host": format!("tcp://{host}:2375"),
                    "SkipTLSVerify": false
                }
            }
        });

        log::debug!("Creating docker context {} in {}", name, context_dir);
        fs::create_dir_all(&context_dir)?;
        fs::write(format!("{context_dir}/meta.json"), serde_json::to_string(&meta_json)?)?;

        Ok(Self { name: name.to_owned(), context_dir })
    }
}

impl Drop for DockerContext {
    fn drop(&mut self) {
        log::debug!("Deleting docker context {} in {}", self.name, self.context_dir);
        if let Err(e) = fs::remove_dir_all(&self.context_dir) {
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
