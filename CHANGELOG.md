# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is
below 1.0 the API may change in any minor release.

## [Unreleased]

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

[Unreleased]: https://github.com/HarjjotSinghh/readback/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.1.0
