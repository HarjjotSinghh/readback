# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project uses
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). While the version is
below 1.0 the API may change in any minor release.

## [Unreleased]

## [0.13.0] - 2026-09-27

Everything needed for someone else's app to evaluate this without trusting a
word of it.

### Added

- **`readback audit --pairs <file.jsonl>`**: runs the Cleanup Guard over a whole
  history of raw/cleaned transcript pairs and reports how often the cleanup step
  changed the meaning, broken down by kind, worst examples first. Field names
  are configurable, unparseable lines are skipped rather than aborting, `-`
  reads stdin, and it exits 1 on findings so it drops into CI.
- **[docs/integrating.md](docs/integrating.md)**, written for a dictation app
  maintainer: the five-minute audit first, wiring it in second, and the
  questions a reviewer will ask answered ahead of time — including the case for
  *not* adopting it.

### Notes

The guide leads with what this does not offer. An app with no LLM cleanup step
gets nothing from the Cleanup Guard, and the acoustic stages are measurably
noisier; both are stated before any installation instruction.

## [0.12.0] - 2026-09-27

An attempt to make re-decoding cheap enough to default on, and the measurement
that says it cannot be — at least not this way.

### Added

- `RedecodeConfig::min_stakes`: skip the expensive path entirely when the
  utterance would cost nothing to get wrong. **Off by default.**

### The result

Re-decoding firing on 64% of clips is the obvious thing to fix. Two gates were
built and measured on the same run:

| Gate | Clips re-decoded | Caught | Noise |
|---|---:|---:|---:|
| none | 64% | 55.6% | 38.1% |
| sentence stakes ≥ 0.3 | 48% | 33.3% | 38.1% |
| sentence stakes ≥ 0.5 | 45% | 33.3% | 38.1% |
| span risk ≥ 0.25 | 34% | 11.1% | 35.7% |

Gating on span risk — the intuitive choice, and what this change originally set
out to do — returns catching to the no-re-decode baseline.

**The catches were coming from spans the cheap stages rated low risk.** When an
engine drops a word outright, what remains often looks perfectly confident:
tiny.en rendered `deploy this to staging` as `to staging.` without hesitation.
There is nothing suspicious in that text to gate on, which is exactly why a
second decode is the only stage that can find it — and exactly why the cheap
signals cannot decide when to run it.

Gating on sentence stakes fares better but still trades catching roughly in
proportion to what it saves, for a related reason: the worst engine failures
destroy the sentence so thoroughly that no protected token survives to raise its
stakes.

So the knob ships off, documented with what it costs rather than tuned to look
good.

## [0.11.0] - 2026-09-27

Selective re-decoding. The last unbuilt stage, and the only one that goes back
to the audio.

### Added

- **`readback_core::redecode`**: when a span looks shaky, the host is asked to
  decode *just that slice* again — a bigger model, a wider beam — and the
  answers are compared. `Readback::with_redecoder` takes any `Redecoder`; core
  still never touches audio, it only decides which slices are worth the cost.
  - A new `RedecodeDisagreement` flag, always critical.
  - `RedecodeConfig` with `padding_ms`, `max_spans`, `min_span_ms` and
    `min_overlap`.
  - `Provenance::spans_redecoded`.
- `SuspicionOutcome::candidates`: the words worth asking about again, ranked by
  weighted doubt.
- **`readback-bench audio --redecode '<cmd>'`**, substituting `{wav}`,
  `{start}` and `{duration}`. A failing second decode is treated as no opinion
  rather than taking the run down.
- `redecode_disagreement` and `spansRedecoded` in the Node binding.

### Results — tiny.en first pass, base.en re-decoding flagged slices

| | tiny.en | + base.en re-decode |
|---|---:|---:|
| Meaning flips reaching the user | 12.5% | 6.2% |
| Caught | 11.1% | 55.6% |
| Noise on correct transcripts | 35.7% | 38.1% |

Five times the catching for 2.4 points more noise — the best trade measured in
this project so far.

### Design notes

- **The disagreement test is deliberately narrow.** A second decode of a padded
  slice has *less* context than the first pass and will return something
  unrelated given the chance. A candidate is believed only when it overlaps the
  original's words by at least half *and* introduces a negation, direction verb
  or environment. Loosening either took noise from 38.1% to 47.6% for no extra
  catching.
- Only one direction counts as evidence: a word the candidate has that the
  original lacks. The reverse is far weaker, since a short slice drops words
  routinely.

### Known limits

