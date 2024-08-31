use crate::steps::{self, fmt::StepExt as _};
use anyhow::Context as _;
use clap::Args;
use either::Either;
use log::{Level, LevelFilter, Log};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::Mutex,
};

type LogLinePrefix = String;

#[derive(Args)]
#[command(next_help_heading = "Logging options")]
pub struct LoggingConfig {
    /// Output only warnings and errors, no progress.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Output additional debugging information.
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Log file for the background worker.
    ///
    /// Applicable only when using '--while'.
    /// Helps debugging docker context failures after the foreground launcher has exited.
    #[arg(long, value_name = "PATH", global = true)]
    pub log_file: Option<String>,
}

impl LoggingConfig {
    /// If `file_logging` is present and `self.log_file` is set, will log to the log file, otherwise not.
    ///
    /// The stderr logger will be set even if full (file) logging could not be configured and `Err` is returned`.
    pub fn init(&self, file_logging: Option<LogLinePrefix>) -> anyhow::Result<()> {
        let user_chosen_level = if self.verbose {
            LevelFilter::Debug
        } else if self.quiet {
            LevelFilter::Warn
        } else {
            LevelFilter::Info
        };

        let mut errors: Vec<anyhow::Error> = Vec::new();

        let logger = Logger {
            level_filter: user_chosen_level,
            show_steps: user_chosen_level >= LevelFilter::Info,
            file_logging: if let Some(prefix) = file_logging {
                if let Some(path) = &self.log_file {
                    let path = Path::new(path);
                    match OpenOptions::new().create(true).append(true).open(path).context("opening log file") {
                        Ok(file) => Some((Mutex::new(file), prefix)),
                        Err(e) => {
                            errors.push(e);
                            None // failed to open log file for writing
                        }
                    }
                } else {
                    None // log file not enabled
                }
            } else {
                None // process does not do file logging
            },
        };

        log::set_boxed_logger(Box::new(logger)).unwrap();
        log::set_max_level(user_chosen_level);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.into_iter().nth(0).unwrap())
        }
    }
}

struct Logger {
    level_filter: LevelFilter,
    show_steps: bool,
    file_logging: Option<(Mutex<File>, LogLinePrefix)>,
}

impl Log for Logger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        if record.level() > self.level_filter {
            return;
        }

        if let Some(module_path) = record.module_path() {
            if module_path != "fleeting" && !module_path.starts_with("fleeting::") {
                return;
            }
        }

        let step_prefix = match self.show_steps {
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

        // Write in a single write
        let stderr_line = format!("{step_prefix}{level_prefix}{}\n", record.args());
        std::io::stderr().write_all(stderr_line.as_bytes()).unwrap_or(());

        if let Some((file, file_prefix)) = &self.file_logging {
            let file_line = format!("{file_prefix}{stderr_line}");
            let mut guard = file.lock().unwrap();
            guard.write_all(file_line.as_bytes()).unwrap_or(());
        }
    }

    fn flush(&self) {}
}
