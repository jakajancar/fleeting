use anyhow::Context;
use async_trait::async_trait;
use russh::{client::Msg, Channel, CryptoVec};

#[derive(PartialEq, Eq, Copy, Clone)]
pub enum StreamMode<'a> {
    Capture,
    Log { level: log::Level, prefix: &'a str },
}

pub struct ExecOutcome {
    #[allow(unused)]
    code: u32,
    /// Some(_) if stdout is configured to `StreamMode::Capture`.
    stdout: Option<CryptoVec>,
    /// Some(_) if stderr is configured to `StreamMode::Capture`.
    #[allow(unused)]
    stderr: Option<CryptoVec>,
}

#[async_trait]
pub trait ChannelExt {
    async fn exec_to_completion(
        &mut self,
        command: &str,
        err_on_nonzero: bool,
        stdout_mode: StreamMode<'_>,
        stderr_mode: StreamMode<'_>,
    ) -> anyhow::Result<ExecOutcome>;

    /// Context is for logs and for the returned error.
    async fn exec_passthru(&mut self, context: &str, command: &str) -> anyhow::Result<()>;

    async fn read_file(&mut self, path: &str) -> anyhow::Result<CryptoVec>;
}

#[async_trait]
impl ChannelExt for Channel<Msg> {
    async fn exec_to_completion(
        &mut self,
        command: &str,
        err_on_nonzero: bool,
        stdout_mode: StreamMode<'_>,
        stderr_mode: StreamMode<'_>,
    ) -> anyhow::Result<ExecOutcome> {
        self.exec(true, command).await?;

        let mut code = None;
        let mut stdout_buf = CryptoVec::new();
        let mut stderr_buf = CryptoVec::new();

        fn handle_data_msg(buf: &mut CryptoVec, mode: &StreamMode, data: CryptoVec) -> anyhow::Result<()> {
            buf.extend(&data);
            match mode {
                StreamMode::Capture => { /* noop */ }
                StreamMode::Log { level, prefix } => {
                    while let Some(newline_pos) = buf.iter().position(|byte| *byte == b'\n') {
                        let line = std::str::from_utf8(&buf[..newline_pos])?;
                        log::log!(*level, "{prefix}: {line}");
                        *buf = CryptoVec::from_slice(&buf[newline_pos + 1..]);
                    }
                }
            }
            Ok(())
        }

        while let Some(msg) = self.wait().await {
            match msg {
                russh::ChannelMsg::Data { data } => {
                    handle_data_msg(&mut stdout_buf, &stdout_mode, data)?;
                }
                russh::ChannelMsg::ExtendedData { data, ext } if ext == 1 => {
                    handle_data_msg(&mut stderr_buf, &stderr_mode, data)?;
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    code = Some(exit_status);
                    // cannot leave the loop immediately, there might still be more data to receive
                }
                _ => {}
            }
        }
        self.close().await?;
        let code = code.ok_or(anyhow::format_err!("channel ended without ExitStatus"))?;
        if err_on_nonzero && code != 0 {
            anyhow::bail!("remote command returned {code}");
        }
        Ok(ExecOutcome {
            code,
            stdout: if stdout_mode == StreamMode::Capture { Some(stdout_buf) } else { None },
            stderr: if stderr_mode == StreamMode::Capture { Some(stderr_buf) } else { None },
        })
    }

    async fn exec_passthru(&mut self, context: &str, command: &str) -> anyhow::Result<()> {
        let passthru = StreamMode::Log { level: log::Level::Debug, prefix: context };
        self.exec_to_completion(command, true, passthru, passthru).await.context(context.to_owned())?;
        Ok(())
    }

    async fn read_file(&mut self, path: &str) -> anyhow::Result<CryptoVec> {
        let command = format!("cat {path}");
        let outcome = self
            .exec_to_completion(
                &command,
                true,
                StreamMode::Capture,
                StreamMode::Log { level: log::Level::Debug, prefix: &command },
            )
            .await?;
        Ok(outcome.stdout.unwrap())
    }
}
