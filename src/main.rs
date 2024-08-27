use clap::Parser;
use fleeting::cli::Cli;
use std::process::ExitCode;

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    tokio::select! {
        biased;
        () = fleeting::shutdown::wait_for_signal() => {
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
