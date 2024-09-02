use clap::{Command, CommandFactory};
use fleeting::cli::Cli;
use std::fs;

fn main() {
    let mut usage_markdown = String::new();
    let mut command = <Cli as CommandFactory>::command();

    // General
    usage_markdown += &render_section("General", &mut command);

    // For each provider
    for subcommand in command.get_subcommands_mut() {
        // Hide global ones to avoid repetition
        let mut subcommand = subcommand.clone().mut_args(|mut arg| {
            if arg.is_global_set() {
                arg = arg.hide(true)
            }
            arg
        });

        // Hide help to avoid repetition
        subcommand = subcommand.disable_help_flag(true);

        // Render about as heading
        let about = subcommand.get_about().unwrap().to_string();
        subcommand = subcommand.about(None);

        usage_markdown += &render_section(&about, &mut subcommand);
    }

    let readme = format!(include_str!("readme_template.md"), usage_markdown = usage_markdown);
    fs::write("README.md", readme).unwrap();
}

fn render_section(heading: &str, command: &mut Command) -> String {
    let styled_str = command.render_long_help();
    let ansi = styled_str.ansi();
    let ansi = format!("{ansi}");
    let html = ansi_to_html::convert(&ansi).unwrap();
    format!("### {heading}\n\n<pre>\n{html}</pre>\n\n")
}
