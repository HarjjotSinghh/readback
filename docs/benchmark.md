# CriticalSpeechBench and CSER

Word error rate cannot tell you whether a transcription stack changed what you
meant. Dropping "um" and dropping "not" are both one deleted word, so both cost
the same. One is irrelevant and the other inverts an instruction.

This is the metric and the dataset for measuring the difference.

## Critical Semantic Error Rate

Every token is classified, and every error is weighted by the class of the word
it touched:

| Class        | Weight | Example                        |
| ------------ | -----: | ------------------------------ |
| punctuation  |    0.1 | a missing comma                |
| filler       |    0.2 | `um`, `basically`, `like`      |
| article      |    0.3 | `a` → `the`                    |
| ordinary     |    1.0 | `send` → `sent`                |
| proper noun  |    2.0 | `Harpawan` → `Harpreet`        |
| number       |    3.0 | `fifteen` → `fifty`            |
| temporal     |    3.0 | `before` → `after`             |
| modality     |    3.0 | `must` → `might`               |
| quantifier   |    3.0 | `only`, `all`                  |
| direction    |    4.0 | `enable` → `disable`           |
| environment  |    4.0 | `staging` → `production`       |
| negation     |    5.0 | `don't` → `do`                 |

```
CSER = Σ weight(error) / Σ weight(reference token)
```

A substitution costs whichever side carries more meaning, so replacing a
negation with an ordinary word is still charged as a negation error.
Punctuation is excluded from the alignment entirely, so a stack cannot improve
its score by dropping commas.

**Meaning flip**: any error weighted 4.0 or above. These are the ones where the
sentence now instructs something different from what was said. The headline
number is the **silent meaning flip rate** — how often a changed instruction
reaches the user with no warning at all.

The weights are a deliberate, documented choice rather than a derived constant.
They are published so they can be argued with; that is the point.

## The dataset

`crates/readback-bench/data/critical-speech-bench-v0.jsonl`, one JSON object per
line:

```json
{
  "id": "neg-001",
  "category": "negation",
  "reference": "never merge a change like this",
  "raw": "never merge a change like this",
  "cleaned": "Merge a change like this."
}
```

| Field        | Meaning                                                        |
| ------------ | -------------------------------------------------------------- |
| `reference`  | What the speaker actually said. Everything is scored against it. |
| `raw`        | What the recogniser produced.                                    |
| `cleaned`    | What an LLM cleanup step rewrote `raw` into. Optional.           |
| `words`      | Per-word confidence and timings. Optional.                       |
| `speech`     | Voice-activity regions, for omission cases. Optional.            |
| `app`        | Destination app, for cases that depend on routing. Optional.     |
| `vocabulary` | Terms the user would have in their dictionary. Optional.         |
| `benign`     | A control: the stack behaved, so a good layer stays quiet.       |

v0 has 64 cases across 15 categories, 26 of them controls.

### An important limitation

**The bundled dataset is text-level.** Its recogniser errors are written by hand
rather than produced by running audio through a model, so it measures *how a
reliability layer responds to a given error*, not *how often that error
happens*. On its own it cannot rank Whisper against Parakeet.

That is what the audio harness below is for. Read the text results as a response
profile, and run the audio harness when you want engine numbers.

The verdict mix is also not representative of real traffic: 38 of 64 cases are
deliberately dangerous, so the hold rate here is far above what anyone would
see while actually dictating.

## Running it

```bash
cargo run --bin readback-bench -- run
```

```
  CriticalSpeechBench — 64 cases, 26 controls

                                 baseline   readback
  --------------------------------------------------
  word error rate                   0.220      0.091
  critical semantic error rate      0.303      0.073
  silent meaning flips              35.9%       0.0%

  caught                           100.0%   caught or repaired, of the baseline's flips
  false hold rate                    0.0%   controls blocked
  false highlight rate               0.0%   controls marked up
  pass / highlight / hold           42.2% / 26.6% / 31.2%
```

