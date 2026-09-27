//! Argument definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "readback",
    version,
    about = "Catch the words your dictation got wrong before you hit send.",
    long_about = "Readback inspects a speech-to-text result and decides whether it can be \
                  trusted. Point it at a transcript, optionally at the LLM-polished rewrite, \
                  and it reports which words changed meaning and what the host app should do."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    /// When to colour output.
    #[arg(long, value_enum, default_value_t = ColorChoice::Auto, global = true)]
    pub color: ColorChoice,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the full pipeline and report a verdict.
    ///
    /// Exits 0 for pass, 1 for highlight and 2 for hold, so a shell script can
    /// branch on the result without parsing anything.
    Check(CheckArgs),

    /// Show only what the Cleanup Guard did: which edits were reverted, and why.
    Diff(DiffArgs),

    /// Show the arithmetic behind a verdict, for tuning thresholds.
    Explain(CheckArgs),

    /// Audit a file of raw/cleaned transcript pairs.
    ///
    /// Point this at a dictation app's own history to find out how often its
    /// cleanup step changed what the user meant. Nothing is sent anywhere and
    /// no dependency is added.
    Audit(AuditArgs),

    /// Inspect the protected lexicon.
    #[command(subcommand)]
    Lexicon(LexiconCommand),
}

#[derive(Debug, Args)]
pub struct AuditArgs {
    /// JSONL file, one object per line, or `-` for stdin.
    #[arg(long, value_name = "FILE")]
    pub pairs: PathBuf,

    /// Field holding the raw transcript.
    #[arg(long, value_name = "NAME", default_value = "raw")]
    pub raw_field: String,

    /// Field holding the cleaned-up rewrite.
    #[arg(long, value_name = "NAME", default_value = "cleaned")]
    pub cleaned_field: String,

    #[command(flatten)]
    pub config: ConfigArgs,

    /// Print the full report as JSON.
    #[arg(long)]
    pub json: bool,

    /// Show every changed pair, not just the worst few.
    #[arg(long, short)]
    pub verbose: bool,

    /// How many examples to print. Ignored with --verbose.
    #[arg(long, default_value_t = 5, value_name = "N")]
    pub examples: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Engine {
    /// Detect from the shape of the JSON.
    Auto,
    FasterWhisper,
    WhisperCpp,
    Parakeet,
    /// `{"text": "..."}`.
    Generic,
    /// Plain text, not JSON.
    Text,
}

/// Where the raw transcript and the polished rewrite come from.
#[derive(Debug, Args)]
pub struct InputArgs {
    /// Transcript file, or `-` for stdin.
    #[arg(long, value_name = "FILE", conflicts_with = "text")]
    pub raw: Option<PathBuf>,

    /// Transcript as a literal string, instead of `--raw`.
    #[arg(long, value_name = "STRING")]
    pub text: Option<String>,

    /// How to parse `--raw`.
    #[arg(long, value_enum, default_value_t = Engine::Auto)]
    pub from: Engine,

    /// The LLM-polished rewrite, as a file or `-` for stdin.
    #[arg(long, value_name = "FILE", conflicts_with = "cleaned_text")]
    pub cleaned: Option<PathBuf>,

    /// The polished rewrite as a literal string.
    #[arg(long, value_name = "STRING")]
    pub cleaned_text: Option<String>,
}

/// Lexicon, policy and vocabulary settings shared by several commands.
#[derive(Debug, Args)]
pub struct ConfigArgs {
    /// A JSON config file, matching `readback_core::Config`.
    #[arg(long, value_name = "FILE")]
    pub config: Option<PathBuf>,

    /// Language packs to load. Repeatable.
    #[arg(long, value_name = "LOCALE")]
    pub locale: Vec<String>,

    /// An always-protected term. Repeatable.
    #[arg(long, value_name = "TERM")]
    pub vocab: Vec<String>,

    /// A file of protected terms, one per line.
    #[arg(long, value_name = "FILE")]
    pub vocab_file: Option<PathBuf>,

    /// Frontmost application, used to pick a policy.
    #[arg(long, value_name = "NAME")]
    pub app: Option<String>,

    /// Use the recommended per-app routing instead of a single default policy.
    #[arg(long)]
    pub recommended: bool,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    #[command(flatten)]
    pub input: InputArgs,

    #[command(flatten)]
    pub config: ConfigArgs,

    /// Print the verdict as JSON.
    #[arg(long)]
    pub json: bool,

    /// Print nothing; report the verdict through the exit code alone.
    #[arg(long, short, conflicts_with = "json")]
    pub quiet: bool,

    /// Always exit 0, even when the verdict is highlight or hold.
    #[arg(long)]
    pub exit_zero: bool,
}

#[derive(Debug, Args)]
pub struct DiffArgs {
    #[command(flatten)]
    pub input: InputArgs,

    #[command(flatten)]
    pub config: ConfigArgs,

    /// Print the guard result as JSON.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Subcommand)]
pub enum LexiconCommand {
    /// Classify words, one per argument.
    Classify {
        #[arg(required = true)]
        words: Vec<String>,

        #[command(flatten)]
        config: ConfigArgs,
    },
    /// List the loaded words, optionally for one class.
    List {
        /// Restrict to one class, such as `negation` or `environment`.
        #[arg(long, value_name = "CLASS")]
        class: Option<String>,

        #[command(flatten)]
        config: ConfigArgs,
    },
}

impl Engine {
    /// The name people actually call it, for reporting.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Auto => "auto",
            Engine::FasterWhisper => "faster-whisper",
            Engine::WhisperCpp => "whisper.cpp",
            Engine::Parakeet => "parakeet",
            Engine::Generic => "generic",
            Engine::Text => "text",
        }
    }
}
