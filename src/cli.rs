use crate::{logging::LoggingConfig, worker::WorkerConfig};
use clap::{Args, Parser};
use futures::{future::FusedFuture, FutureExt, TryFutureExt as _};
use std::{
    env,
    ffi::OsStr,
    future::Future,
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

    EC2_MACHINE=$(fleeting ec2 --while $$)
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
#[command(next_help_heading = "Task (mutually exclusive)", max_term_width = 80)]
pub struct WhatToRun {
    /// The subprocess to run.
    #[arg(trailing_var_arg = true, global = true)]
    pub command: Option<Vec<String>>,

    /// Keep the VM/Docker context alive in background while PID is running.
    ///
    /// When started with '--while', fleeting does the following:
    ///
    ///  1. Starts a detached worker in background and prints its PID to stdout so it can be killed explicitly, if desired.
    ///
    ///  2. Waits for the worker to finish launching a Docker context and exits.
    ///     The exit code is 0 is the VM started successfully or 1 if not.
    ///     This ensures the following commands have a fully-functioning Docker context.
    ///
    ///  3. The worker monitors PID and exits when it is no longer running.
    ///     Consider using $$, $PPID or 1 as PID.
    #[arg(long, value_name = "PID", global = true)]
    pub r#while: Option<u32>,

    /// [INTERNAL] This is the worker for the --while/background launch.
    #[arg(long, value_name = "LAUNCHER_PID", hide = true, global = true)]
    pub worker: Option<u32>,
}

impl Cli {
    pub async fn run(&self) -> anyhow::Result<ExitCode> {
        match &self.what_to_run {
            WhatToRun { command: Some(command), r#while: None, worker: None } => {
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
            WhatToRun { command: None, r#while: Some(_), worker: None } => {
                // Background launcher
                self.logging.init("fleeting[launcher]: ");
                let (worker_pid, context_ready) = spawn_worker(std::process::id())?;
                println!("{worker_pid}"); // to allow MY_VM=$(fleeting ... --while PID) for later killing
                let succeeded = context_ready.await?;
                if succeeded {
                    Ok(ExitCode::SUCCESS)
                } else {
                    Ok(ExitCode::FAILURE)
                }
            }
            WhatToRun { command: None, r#while: Some(watch_pid), worker: Some(launcher_pid) } => {
                // Background worker
                let process_prefix = format!(
                    "fleeting[{}{}{}]: ",
                    std::process::id(),
                    if let Some(_) = &self.worker.custom_context_name { " " } else { "" },
                    if let Some(s) = &self.worker.custom_context_name { s.as_str() } else { "" },
                );
                self.logging.init(process_prefix);

                let launcher_exited = waitpid(*launcher_pid).map_err(|e| e.context("waitpid launcher")).fuse();
                let watch_exited = waitpid(*watch_pid).map_err(|e| e.context("waitpid user process")).fuse();
                let docker_context_ready = self.worker.spawn().fuse();

                tokio::pin!(launcher_exited);
                tokio::pin!(watch_exited);
                let _docker_context = tokio::select! {
                    biased;
                    result = docker_context_ready => {
                        let docker_context = result?;
                        println!("ok");
                        docker_context
                    }
                    result = &mut launcher_exited => {
                        result?;
                        log::warn!("Launcher exited (killed?) before docker context was ready, aborting.");
                        return Ok(ExitCode::SUCCESS)
                    }
                    result = &mut watch_exited => {
                        result?;
                        log::warn!("Watched process exited before docker context was ready, aborting.");
                        return Ok(ExitCode::SUCCESS)
                    }
                };

                loop {
                    tokio::select! {
                        biased;
                        result = &mut launcher_exited, if !launcher_exited.is_terminated() => {
                            result?;
                            log::debug!("Launcher exited after sending OK (expected)");
                        }
                        result = &mut watch_exited => {
                            result?;
                            log::info!("Watched processes exited, exiting.");
                            break Ok(ExitCode::SUCCESS)
                        }
                    }
                }
            }
            WhatToRun { r#while: None, worker: Some(_), .. } => {
                panic!("--worker but no --while?");
            }
            WhatToRun { command: None, r#while: None, .. } | WhatToRun { command: Some(_), r#while: Some(_), .. } => {
                <Self as clap::CommandFactory>::command()
                    .error(clap::error::ErrorKind::MissingRequiredArgument, "provide exactly one of COMMAND and '--while'")
                    .exit();
            }
        }
    }
}

/// Spawns the worker and returns its pid and a future that completes when it has reported the context is ready or exited.
fn spawn_worker(launcher_pid: u32) -> anyhow::Result<(u32, impl Future<Output = anyhow::Result<bool>>)> {
    let mut remaining = env::args_os();
    let program = remaining.next().expect("arg0");
    let mut command = Command::new(program);
    command.args(remaining);
    command.args(["--worker", &launcher_pid.to_string()]);

    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());
    #[cfg(windows)]
    command.creation_flags({
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
    });
    #[cfg(unix)]
    command.process_group(0);

    let mut worker = command.spawn()?;
    let worker_pid = worker.id().expect("child pid");
    let context_ready = async move {
        let mut stdout = worker.stdout.take().expect("stdout");
        let mut buf = [0u8; 3];
        match stdout.read_exact(&mut buf).await {
            Ok(_) if &buf == b"ok\n" => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Ok(_) => Err(anyhow::anyhow!("unexpected response: {}", String::from_utf8_lossy(&buf))),
            Err(e) => Err(anyhow::Error::new(e)),
        }
    };
    Ok((worker_pid, context_ready))
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

async fn waitpid(pid: u32) -> anyhow::Result<()> {
    let pid = sysinfo::Pid::from_u32(pid);
    loop {
        // Must recreate `system` to remove dead processes, see `refresh_processes_specifics` docs.
        let mut system = sysinfo::System::new();
        system.refresh_processes_specifics(sysinfo::ProcessesToUpdate::Some(&[pid]), sysinfo::ProcessRefreshKind::new());
        if let Some(_) = system.process(pid) {
            sleep(Duration::from_secs(1)).await;
        } else {
            break Ok(());
        }
    }
}