| Flag           | Meaning                                                    |
| -------------- | ---------------------------------------------------------- |
| `--dataset`    | A JSONL file. Defaults to the bundled v0 dataset.           |
| `--category`   | Run one category only.                                      |
| `--config`     | Benchmark a specific Readback config instead of the default.|
| `--json`       | The full report, including every case.                      |
| `--markdown`   | A table for pasting into a README or release note.          |
| `--verbose`    | List every case, not just the failures.                     |

Other commands:

```bash
# Score one pair by hand
readback-bench score --reference "never merge this" --hypothesis "merge this"

# List the cases
readback-bench cases --category negation
```

## Reading the results

The report always prints two lists of failures, because the aggregate numbers
hide the interesting parts:

- **missed** — cases where a changed instruction still passed unflagged. These
  are the real failures.
- **noise on controls** — controls that got held or marked up. Flag fatigue is
  what gets a tool like this uninstalled, so this list matters as much as the
  first one.

In the current run both lists are empty: nothing was missed, and no control was
touched.

That was not true before 0.6.0. `ben-016` used to be highlighted, because the
cleanup step rewrote `three` as `3` and the guard treated it as a protected
number edit. Numbers are now compared **by value**, so re-spellings pass while
real changes are still caught:

| Rewrite | Verdict |
| --- | --- |
| `three` → `3` | passes |
| `1000` → `1,000` | passes |
| `20 percent` → `20%` | passes |
| `fifteen` → `fifty` | caught |
| `$50` → `50` | caught — dropping a currency is a change |
| `20%` → `20` | caught |

Note what CSER does with a pure re-spelling: it scores **zero**, while WER still
counts it as a word error. That contrast is the entire argument for the metric,
and the `normalisation` category exists to keep it honest.

## Running it against real audio

The text dataset measures how a reliability layer *responds* to an error. It
cannot measure how often an engine *makes* one. The audio harness closes that
gap: point it at clips and an ASR command, and the same CSER machinery scores
what the engine actually produced.

### Generating fixtures

```bash
scripts/make-fixtures.sh            # writes fixtures/clips + fixtures/manifest.jsonl
READBACK_VOICE=Daniel scripts/make-fixtures.sh
```

The script speaks every reference in the text dataset with macOS `say`, so the
two datasets stay in step: each clip keeps the id, category and reference of the
text case it came from.

**These are synthetic.** Text-to-speech is clean, evenly paced and never
mumbles, so an engine will score far better here than on real speech. Treat the
numbers as a floor and a regression guard, not as a claim about field accuracy.
The manifest format does not care where a clip came from — replace them with
real recordings when you have them:

```json
{"id":"neg-001","category":"negation","reference":"never merge a change like this","wav":"clips/neg-001.wav"}
```

### Running an engine

```bash
readback-bench audio --manifest fixtures/manifest.jsonl --vad \
  --asr 'whisper-cli -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json'
```

`{wav}` is replaced with the clip path. Whatever the command prints to stdout is
parsed by the normal adapters, so any engine that emits whisper.cpp,
faster-whisper, Parakeet or `{"text": ...}` JSON works without new code. Plain
text works too.

| Flag         | Meaning                                                          |
| ------------ | ---------------------------------------------------------------- |
| `--asr`      | The engine command. Must contain `{wav}`.                         |
| `--label`    | Name for the engine in the report. Defaults to the program name.  |
| `--cleanup`  | An LLM polish step. Must contain `{text}`; stdout is the result.  |
| `--vad`      | Run voice-activity detection so dropped words can be found.       |
| `--markdown` | A table for pasting into a README.                                |

### Ask your engine for word-level output

This is the part that decides whether the run means anything. Every audio report
ends with a coverage line:

```
  evidence supplied              0/64 confidence   0/64 timings   62/64 vad   0/64 cleanup
  the engine reported text only, and no cleanup step was given, so there
  was nothing for Readback to reason about.
```

An engine that returns a bare string gives Readback nothing to work with, and it
will correctly pass everything. Use `whisper-cli --output-json-full`, or
faster-whisper with `word_timestamps=True`. Without **timings**, voice-activity
regions cannot be turned into omission evidence, so `--vad` will find speech and
still report nothing.

