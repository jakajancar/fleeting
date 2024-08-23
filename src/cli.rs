use crate::{logging::LoggingConfig, worker::WorkerConfig};
use clap::{Args, Parser};
use std::{
    env,
    ffi::OsStr,
    process::{ExitCode, Stdio},
    time::Duration,
};
use tokio::{io::AsyncReadExt as _, process::Command, time::sleep};

#[derive(Parser)]
#[command(
    about = r#"The simplest way to "docker run" or "docker build" in the cloud"#,
    override_usage = color_print::cstr! {r#"<bold>fleeting</bold> <<PROVIDER>> [OPTIONS] [COMMAND]...

Run a single docker command on an ephemeral host:

    fleeting ec2 docker run debian:bookworm echo hello world

Run multiple commands on the same ephemeral host:

    EC2_MACHINE=$(fleeting ec2 --bg)
    docker --context "fleeting-$EC2_MACHINE" run debian:bookworm echo hello world
    docker --context "fleeting-$EC2_MACHINE" run debian:bookworm echo hello again
    kill $EC2_MACHINE
"#},
    flatten_help = true,
)]

pub struct Cli {
    #[command(flatten)]
    what_to_run: WhatToRun,

    #[command(flatten)]
    logging: LoggingConfig,

    #[command(flatten)]
    worker: WorkerConfig,
}

#[derive(Args, Debug)]
#[command(next_help_heading = "Task (mutually exclusive)")]
pub struct WhatToRun {
    /// The subprocess to run.
    #[arg(trailing_var_arg = true, global = true)]
    pub command: Option<Vec<String>>,

    /// Start a worker in background, print its pid, and wait until VM is up.
    #[arg(long, global = true)]
    pub bg: bool,

    /// [INTERNAL] This is the worker for the bg launch.
    #[arg(long, hide = true, global = true)]
    pub worker: bool,
}

impl Cli {
    pub async fn run(&self) -> anyhow::Result<ExitCode> {
        match &self.what_to_run {
            WhatToRun { command: Some(command), bg: false, worker: false } => {
                // Foreground
                self.logging.init("");
                let docker_context = self.worker.spawn().await?;
                let docker_context_name = docker_context.name().to_owned();
                let user_command = run_user_command(&docker_context_name, command);
                tokio::select! {
                    result = docker_context => {
                        // Context finished first
                        let err = result.expect_err("should not complete cleanly");
                        Err(err)
                    }
                    result = user_command => {
                        // User command run finished first
                        result
                    }
                }
            }
            WhatToRun { command: None, bg: true, worker: false } => {
                // Background launcher
                self.logging.init("fleeting[launcher]");
                spawn_worker_and_wait_for_ok().await?;
                Ok(ExitCode::SUCCESS)
            }
            WhatToRun { command: None, bg: true, worker: true } => {
                // Background worker
                let process_prefix = format!(
                    "fleeting[{}{}{}]: ",
                    std::process::id(),
                    if let Some(_) = &self.worker.custom_context_name { " " } else { "" },
                    if let Some(s) = &self.worker.custom_context_name { s.as_str() } else { "" },
                );
                self.logging.init(process_prefix);
                let _docker_context = self.worker.spawn().await?;
                println!("ok");
                sleep(Duration::MAX).await;
                unreachable!()
            }
            WhatToRun { bg: false, worker: true, .. } => {
                panic!("--worker but no --bg?");
            }
            WhatToRun { command: None, bg: false, .. } | WhatToRun { command: Some(_), bg: true, .. } => {
                <Self as clap::CommandFactory>::command()
                    .error(clap::error::ErrorKind::MissingRequiredArgument, "provide exactly one of COMMAND and '--bg'")
                    .exit();
            }
        }
    }
}

async fn spawn_worker_and_wait_for_ok() -> anyhow::Result<()> {
    let mut remaining = env::args_os();
    let program = remaining.next().expect("arg0");
    let mut worker = Command::new(program)
        .arg("--worker")
        .args(remaining)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let worker_pid = worker.id().expect("child pid");
    println!("{worker_pid}"); // to allow MY_VM=$(fleeting ... --bg) for later killing
    let mut stdout = worker.stdout.take().expect("stdout");
    let mut buf = [0u8; 3];
    stdout.read_exact(&mut buf).await?;
    match &buf {
        b"ok\n" => Ok(()),
        x => Err(anyhow::anyhow!("unexpected response from child: {}", String::from_utf8_lossy(x))),
    }
}

async fn run_user_command(docker_context_name: impl Into<String>, command: impl IntoIterator<Item = impl AsRef<OsStr>>) -> anyhow::Result<ExitCode> {
    log::debug!("Running user command");
    let mut remaining = command.into_iter();
    let program = remaining.next().expect("non-empty command");
    let mut child = tokio::process::Command::new(program)
        .args(remaining)
        .env("DOCKER_CONTEXT", docker_context_name.into())
        .spawn()?;
    let exit_status = child.wait().await?;
    log::debug!("User command exited with status {exit_status:?}");
    Ok(match exit_status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => anyhow::bail!("command did not exit"), // e.g. signal
    })
}
