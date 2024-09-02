use async_trait::async_trait;
use serde::de::DeserializeOwned;
use std::ffi::OsStr;
use tokio::process::Command;

#[async_trait]
pub trait CommandExt {
    /// Create instance using `argv[0]` for `program` and the remainder for `args`.
    fn new_argv<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(argv: I) -> Self;

    /// Configures the process to be started in background.
    fn detached(&mut self) -> &mut Self;

    /// Captures stdout and expect exit code 0.
    async fn capture_stdout(&mut self) -> anyhow::Result<Vec<u8>>;

    /// Captures stdout and expect exit code 0.
    async fn capture_json<T: DeserializeOwned>(&mut self) -> anyhow::Result<T>;
}

#[async_trait]
impl CommandExt for Command {
    fn new_argv<I: IntoIterator<Item = S>, S: AsRef<OsStr>>(argv: I) -> Self {
        let mut remainder = argv.into_iter();
        let mut command = Self::new(remainder.next().expect("arg0"));
        command.args(remainder);
        command
    }

    fn detached(&mut self) -> &mut Self {
        #[cfg(windows)]
        self.creation_flags({
            const DETACHED_PROCESS: u32 = 0x00000008;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
        });
        #[cfg(unix)]
        self.process_group(0);

        self
    }

    async fn capture_stdout(&mut self) -> anyhow::Result<Vec<u8>> {
        let output = self.output().await?;
        if !output.status.success() {
            anyhow::bail!("command failed with status {:?}: {}", output.status, String::from_utf8_lossy(&output.stderr));
        }
        Ok(output.stdout)
    }

    async fn capture_json<T: DeserializeOwned>(&mut self) -> anyhow::Result<T> {
        let stdout = self.capture_stdout().await?;
        Ok(serde_json::from_slice(&stdout)?)
    }
}