### What a run looks like

Two stand-in engines, over the 64 generated clips (100.6 s of audio). Neither is
a real recogniser; they exist to show what the harness measures.

A **perfect engine** — everything correct:

```
  critical semantic error rate      0.000      0.000
  pass / highlight / hold          100.0% / 0.0% / 0.0%
```

Nothing flagged, nothing held. That is the control for the harness itself.

A **word-level engine with a cleanup step that deletes negations**, which is the
failure everyone actually reports:

```
                                 baseline   readback
  word error rate                   0.050      0.000
  critical semantic error rate      0.087      0.000
  silent meaning flips              21.9%       0.0%

  caught                           100.0%
  evidence supplied             64/64 confidence   64/64 timings   62/64 vad   64/64 cleanup
```

Two honest notes on that run. The false hold rate reads 11.5%, but those are
controls whose negations the stand-in cleanup stripped as well — `don't merge
this before Friday` really was broken, so holding it is correct, and the
`benign` flag simply came from the text dataset where the cleanup behaved. And
voice-activity detection found speech in 62 of 64 clips; the two misses are the
shortest utterances.

### A real engine: whisper.cpp tiny.en

The first measured run. `whisper.cpp` 1.9.4 with `ggml-tiny.en`, over the 64
generated clips, no cleanup step:

```bash
readback-bench audio --manifest fixtures/manifest.jsonl --vad \
  --label 'whisper.cpp tiny.en' \
  --asr 'whisper-cli -m ggml-tiny.en.bin -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json'
```

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.199 | 0.199 |
| Critical semantic error rate | 0.147 | 0.147 |
| **Silent meaning flips** | **14.1%** | **6.2%** |

```
  engine meaning flips              14.1%   the engine changed the instruction
  caught                            55.6%   caught or repaired, of the baseline's flips
  pass / highlight / hold           45.3% / 43.8% / 10.9%
  noise on correct transcripts      50.0%   of the 42 clips the engine got right
  evidence supplied             64/64 confidence   64/64 timings   62/64 vad   0/64 cleanup
```

**Read WER and CSER as unchanged on purpose.** There is no cleanup step in this
run, so Readback has nothing to repair — it can only flag. The number that moves
is the one that matters: how often a changed instruction reached the user with
no warning, which more than halved.

What tiny.en actually did to these sentences:

```
    env-001    said:   deploy this to staging
               heard:  to staging.              ← the verb vanished
    env-003    said:   point it at localhost
               heard:  pointed at localfist.
    dir-004    said:   revert the change
               heard:  Reverse the change.      ← direction verb flipped
    hin-001    said:   abhi mat bhejo
               heard:  Obi Matbijo              ← passed silently
```

### The two findings worth more than the table

**Readback is blind when the transcript is destroyed.** The Hinglish clips came
back as `Obi Matbijo` and `Yinahi Karna Hai` and **passed silently**, because a
mangled string contains no protected tokens at all — no negation, no
environment, no number — so stakes score near zero and there is nothing to
anchor a flag to. Readback assumes the recogniser produces *mostly* correct
text with a dangerous word wrong. When an engine has no purchase on the language
at all, that assumption fails and the layer has nothing to say. This is a real
limit, not a tuning problem.

**Flag fatigue is the live risk, and this run proves it.** Half of the clips
tiny.en transcribed *correctly* were still marked up. The README's own threshold
is that a tool holding more than about 5% of messages gets uninstalled; the hold
rate here is 10.9%, and highlighting — which is cheap, an underline rather than
a block — ran at 43.8%.

Two things drive that. tiny.en is the weakest model available and its per-word
confidence is correspondingly poor, so the acoustic suspicion stage fires
constantly. And the default thresholds were calibrated against the Cleanup
Guard, whose evidence is a concrete reverted span, not against raw acoustic
confidence from a weak model. **Thresholds should be calibrated per engine**,
and nothing in the project does that yet.

### `benign` does not survive an audio run

A methodology note that cost a wrong number before it was caught. The manifest's
`benign` flag describes whether the *hand-written* error in the text dataset was
harmless. It says nothing about whether the engine erred on the same sentence.

