//! `readback` — inspect, debug and tune speech-to-text reliability verdicts.

mod cli;
mod commands;
mod input;
mod render;

use clap::Parser;
use cli::{Cli, Command};
use render::Style;

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let style = Style::resolve(cli.color);

    let result = match cli.command {
        Command::Check(args) => commands::check(args, style),
        Command::Explain(args) => commands::explain(args, style),
        Command::Diff(args) => commands::diff(args, style),
        Command::Audit(args) => commands::audit(args, style),
        Command::Lexicon(command) => commands::lexicon(command, style),
    };

    match result {
        Ok(code) => std::process::ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("readback: {err:#}");
            std::process::ExitCode::from(70)
        }
    }
}
