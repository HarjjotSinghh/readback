//! Readback catches the words your dictation got wrong before you hit send.
//!
//! Speech recognisers drop `not`, `never` and `before`. LLM cleanup steps
//! delete them on their own. Either way the sentence stays fluent, so nothing
//! downstream notices that the instruction inverted. Readback sits between the
//! recogniser and the paste, and answers one question:
//!
//! > Should I trust this, and if not, which exact words should the user look at?
//!
//! It does not record audio, own a hotkey, or paste anything. The host app
//! keeps doing all of that.
//!
//! # Example
//!
//! ```
//! use readback_core::{CheckInput, Readback, Action};
//!
//! let rb = Readback::recommended();
//! let verdict = rb.check(
//!     CheckInput::from_text("never merge a change like this")
//!         .with_cleaned("Merge a change like this."),
//! );
//!
//! assert_eq!(verdict.action, Action::Hold);
//! assert!(verdict.text.to_lowercase().contains("never"));
//! ```
//!
//! # Pipeline
//!
//! Each stage runs only when the evidence for it exists, cheapest first:
//!
//! 1. [`adapters`] normalise whatever the recogniser produced.
//! 2. [`guard`] reverts cleanup edits that removed a protected token.
//! 3. [`suspicion`] scores how shaky the recognition looks, and [`omission`]
//!    finds speech that no transcribed word covers.
//! 4. [`scorer`] scores how much a wrong word would cost here.
//! 5. [`policy`] turns the combined risk into pass, highlight or hold.
//!
//! # What it cannot do
//!
//! It cannot catch a word the recogniser got confidently wrong in a way that
//! still sounds plausible: `merge` misheard as `purge` at 0.95 confidence looks
//! exactly like a correct transcript from here.
//!
//! Words that were never transcribed at all *can* be caught, but only when the
//! host supplies voice-activity regions alongside word timings — see
//! [`omission`]. That check is probabilistic and will produce false positives in
//! a noisy room.

pub mod adapters;
pub mod align;
pub mod config;
pub mod engine;
pub mod error;
pub mod guard;
pub mod lexicon;
pub mod omission;
pub mod policy;
pub mod scorer;
pub mod suspicion;
pub mod tokenize;
pub mod types;

pub use config::Config;
pub use engine::{CheckInput, Readback};
pub use error::{AdapterError, Result};
pub use guard::{GuardOutcome, check_cleanup};
pub use lexicon::{Lexicon, Locale, SemanticClass};
pub use omission::OmissionConfig;
pub use policy::{AppPolicy, AppRule, Policy};
pub use scorer::{RulesScorer, StakesScorer};
pub use types::{
    Action, AudioEvidence, Context, DecodeSignals, Flag, FlagKind, Hypothesis, Provenance,
    Severity, Span, SpeechRegion, Transcript, Verdict, Word,
};