Judged by `benign`, this run had a 46.2% false highlight rate. But 8 of those 13
"controls" were clips tiny.en genuinely broke — `yeah lol sounds good to me`
came back as `Yellow L sounds good to me`, and `ship it when CI passes` as
`Ship at Wednesday eye passes`. Flagging those is correct behaviour, not noise.

So the audio report also computes **noise on correct transcripts**: of the clips
the engine got right, how many Readback marked anyway. That is the number that
predicts flag fatigue, and it is the one to watch.

### Three runs compared

`tiny.en` and `base.en`, then `base.en` with a cleanup step added to the loop.
The cleanup is a stand-in that deletes negations, which is why the engine flip
rate rises in the third column — it is breaking transcripts the engine got right.

| | tiny.en | base.en | base.en + cleanup |
|---|---:|---:|---:|
| Word error rate | 0.199 | 0.157 | 0.169 → **0.157** |
| Critical semantic error rate | 0.147 | 0.105 | 0.132 → **0.108** |
| Meaning flips reaching the user | 14.1% → 6.2% | 9.4% → **3.1%** | 17.2% → **3.1%** |
| Caught | 55.6% | 66.7% | **81.8%** |
| Noise on correct transcripts | 50.0% | 39.1% | 39.0% |

Three things fall out of this.

**A better model helps, and does not fix the noise.** Going from tiny to base
halved the flips that reached the user and cut the hold rate from 10.9% to 6.2%,
but noise on correct transcripts only fell from 50% to 39%. Model quality is not
the lever for flag fatigue.

**A cleanup step is worth more than a better threshold.** It is the only column
where word error rate and CSER actually *improve* — with something to diff
against, Readback repairs rather than merely flags, and catching jumps to 81.8%
even though the cleanup itself was introducing errors.

**The acoustic-only signal is weakly discriminative.** That is what the
threshold sweep shows.

### Re-decoding shaky slices

The only stage that goes back to the audio. When a span looks shaky, the host is
asked to decode *just that slice* again — a bigger model, a wider beam — and the
answers are compared.

```bash
readback-bench audio --manifest fixtures/manifest.jsonl --vad \
  --asr 'whisper-cli -m ggml-tiny.en.bin -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json' \
  --redecode 'whisper-cli -m ggml-base.en.bin -f {wav} -ot {start} -d {duration} -bs 8 --no-prints'
```

`{start}` and `{duration}` are milliseconds. A cheap first pass with an
expensive second opinion is the intended shape.

tiny.en as the first pass, base.en re-decoding only the flagged slices:

| | tiny.en | + base.en re-decode |
|---|---:|---:|
| Meaning flips reaching the user | 12.5% | **6.2%** |
| Caught | 11.1% | **55.6%** |
| Noise on correct transcripts | 35.7% | 38.1% |

**Five times the catching for 2.4 points more noise.** That is the best trade in
this document, and it is the only stage that can act on evidence the text does
not contain.

Two things to know before turning it on.

**The expensive path fired on 41 of 64 clips.** Re-decoding is gated on low
confidence, and a weak recogniser makes everything low-confidence, so the gate
barely gates. For a dictation tool whose whole premise is that most utterances
paste instantly, 64% is far too often — the run took 24 s instead of 9 s. Cap it
with `max_spans`, and expect a stronger engine to trigger it far less.

**The disagreement test is deliberately narrow.** A second decode of a padded
slice has *less* context than the first pass and will happily return something
unrelated, so a candidate is only believed when it overlaps the original's words
by at least half *and* introduces a negation, direction verb or environment.
Loosening either — counting any protected class, or skipping the overlap check —
took noise from 38.1% to 47.6% in testing for no extra catching.

### Can the expensive path be gated? Not cheaply

Re-decoding firing on 64% of clips is the obvious thing to fix, and the obvious
fix does not work. Two gates were built and measured on the same run:

