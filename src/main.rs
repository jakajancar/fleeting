mod cli;
mod docker_context;
mod logging;
mod shutdown;
mod ssh;
mod steps;
mod vm_providers;
mod worker;

use clap::Parser;
use cli::Cli;
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tokio::select! {
        biased;
        () = shutdown::wait_for_signal() => {
            ExitCode::FAILURE
        }
        result = cli.run() => {
            result.unwrap_or_else(|internal_error: anyhow::Error| {
                log::error!("{internal_error:#}");
                ExitCode::FAILURE
            })
        }
    }
}
