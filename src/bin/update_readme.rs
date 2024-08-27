use clap::CommandFactory;
use fleeting::cli::Cli;
use std::fs;

fn main() {
    let usage = <Cli as CommandFactory>::command().render_long_help();
    let usage = usage.ansi();
    let usage_html = ansi_to_html::convert(&format!("{usage}")).unwrap();
    let readme = format!(include_str!("readme_template.md"), usage_html = usage_html);
    fs::write("README.md", readme).unwrap();
}
