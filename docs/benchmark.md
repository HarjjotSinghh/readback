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

**v0 is a text-level benchmark.** The recogniser errors are written by hand
rather than produced by running audio through a model. So it measures *how a
reliability layer responds to a given error*, not *how often that error
happens*. It cannot rank Whisper against Parakeet.

Audio fixtures are the obvious next step, and would make the numbers comparable
across engines. Until then, read the results as a response profile rather than
an engine leaderboard.

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

## Adding cases

Append a line to the JSONL file, or point `--dataset` at your own. A good case:

- has a `reference` that a person would plausibly say,
- differs from `raw`/`cleaned` in exactly one meaningful way,
- states its `category`, so it lands in the breakdown,
- and comes with a control that is *almost* the same sentence, so the benchmark
  measures discrimination rather than paranoia.

That last point is the one people skip. A layer that holds everything scores
perfectly on danger and is useless.
