use super::VmProvider;
use crate::{command_ext::CommandExt, steps};
use async_trait::async_trait;
use base64::prelude::*;
use clap::Args;
use indoc::indoc;
use serde::Deserialize;
use std::{net::Ipv4Addr, process::Stdio};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
};

/// Canonical Multipass (local)
#[derive(Args, Clone)]
#[command(
    override_usage = color_print::cstr! {r#"<bold>fleeting</bold> <bold>multipass</bold> [OPTIONS] [COMMAND]...

This provider is primarily intended for developing and testing fleeting
itself. To get started, install multipass as described on:

    https://multipass.run/install
"#},)]
pub struct Multipass {
    /// CPUs.
    #[arg(long)]
    cpus: Option<usize>,

    /// Memory, in GBs.
    #[arg(long)]
    memory: Option<usize>,

    /// Disk size, in GiBs.
    #[arg(long)]
    disk: Option<usize>,
}

#[async_trait]
impl VmProvider for Multipass {
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr> {
        let step = steps::start();
        log::info!("Checking multipass installation...");
        {
            #[derive(Deserialize, Debug)]
            struct Version {
                #[allow(dead_code)]
                multipass: String,
                /// None if not authenticated
                #[allow(dead_code)]
                multipassd: Option<String>,
            }
            let version: Version = Command::new("multipass").arg("version").args(["--format", "json"]).capture_json().await?;
            log::debug!("{version:?}");
        }

        let step: _ = step.next();
        log::info!("Purging old stopped fleeting VMs...");
        {
            let vms = multipass_list().await?;
            let orphan_names = vms
                .into_iter()
                .filter(|vm| vm.name.starts_with("fleeting-") && vm.state == "Stopped")
                .map(|vm| vm.name)
                .collect::<Vec<String>>();

            if !orphan_names.is_empty() {
                Command::new("multipass")
                    .args(["delete", "--purge"])
                    .args(orphan_names)
                    .capture_stdout()
                    .await?;
            }
        }

        let step: _ = step.next();
        log::info!("Launching a VM...");
        let name = format!("fleeting-{}", std::process::id());
        {
            let cloud_init = format!(
                indoc! {r#"
                #cloud-config
                write_files:
                  - path: /fleeting-init
                    encoding: b64
                    content: "{}"
                runcmd:
                  - chmod +x /fleeting-init
                  - /fleeting-init &
                "#},
                BASE64_STANDARD.encode(user_data)
            );

            let mut command = Command::new("multipass");
            command.args(["launch", "--name", &name, "--cloud-init", "-", "24.04"]);
            if let Some(cpus) = self.cpus {
                command.args(["--cpus", &cpus.to_string()]);
            }
            if let Some(memory) = self.memory {
                command.args(["--memory", &memory.to_string()]);
            }
            if let Some(disk) = self.disk {
                command.args(["--disk", &disk.to_string()]);
            }
            command.stdin(Stdio::piped());
            command.stdout(Stdio::piped());
            command.stderr(Stdio::piped());

            let mut child = command.spawn()?;
            let mut stdin = child.stdin.take().unwrap();
            let mut stdout = child.stdout.take().unwrap();
            let mut stderr = child.stderr.take().unwrap();

            let mut stdout_buf = String::new();
            let mut stderr_buf = String::new();

            tokio::try_join!(
                async move {
                    stdin.write_all(cloud_init.as_bytes()).await?;
                    drop(stdin);
                    Ok(())
                },
                stdout.read_to_string(&mut stdout_buf),
                stderr.read_to_string(&mut stderr_buf),
            )?;

            let exit_status = child.wait().await?;
            if !exit_status.success() {
                anyhow::bail!("launch failed with {exit_status:?}: {}", stderr_buf.trim())
            }
        }

        let step: _ = step.next();
        log::info!("Getting VM IP...");
        let ip = {
            let vms = multipass_list().await?;
            let vm = vms
                .into_iter()
                .find(|vm| vm.name == name)
                .ok_or(anyhow::format_err!("vm that was started should be found"))?;
            vm.ipv4.into_iter().nth(0).ok_or(anyhow::format_err!("vm should have an ip"))?
        };

        steps::end(step);
        Ok(ip)
    }
}

#[derive(Deserialize, Debug)]
struct VM {
    name: String,
    ipv4: Vec<Ipv4Addr>,
    state: String,
}

async fn multipass_list() -> anyhow::Result<Vec<VM>> {
    #[derive(Deserialize, Debug)]
    struct VMList {
        list: Vec<VM>,
    }
    let list: VMList = Command::new("multipass").arg("list").args(["--format", "json"]).capture_json().await?;
    log::debug!("listed vms: {:?}", list.list);
    Ok(list.list)
}