- **The gate barely gates with a weak engine.** Re-decoding fired on 41 of 64
  clips, because low confidence is the trigger and tiny.en is unsure about
  everything. For a tool whose premise is that most utterances paste instantly,
  64% is far too often; the run took 24 s rather than 9 s.
- **The case that motivated this stage is not demonstrated by that run.**
  `cnf-003` (`do merge` for `don't merge`) is the failure no text-only stage can
  see, but tiny.en transcribed that particular clip correctly, so there was no
  error to recover and the hold it received is a false one. The mechanism is
  covered by a unit test and by the text benchmark; the audio run shows the
  stage working on the errors tiny.en did make, which is a different claim.

## [0.10.0] - 2026-09-27

The acoustic stage now asks *which* word the recogniser fumbled, not just how
badly.

### Changed

- **Acoustic suspicion is weighted per word.** A word's contribution is scaled
  by what that word carries: a protected token at full weight, an ordinary
  content word at 0.6, an article or filler at 0.1. Previously the score was the
  maximum raw doubt across the utterance, so a wobble on `the` in "deploy to
  production" scored exactly as high as a wobble on `production`. That was the
  mechanism behind most of the noise an acoustic-only run produced.
- **Trivial flags are no longer reported.** A weighted word risk below
  `SuspicionConfig::min_word_risk` (0.15) still raises the score, so policy can
  act on it, but the user is not told to look at a word that cannot flip the
  instruction.
- `suspicion::assess` takes a `&Lexicon`.

### Added

- `Lexicon::lexical_weight` and `Lexicon::is_low_information`, with a list of
  articles, fillers and common function words.

### Results

Measured on whisper.cpp base.en, 64 clips, compared at a matched catch rate:

| Caught | Noise before | Noise after |
|---:|---:|---:|
| 83.3% | 45.7% | 37.0% |

A 19% relative reduction in noise for the same catching — real, and smaller than
hoped.

### The cost, stated plainly

- **The text benchmark's catch rate fell from 100% to 95.7%.** The case lost is
  `cnf-003`, where the recogniser heard `do merge that branch` for `don't merge
  that branch`. The dangerous word in the output is `do`, an ordinary word, so no
  amount of lexical weighting can see it. This is the same blind spot 0.8.0
  recorded: when the transcript itself looks unremarkable, there is nothing to
  weight.
- **The shipped 0.35 highlight default no longer sits well on the acoustic
  curve.** The sweep now recommends 0.20 for a raw-acoustic setup. The default is
  unchanged because it is calibrated for the Cleanup Guard path, which this
  change does not touch — run `--calibrate` against your own engine.

## [0.9.0] - 2026-09-27

Threshold calibration, and the negative result it produced.

### Added

- **`readback-bench audio --calibrate`**: sweeps the highlight threshold across
  a run and reports what each cutoff would have caught and how much noise it
  would have cost. An action is a pure function of risk and the thresholds, so
  the sweep is exact and needs no re-run.
- `audio::calibrate` and `audio::best_threshold`, with a `CalibrationPoint` per
  candidate cutoff. Also exposed in `--json` output.
- **The report says when no threshold works.** If the best trade-off still marks
  more than a quarter of the clips the engine got right, the sweep says so
  plainly instead of handing over a number that looks like a fix.

### Results — three runs

| | tiny.en | base.en | base.en + cleanup |
|---|---:|---:|---:|
| Word error rate | 0.199 | 0.157 | 0.169 → 0.157 |
| Critical semantic error rate | 0.147 | 0.105 | 0.132 → 0.108 |
| Meaning flips reaching the user | 14.1% → 6.2% | 9.4% → 3.1% | 17.2% → 3.1% |
| Caught | 55.6% | 66.7% | 81.8% |
| Noise on correct transcripts | 50.0% | 39.1% | 39.0% |

The cleanup column uses a stand-in that deletes negations, which is why the
engine flip rate rises there: it is breaking transcripts the engine got right.

### What these runs establish

- **A better model helps and does not fix the noise.** tiny to base halved the
  flips reaching the user and cut the hold rate from 10.9% to 6.2%, but noise on
  correct transcripts only fell from 50% to 39%.
- **A cleanup step is worth more than a better threshold.** It is the only
  configuration where WER and CSER *improve* — with something to diff against,
  Readback repairs rather than merely flags, and catching rises to 81.8% even
  while the cleanup is itself introducing errors.
- **The acoustic-only signal is weakly discriminative.** On these runs no
  highlight cutoff separates the two populations: catching 83% of flips costs
  marking 46% of what the engine got right, and pushing noise under 10% drops
  catching to 17%. This is a negative result about the current design, recorded
  rather than tuned away.