| Gate | Clips re-decoded | Caught | Noise |
|---|---:|---:|---:|
| none | 64% | **55.6%** | 38.1% |
| sentence stakes ≥ 0.3 | 48% | 33.3% | 38.1% |
| sentence stakes ≥ 0.5 | 45% | 33.3% | 38.1% |
| span risk ≥ 0.25 | 34% | 11.1% | 35.7% |

Gating on **span risk** — how unsure the recogniser was about that word, scaled
by what the word carries and what the sentence costs — is the intuitive choice
and the worst one. It returns catching to the no-re-decode baseline.

The reason is worth stating, because it is the shape of the whole problem. **The
catches were coming from spans the cheap stages rated low risk.** When an engine
drops a word outright, what remains often looks perfectly confident: tiny.en
rendered `deploy this to staging` as `to staging.` with no hesitation at all.
There is nothing suspicious in that text to gate on. That is precisely why a
second decode is the only stage that can find it — and precisely why you cannot
use the cheap signals to decide when to run it.

Gating on sentence **stakes** fares better but still trades catching roughly in
proportion to what it saves, for a related reason: the worst engine failures
destroy the sentence so thoroughly that no protected token survives to raise its
stakes.

So `min_stakes` ships **off by default**, and the knob is documented with what
it costs rather than tuned to look good. Turn it up when latency matters more
than catching.

**On the case that motivated this stage.** `cnf-003` (`do merge` for `don't
merge`) is the failure no text-only stage can see. It is not demonstrated by the
run above: tiny.en transcribed that particular clip correctly, so there was no
error to recover, and the hold it received is a false one. The mechanism is
covered by a unit test and by the text benchmark; this audio run shows the stage
working on the errors tiny.en *did* make, which is a different claim.

### Calibrating thresholds

```bash
readback-bench audio --manifest fixtures/manifest.jsonl --vad --calibrate --asr '...'
```

The sweep recomputes what each highlight cutoff would have done. An action is a
pure function of risk and the thresholds, so this is exact and needs no re-run.

```
  highlight    caught     noise   marked
       0.30     83.3%     45.7%       36
       0.35     66.7%     39.1%       32
       0.50     33.3%     26.1%       20
       0.65     16.7%      2.2%        4

  best trade-off for this engine: highlight at 0.30 (83.3% caught, 45.7% noise)

  but no threshold separates these two populations well: the best
  trade-off still marks 45.7% of the clips this engine got right.
```

Since 0.10.0 the acoustic signal is weighted per word, which moved the curve:

| Caught | Noise before | Noise after |
|---:|---:|---:|
| 83.3% | 45.7% | **37.0%** |

That is a 19% relative reduction in noise at the same catch rate — real, and
smaller than hoped. It also means **the shipped 0.35 default no longer sits at a
sensible point on this curve** for an acoustic-only setup; the sweep recommends
0.20. The default is left alone because it is calibrated for the Cleanup Guard
path, which this change does not touch.

**Read the sweep as a negative result, because it still is one.** There is no good cutoff
on this run. Catching 83% costs marking nearly half of what the engine got
right; getting noise under 10% drops catching to 17%. The report says so rather
than handing over a number that looks like a fix.

What it means: with no cleanup step to diff against, risk is driven by raw
acoustic confidence, and a whisper-family model's per-word confidence does not
cleanly separate "changed the instruction" from "fine". The Cleanup Guard's
evidence — a concrete reverted span — is far stronger, which is what the third
column above demonstrates.

### Reproducing this

```bash
brew install whisper-cpp
curl -L -o ggml-base.en.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
scripts/make-fixtures.sh

readback-bench audio --manifest fixtures/manifest.jsonl --vad --calibrate \
  --asr 'whisper-cli -m ggml-base.en.bin -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json'
```

## Adding cases

Append a line to the JSONL file, or point `--dataset` at your own. A good case:

- has a `reference` that a person would plausibly say,
- differs from `raw`/`cleaned` in exactly one meaningful way,
- states its `category`, so it lands in the breakdown,
- and comes with a control that is *almost* the same sentence, so the benchmark
  measures discrimination rather than paranoia.

That last point is the one people skip. A layer that holds everything scores
perfectly on danger and is useless.
