//! `readback-bench` — measure whether a speech-to-text stack changed your meaning.

use anyhow::Result;
use clap::{Parser, Subcommand};
use readback_bench::{dataset, metric, report, runner};
use readback_core::config::Config;
use readback_core::lexicon::{Lexicon, Locale};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "readback-bench",
    version,
    about = "Run CriticalSpeechBench and report the Critical Semantic Error Rate."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a dataset through Readback and compare it against the baseline.
    Run {
        /// A JSONL dataset. Defaults to the bundled CriticalSpeechBench v0.
        #[arg(long, value_name = "FILE")]
        dataset: Option<PathBuf>,

        /// Only run cases in this category.
        #[arg(long, value_name = "CATEGORY")]
        category: Option<String>,

        /// A Readback config file to benchmark, instead of the recommended one.
        #[arg(long, value_name = "FILE")]
        config: Option<PathBuf>,

        /// Print the full report as JSON.
        #[arg(long)]
        json: bool,

        /// Print a Markdown table for pasting into a README.
        #[arg(long, conflicts_with = "json")]
        markdown: bool,

        /// List every case, not just the failures.
        #[arg(long, short)]
        verbose: bool,
    },

    /// Score one pair of strings.
    Score {
        /// What was actually said.
        #[arg(long)]
        reference: String,

        /// What the stack produced.
        #[arg(long)]
        hypothesis: String,

        #[arg(long)]
        json: bool,
    },

    /// List the cases in a dataset.
    Cases {
        #[arg(long, value_name = "FILE")]
        dataset: Option<PathBuf>,

        #[arg(long, value_name = "CATEGORY")]
        category: Option<String>,
    },
}

fn load_cases(path: Option<&PathBuf>, category: Option<&String>) -> Result<Vec<dataset::Case>> {
    let mut cases = match path {
        Some(path) => dataset::load(path)?,
        None => dataset::bundled()?,
    };
    if let Some(category) = category {
        cases.retain(|c| &c.category == category);
    }
    Ok(cases)
}

/// The benchmark loads Hinglish because the dataset contains Hinglish cases;
/// a stack that cannot read them should score badly rather than be excused.
fn bench_config(path: Option<&PathBuf>) -> Result<Config> {
    Ok(match path {
        Some(path) => serde_json::from_str(&std::fs::read_to_string(path)?)?,
        None => Config::recommended().with_locales([Locale::En, Locale::Hinglish]),
    })
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run {
            dataset: path,
            category,
            config,
            json,
            markdown,
            verbose,
        } => {
            let cases = load_cases(path.as_ref(), category.as_ref())?;
            anyhow::ensure!(!cases.is_empty(), "no cases matched");
            let report_data = runner::run(&cases, &bench_config(config.as_ref())?)?;

            if json {
                println!("{}", serde_json::to_string_pretty(&report_data)?);
            } else if markdown {
                print!("{}", report::render_markdown(&report_data));
            } else {
                print!("{}", report::render(&report_data, verbose));
            }
        }

        Command::Score {
            reference,
            hypothesis,
            json,
        } => {
            let lexicon = Lexicon::new(&[Locale::En, Locale::Hinglish]);
            let scores = metric::score(&reference, &hypothesis, &lexicon);
            if json {
                println!("{}", serde_json::to_string_pretty(&scores)?);
            } else {
                println!();
                println!("  wer            {:.3}", scores.wer);
                println!("  cser           {:.3}", scores.cser);
                println!("  meaning flip   {}", scores.meaning_flip);
                if !scores.errors.is_empty() {
                    println!("\n  errors:");
                    for error in &scores.errors {
                        println!(
                            "    {:<13} {:<12} weight {:.1}   \"{}\" -> \"{}\"",
                            format!("{:?}", error.kind).to_lowercase(),
                            error.class.name(),
                            error.weight,
                            error.reference,
                            error.hypothesis
                        );
                    }
                }
                println!();
            }
        }

        Command::Cases {
            dataset: path,
            category,
        } => {
            let cases = load_cases(path.as_ref(), category.as_ref())?;
            println!();
            for case in &cases {
                println!(
                    "  {:<10} {:<14} {:<7} {}",
                    case.id,
                    case.category,
                    if case.benign { "control" } else { "danger" },
                    case.reference
                );
            }
            println!("\n  {} cases\n", cases.len());
        }
    }
    Ok(())
}
