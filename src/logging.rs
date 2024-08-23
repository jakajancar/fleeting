use crate::steps::{self, fmt::StepExt as _};
use clap::Args;
use either::Either;
use log::{Level, LevelFilter};
use std::io::Write;

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
    pub fn init(&self, process_prefix: impl Into<String>) {
        let process_prefix = process_prefix.into();
        let user_chosen_level = if self.verbose {
            LevelFilter::Debug
        } else if self.quiet {
            LevelFilter::Warn
        } else {
            LevelFilter::Info
        };
        let show_steps = user_chosen_level >= LevelFilter::Info;

        env_logger::Builder::new()
            .format(move |buf, record| {
                let step_prefix = match show_steps {
                    true => Either::Left(steps::current().log_prefix()),
                    false => Either::Right(""),
                };

                let level_prefix = match record.level() {
                    Level::Error => "error: ",
                    Level::Warn => "warning: ",
                    Level::Info => "",
                    Level::Debug => "debug: ",
                    Level::Trace => "trace: ",
                };
                writeln!(buf, "{process_prefix}{step_prefix}{level_prefix}{}", record.args())?;
                Ok(())
            })
            .filter(None, user_chosen_level.min(LevelFilter::Warn)) // gets too crazy otherwise
            .filter(Some("fleeting"), user_chosen_level)
            .init();
    }
}
