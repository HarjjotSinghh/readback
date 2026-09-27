//! CriticalSpeechBench and the Critical Semantic Error Rate.
//!
//! Word error rate cannot tell you whether a transcription stack changed what
//! you meant: dropping "um" and dropping "not" are both one deleted word. This
//! crate defines a metric that can, and ships a dataset of the cases where one
//! wrong word inverts an instruction.
//!
//! ```
//! use readback_bench::{dataset, metric};
//! use readback_core::Lexicon;
//!
//! let lexicon = Lexicon::default();
//! let filler = metric::score("um merge this change", "merge this change", &lexicon);
//! let negation = metric::score("not merge this change", "merge this change", &lexicon);
//!
//! assert_eq!(filler.wer, negation.wer);       // WER cannot separate them
//! assert!(negation.cser > filler.cser * 10.0); // CSER can
//! assert!(negation.meaning_flip && !filler.meaning_flip);
//!
//! assert!(dataset::bundled().unwrap().len() >= 40);
//! ```

pub mod audio;
pub mod dataset;
pub mod metric;
pub mod report;
pub mod runner;
pub mod vad;

pub use audio::{AsrCommand, AudioCase};
pub use dataset::Case;
pub use metric::{ErrorClass, ErrorKind, Scores, SemanticError, score};
pub use runner::{CaseResult, Report, Summary, run, run_case, summarise};