## [0.8.0] - 2026-09-27

The first measured run against a real recogniser, and two fixes the run
uncovered.

### Security

- **Command injection in the audio harness.** Clip paths from a manifest were
  interpolated into the shell command unquoted, so a manifest entry of
  `a.wav; rm -rf ~` would have executed. Both substitutions now go through
  POSIX single-quoting. A manifest is data — it can come from another machine,
  another team, or a generator — and is treated as such. Covered by a test that
  round-trips hostile values through a real shell rather than asserting on the
  escaped string.

### Added

- **`noise on correct transcripts`** in the audio report: of the clips the
  engine transcribed without a semantic error, how many Readback marked anyway.
  This is the number that predicts flag fatigue.
- `AudioResult::engine_was_correct` and `::is_noise`, plus `engine_correct` and
  `noise_on_correct` on `Coverage`.
- `AudioResult` now carries `reference`, so reports can show what was actually
  said.
- The first published engine numbers, in [docs/benchmark.md](docs/benchmark.md).

### Fixed

- The audio report labelled the verdict text as `said:`, so a reader comparing
  "what was said" against "what was heard" was shown the transcript twice. It
  now prints the reference, the raw transcript, and the final text when the two
  differ.

### Results — whisper.cpp 1.9.4, ggml-tiny.en, 64 clips, no cleanup step

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.199 | 0.199 |
| Critical semantic error rate | 0.147 | 0.147 |
| Silent meaning flips | 14.1% | 6.2% |

WER and CSER are unchanged on purpose: with no cleanup step there is nothing to
repair, only to flag. Caught 55.6% of the engine's meaning flips.

### What the run exposed

- **Readback is blind when the transcript is destroyed.** tiny.en rendered
  `abhi mat bhejo` as `Obi Matbijo`, which passed silently. A mangled string
  contains no protected token to anchor a flag to. The layer assumes
  mostly-correct text with one dangerous word wrong; when an engine has no
  purchase on the language, that assumption fails.
- **Flag fatigue is the live risk.** Half the clips tiny.en got *right* were
  still marked up. Thresholds were calibrated against the Cleanup Guard, whose
  evidence is a concrete reverted span, not against raw acoustic confidence from
  a weak model. Per-engine calibration does not exist yet.
- **`benign` does not survive an audio run.** The flag describes whether the
  hand-written error in the text dataset was harmless, not whether the engine
  erred. Judged by `benign` the run showed 46.2% false highlights, but 8 of
  those 13 clips the engine genuinely broke. Hence the new metric above.

## [0.7.0] - 2026-09-27

Audio. The benchmark can now measure a recogniser, not just a reliability layer.

### Added

- **`readback-bench audio`**: runs the benchmark against real clips using an
  external ASR command. `{wav}` in `--asr` is replaced with the clip path, and
  whatever the command prints is parsed by the normal adapters, so any engine
  emitting whisper.cpp, faster-whisper, Parakeet or `{"text": ...}` JSON works
  without new code. `--cleanup` adds an LLM polish step, `--vad` turns on
  voice-activity detection, and `--label` names the engine in the report.
- **`readback-bench manifest`**: prints an audio manifest derived from the text
  dataset, so every clip keeps the id, category and reference of the case it
  came from and the two datasets cannot drift apart.
- **`scripts/make-fixtures.sh`**: speaks every reference with macOS `say`,
  producing 64 clips and a manifest. These are synthetic and far cleaner than
  real speech; the script says so, and the manifest format accepts real
  recordings just as happily.
- **An evidence coverage line on every audio report.** An engine that returns a
  bare string gives Readback nothing to reason about, and it will correctly pass
  everything. Rather than leaving that looking like a failure, the report now
  states how many clips arrived with confidence, timings, voice-activity regions
  and a cleanup step, and says what to do about it.
- `readback_bench::vad`, moved out of the demo app so the benchmark and the demo
  share one implementation.

### Changed

- The demo app now depends on `readback_bench::vad` instead of carrying its own
  copy.

### Notes

- **No real engine numbers are published.** Nothing in this repository has been
  run against Whisper or Parakeet; the harness exists so that users can. The
  verified runs used stand-in engines to prove the harness end to end.
- Without word **timings**, voice-activity regions cannot be turned into
  omission evidence. `--vad` will find speech and still report nothing, which
  the coverage line now explains.

## [0.6.0] - 2026-09-27

