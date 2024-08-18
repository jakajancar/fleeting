mod ec2;
pub use ec2::Ec2;

use async_trait::async_trait;
use clap::{Args, Subcommand};
use std::net::Ipv4Addr;

/// A provider must define its specific CLI args and be able to spawn the VM.
#[async_trait]
pub trait VmProvider: Args + Clone {
    /// Currently we expects Ubuntu 24.04 (Noble Numbat) on arm64 or amd64
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr>;
}

#[derive(Args, Clone)]
pub struct SomeVmProvider {
    #[command(subcommand)]
    inner: SomeVmProviderEnum,
}

#[derive(Subcommand, Clone)]
#[command(subcommand_help_heading = "Providers", subcommand_value_name = "PROVIDER", disable_help_subcommand = true)]
enum SomeVmProviderEnum {
    /// AWS Elastic Compute Cloud
    Ec2(Ec2),
}

#[async_trait]
impl VmProvider for SomeVmProvider {
    async fn spawn(&self, user_data: &str) -> anyhow::Result<Ipv4Addr> {
        match &self.inner {
            SomeVmProviderEnum::Ec2(p) => p.spawn(user_data).await,
        }
    }
}
