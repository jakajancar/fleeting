use clap::Args;
use log::Level;
use std::{fmt::Display, io::Write};

#[derive(Args)]
#[command(next_help_heading = "Logging options")]
pub struct LoggingConfig {
    /// Output only warnings and errors, no progress.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Output additional debugging information.
    #[arg(short, long, global = true)]
    verbose: bool,
}

impl LoggingConfig {
    pub fn init(&self, process_name: impl Display, custom_context_name: Option<String>) {
        let process_name = process_name.to_string();
        env_logger::Builder::new()
            .format(move |buf, record| {
                let level_prefix = match record.level() {
                    Level::Error => "error: ",
                    Level::Warn => "warning: ",
                    Level::Info => "",
                    Level::Debug => "debug: ",
                    Level::Trace => "trace: ",
                };
                write!(buf, "fleeting[{process_name}")?;
                if let Some(context_name) = &custom_context_name {
                    write!(buf, " {context_name}")?;
                }
                writeln!(buf, "]: {level_prefix}{}", record.args())
            })
            .filter(
                None,
                if self.verbose {
                    log::LevelFilter::max()
                } else if self.quiet {
                    log::LevelFilter::Warn
                } else {
                    log::LevelFilter::Info
                },
            )
            .init();
    }
}