Numbers are compared by value, not by spelling. This removes the last piece of
noise the benchmark was reporting on its controls.

### Added

- **`readback_core::number`**: canonical keys for numeric tokens. `canonical`
  reduces a token to a `kind:value` key, and `canonical_phrase` handles a number
  followed by a spelled-out unit. The kind is part of the key, because dropping
  a currency symbol is itself a change.
- Four new controls and one new danger case in CriticalSpeechBench covering
  renumbering, bringing the dataset to 64 cases with 26 controls.

### Changed

- **The Cleanup Guard compares numbers by value.** `three` → `3`,
  `1000` → `1,000` and `20 percent` → `20%` now pass untouched, while
  `fifteen` → `fifty`, `$50` → `50` and `20%` → `20` are still caught.
- **CSER scores a pure re-spelling as zero**, while word error rate still counts
  it as an error. That contrast is the argument for the metric, and the
  `normalisation` category now exists to keep it honest.
- The tokeniser keeps `1,000` and `20%` whole. A comma is only absorbed between
  two digits, so `hello, world` still splits.

### Fixed

- **Romanised Hindi numerals that are ordinary English words are no longer
  treated as numbers.** `do`, `teen`, `char` and `bees` were in the Hinglish
  number list, so with that locale loaded "do merge this" read as "2 merge
  this". Reading an instruction as a quantity is worse than missing a numeral.
- `koi`, `kuch`, `kabhi` and `bilkul` moved from the Hinglish negation list to
  the quantifier list. They are only negative in combination (`koi nahi`,
  `kabhi nahi`), so labelling them negations overstated the stakes of any
  sentence containing them. They are still protected, at a more accurate weight.

### Results

CriticalSpeechBench v0, 64 cases, 26 controls:

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.220 | 0.091 |
| Critical semantic error rate | 0.303 | 0.073 |
| Silent meaning flips | 35.9% | 0.0% |

100% of the baseline's meaning flips caught. On the controls: **nothing held,
nothing highlighted.**

### Known limits

- Multi-word compounds such as `twenty five` are not reduced to a value; they
  fall back to token-by-token comparison, which is conservative rather than
  wrong.
- `quarter` is deliberately not treated as `0.25`, since "quarter past" is the
  more common reading.

## [0.5.0] - 2026-09-27

The reference app, and the last item on the 0.x roadmap.

### Added

- **`apps/demo`**, a Tauri desktop app showing what Readback decides and what
  the overlay should look like. It deliberately has **no microphone**: audio
  capture, hotkeys and text injection belong to the host, and none of them
  demonstrate the reliability layer.
  - Scenarios load from CriticalSpeechBench rather than being hard-coded, so the
    demo and the benchmark cannot drift apart. The sidebar separates cases where
    something went wrong from the controls, because clicking through the
    controls is the fastest way to feel what flag fatigue would be like.
  - Every field is editable, and the verdict updates as you type.
  - A strikethrough line shows what the stack would have pasted without Readback
    in the loop.
  - A **WAV clip** field runs a small built-in energy VAD and passes the speech
    regions to Readback the way a real host would, so omission detection can be
    seen working on real audio. The VAD lives in `apps/demo/src/vad.rs`; it is
    short-time energy with hysteresis, deliberately simple so the demo has no
    model to download, and not good enough for a noisy room.
  - Frontend is plain HTML, CSS and JavaScript with no bundler. Design tokens
    are shadcn/ui's default neutral theme, adapted to CSS custom properties.
    Motion is a single 160 ms meter transition; nothing animates on a keystroke.
- [docs/demo.md](docs/demo.md), documenting the two overlay details that are
  easy to get wrong: flag spans are **byte** offsets, and a zero-width span is a
  caret marking a missing word rather than a range to underline.
- CI builds and lints the demo app.

### Notes

- The demo is **outside the Cargo workspace** on purpose. Tauri pulls a large
  dependency tree, and keeping it separate means `cargo test` at the repository
  root stays fast.

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

[Unreleased]: https://github.com/HarjjotSinghh/readback/compare/v0.13.0...HEAD
[0.13.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.13.0
[0.12.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.12.0
[0.11.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.11.0
[0.10.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.10.0
[0.9.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.9.0
[0.8.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.8.0
[0.7.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.7.0
[0.6.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.6.0
[0.5.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.5.0
[0.4.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.4.0
[0.3.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.3.0
[0.2.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.2.0
[0.1.0]: https://github.com/HarjjotSinghh/readback/releases/tag/v0.1.0
