# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is
below 1.0 the API may change in any minor release.

## [Unreleased]

## [0.4.0] - 2026-09-27

The Node binding. Most open-source dictation apps are Electron or TypeScript,
and this is how they import the engine.

### Added

- **`@readback/core`**, a native Node addon built with napi-rs from the same
  Rust core, with no runtime dependency on Rust. Targets macOS (x64, arm64),
  Linux (x64, arm64) and Windows (x64).
  - `new Readback(options?)` with `locales`, `vocabulary`, `recommended`, `apps`
    and default thresholds. Rules passed in `apps` are checked before the
    recommended set, so they override it. An unknown locale throws rather than
    being silently ignored.
  - `rb.check(input)` taking `text` plus any of `cleaned`, `words`,
    `alternatives`, `signals`, `speech`, `app` and `vocabulary`. Each field
    turns on another stage, so adoption is incremental.
  - `checkCleanup(raw, cleaned, options?)` for running the Cleanup Guard alone.
  - `version()`.
- TypeScript definitions generated from the Rust source, so the doc comments in
  an editor are the ones in the crate.
- Flag kind and severity strings match the Rust JSON and the CLI output exactly,
  so output from the three can be compared directly.
- [docs/node.md](docs/node.md), including the byte-offset caveat: `flag.start`
  and `flag.end` are **byte** offsets into `verdict.text`, not JavaScript string
  indices, which stops mattering only if the text is ASCII.
- CI now builds and tests the binding on Linux, macOS and Windows.

### Notes

- The compiled `.node` artifact is not committed; it is built per platform and
  published as an npm artifact.

## [0.3.0] - 2026-09-27

Proof. A metric that can tell a dropped filler from a dropped negation, a
dataset of the cases where one word inverts an instruction, and the one error
class text alone can never catch.

### Added

- **Omission detection** (`readback_core::omission`). Given voice-activity
  regions from the host alongside word timings, Readback finds stretches of
  speech that no transcribed word covers and raises `PossibleOmission`. This is
  the only way to notice a word that was never transcribed: "merge a change like
  this" is perfectly grammatical, so no language model can tell that "never"
  used to be in front of it. Readback never decodes audio itself — the host runs
  VAD with whatever it already has and passes the regions in via
  `CheckInput::with_audio`.
- `AudioEvidence` and `SpeechRegion` types, an `OmissionConfig` section in
  `Config`, and `Provenance::omission_check_ran`.
- **`readback-bench`**, a new crate:
  - The **Critical Semantic Error Rate**: errors weighted by the class of word
    they touched, from 0.1 for punctuation to 5.0 for a negation. Punctuation is
    excluded from the alignment, so a stack cannot improve its score by dropping
    commas.
  - **CriticalSpeechBench v0**: 59 cases across 15 categories, 22 of them
    controls where the stack behaved and a good layer must stay quiet.
  - A runner reporting baseline against Readback, a per-category breakdown, and
    two explicit failure lists: cases missed, and noise raised on controls.
  - `readback-bench run`, `score` and `cases` commands, with `--json` and
    `--markdown` output.
- [docs/benchmark.md](docs/benchmark.md) documenting the weights, the dataset
  format and the benchmark's limitations.

### Results

CriticalSpeechBench v0, recommended config:

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.220 | 0.080 |
| Critical semantic error rate | 0.328 | 0.080 |
| Silent meaning flips | 39.0% | 0.0% |

100% of the baseline's meaning flips were caught or repaired, and no control was
blocked.

### Known limits

- v0 of the benchmark is **text-level**: recogniser errors are hand-written
  rather than produced from audio, so it measures how a reliability layer
  responds to an error, not how often that error occurs. It cannot rank engines.
- Readback cannot tell number **normalisation** from number **substitution**, so
  a cleanup step rewriting `three` as `3` is currently reverted. This shows up
  as the single piece of noise on the controls.
- Omission detection is probabilistic and will produce false positives in a
  noisy room.

## [0.2.0] - 2026-09-27

The `readback` command-line interface, plus the core API it needed.

### Added

- **`readback-cli`**, installing a `readback` binary with four commands:
  - `check` runs the pipeline and reports a verdict. Exits 0 for pass, 1 for
    highlight and 2 for hold, so a shell script can branch without parsing
    anything; `--exit-zero` suppresses that, `--quiet` suppresses output, and
    `--json` prints the `Verdict` verbatim.
  - `diff` runs the Cleanup Guard alone and shows which edits were reverted.
  - `explain` prints the arithmetic behind a verdict, for tuning thresholds.
  - `lexicon classify` and `lexicon list` inspect the protected word lists.
- Engine auto-detection from the shape of the payload, so `--from` is optional.
- Configuration from a JSON file (`--config`) or flags (`--locale`, `--vocab`,
  `--vocab-file`, `--app`, `--recommended`).
- Colour that follows the usual rules: on for terminals, off when piped, off
  under `NO_COLOR`, overridable with `--color`.
- `Lexicon::classify_str`, `Lexicon::words` and `Lexicon::words_destructive`,
  for inspecting a loaded lexicon.
- [docs/cli.md](docs/cli.md) documenting every command, flag and exit code.

### Fixed

- The Cleanup Guard no longer swallows trailing punctuation when reverting the
  last span of a sentence: `"Deploy this to production."` reverted against
  `"deploy this to staging"` now yields `"Deploy this to staging."` rather than
  dropping the full stop.
- Flag evidence quotes the rewritten text as it was actually written, instead of
  re-joining tokens and inventing spacing around punctuation.

## [0.1.0] - 2026-09-27

First release. `readback-core`: the pipeline that decides whether a transcript
can be trusted.

### Added

- **Adapters** normalising `faster-whisper`, `whisper.cpp`, Parakeet/NeMo and
  generic output into one `Transcript`. Engines that report no confidence at all
  are supported; the later stages carry the weight instead.
- **Cleanup Guard** (`guard`): aligns the raw transcript against an LLM-polished
  rewrite and reverts any edit that removed a protected token, keeping the rest
  of the polish byte for byte. Deterministic, no model.
- **Protected lexicon** (`lexicon`) covering negations, temporal anchors,
  direction verbs, quantifiers, modality, environments and numbers — including
  numbers spelled as words — in **English and Hinglish**, plus per-call user
  vocabulary.
- **Acoustic suspicion** (`suspicion`): per-word confidence, decoder
  hallucination tells (near-silence, repetition loops, low mean log-probability)
  and cross-engine disagreement folded into one doubt score.
- **Stakes scoring** (`scorer`): the `StakesScorer` trait plus a deterministic
  `RulesScorer`, so a local decision model can be substituted without the core
  depending on one.
- **Policy** (`policy`): per-app routing from a risk number to `pass`,
  `highlight` or `hold`, with `paranoid` and `never_hold` presets.
- `Verdict` carries the repaired text, flagged byte spans with a reason each,
  and provenance describing which stages ran.

### Known limits

- Cannot catch a word the recogniser got confidently wrong in a plausible way.
- Cannot recover a word that was never transcribed; that needs the audio and is
  planned for 0.3.

[Unreleased]: https://github.com/HarjjotSinghh/readback/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.4.0
[0.3.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.3.0
[0.2.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.2.0
[0.1.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.1.0
