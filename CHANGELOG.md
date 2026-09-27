# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is
below 1.0 the API may change in any minor release.

## [Unreleased]

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

[Unreleased]: https://github.com/HarjjotSinghh/readback/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.2.0
[0.1.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.1.0
