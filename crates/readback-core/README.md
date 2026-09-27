# Readback

**Catch the words your dictation got wrong before you hit send.**

Readback is a local-first reliability layer for speech-to-text. It sits between
the recogniser and the paste, and answers one question:

> Should I trust this, and if not, which exact words should the user look at?

It does not record audio, own a hotkey, or paste anything. Your app keeps doing
all of that.

```
Audio:      "Don't merge that change."
ASR:        "Merge that change."
Cleanup:    "Merge that change."
Readback:   HOLD — critical: negation dropped
```

## The problem

When you mistype, you get one or two wrong letters and the meaning survives.
When a dictation stack gets a word wrong, it deletes `not`, `never` or `before`
and hands you a perfectly fluent sentence that means the opposite of what you
said.

There is a real gap between *"never merge a change like this"* and *"merge a
change like this"*, and nothing downstream can tell they were ever different.
So people reread every message they dictate, which is more tiring than typing
would have been.

Readback's bet is that you do not need perfect transcription. You need to stop
rereading everything. Roughly nine messages in ten should paste instantly; the
tenth is the one worth a second look.

## What it is

A model-agnostic engine that takes whatever evidence your stack already has —
the transcript, the polished rewrite, per-word confidence, N-best candidates,
a second engine's output — and returns a verdict:

| Action      | Meaning                                              |
| ----------- | ---------------------------------------------------- |
| `pass`      | Insert immediately. The common case.                  |
| `highlight` | Insert, but mark the suspicious spans.                |
| `hold`      | Don't insert or auto-send until the user confirms.    |

Plus the repaired text, and a list of flagged spans with a reason for each.

## What it is not

- Not a dictation app. There are already fifteen good ones.
- Not a transcription model. It never sees more than your recogniser did.
- Not an LLM cleanup step. It exists to stop those from changing your meaning.

## Quick start

```toml
[dependencies]
readback-core = "0.9"
```

```rust
use readback_core::{Action, CheckInput, Readback};

let rb = Readback::recommended();

let verdict = rb.check(
    CheckInput::from_text("never merge a change like this")
        .with_cleaned("Merge a change like this."),
);

assert_eq!(verdict.action, Action::Hold);
assert!(verdict.text.to_lowercase().contains("never")); // restored
assert_eq!(verdict.flags[0].kind, readback_core::FlagKind::DroppedNegation);
```

Everything past `raw` is optional, so adoption is incremental. Start by passing
your cleanup output and get the Cleanup Guard today; add word confidence next
month.

### With a real recogniser

```rust
use readback_core::{adapters, CheckInput, Context, Readback};

let raw = adapters::faster_whisper::from_json(&asr_output)?;

let verdict = Readback::recommended().check(
    CheckInput::new(raw)
        .with_cleaned(polished)
        .with_context(Context::for_app("Slack").with_vocabulary(["Klaviyo", "Recharge"])),
);

match verdict.action {
    Action::Pass => paste(&verdict.text),
    Action::Highlight => paste_with_marks(&verdict.text, &verdict.flags),
    Action::Hold => confirm(&verdict),
}
```

Adapters ship for `faster_whisper`, `whisper_cpp`, `parakeet` (NeMo) and a
`generic` fallback. An engine that reports no confidence at all still works —
Readback leans harder on the later stages.

## Command line

```bash
cargo install readback-cli
```

```bash
readback check --text "never merge a change like this" \
               --cleaned-text "Merge a change like this."
```

```
   HOLD   risk 0.94   stakes 0.90   suspicion 0.00

  text     never Merge a change like this.
  guard    ran, reverted a span

  critical dropped_negation — "never"
      cleanup dropped "never"; restored from the raw transcript
```

`check` exits 0 for pass, 1 for highlight and 2 for hold, so a shell script can
branch on the verdict without parsing anything. `diff` shows what the Cleanup
Guard changed, `explain` shows the arithmetic behind a verdict, and `lexicon`
inspects the protected word lists.

Full reference: [docs/cli.md](docs/cli.md).

## Node

```bash
npm install @readback/core
```

```js
import { Readback } from '@readback/core'

const rb = new Readback({ recommended: true })

const verdict = rb.check({
  text: 'never merge a change like this',
  cleaned: 'Merge a change like this.',
})

verdict.action // 'hold'
verdict.text   // 'never Merge a change like this.'  ← restored
verdict.flags  // [{ kind: 'dropped_negation', severity: 'critical', start: 0, end: 5, ... }]
```

A native addon with prebuilt binaries for macOS, Linux and Windows, and
TypeScript definitions generated from the Rust source. Flag spans are **byte**
offsets, which matters as soon as the text is not ASCII.

Full reference: [docs/node.md](docs/node.md).

## The reference app

```bash
cd apps/demo && cargo run
```

A small Tauri window that shows what Readback decides and what the overlay
should look like. **It has no microphone** — audio capture, hotkeys and text
injection are the host's job, and none of them teach you anything about the
reliability layer. Scenarios load from CriticalSpeechBench rather than being
hard-coded, so the demo and the benchmark cannot drift apart.

Every field is editable and the verdict updates as you type, which is the
fastest way to build intuition for what trips a hold. Point it at a WAV file and
a built-in energy VAD finds the speech regions, so omission detection can be
seen working on real audio.

Notes, including the two overlay details that are easy to get wrong:
[docs/demo.md](docs/demo.md).

## How it works

Five stages, cheapest first. Each runs only when the evidence for it exists.

**1. Adapters.** Every engine reports confidence differently: faster-whisper
gives word probabilities plus `avg_logprob` and `no_speech_prob`, whisper.cpp
gives subword token probabilities, NeMo has its own scheme. They all normalise
into one `Transcript`.

**2. Cleanup Guard.** Deterministic, no model, microseconds. Most meaning-flips
are not ASR errors at all — they are the LLM polish step tidying a sentence and
deleting the word that mattered. The guard aligns the raw transcript against the
polished text and checks every edit against a protected lexicon: negations,
temporal anchors, numbers, direction verbs, modality, environments, and your own
vocabulary. If the polish removed a protected token, that span is reverted and
the rest of the polish is kept. **English and Hinglish ship on day one**, because
a dropped `nahi` or `mat` inverts an instruction exactly like a dropped `not`.

**3. Acoustic suspicion.** Flags words the recogniser itself was unsure about,
plus the decoder-level hallucination tells: text written over near-silence,
repetitive output, a low mean log-probability. Disagreement between two engines
counts as evidence too.

When the host also supplies voice-activity regions, this stage catches the one
error class text can never see: a stretch of speech with no transcribed word
aligned to it. "Merge a change like this" is perfectly grammatical, so no
language model can tell that "never" used to be in front of it — but 400 ms of
unexplained speech right before "merge" can.

```rust
let verdict = rb.check(
    CheckInput::new(raw).with_audio(AudioEvidence::from_regions([(480, 1700)])),
);
```

Readback never decodes audio itself. The host runs VAD with whatever it already
has and passes the regions in.

**4. Stakes scoring.** Not every message deserves scrutiny. `"lol sounds good"`
with a shaky word is fine. `"Don't run the migration before Friday"` with a shaky
word is not. The default scorer is deterministic rules over the same lexicon.
The `StakesScorer` trait exists so a local decision model can be swapped in
without the core depending on one.

**5. Policy.** Risk combines suspicion, semantic severity and stakes, then routes
per destination app. A terminal or coding agent is paranoid; a notes app never
blocks.

```rust
use readback_core::{AppPolicy, AppRule, Config, Policy, Readback};

let policy = Policy {
    default: AppPolicy::default(),
    apps: vec![
        AppRule::new("Terminal|Ghostty|Cursor|Claude", AppPolicy::paranoid()),
        AppRule::new("Slack|Discord", AppPolicy::default()),
        AppRule::new("Notes|Obsidian", AppPolicy::never_hold()),
    ],
};

let rb = Readback::with_config(
    Config::default()
        .with_policy(policy)
        .with_vocabulary(["prod", "staging", "Recharge"]),
);
```

The sharpest case is voice-to-agent. An agent with shell access receiving
*"delete the old staging tables"* when you said *"**don't** delete the old
staging tables"* is not an embarrassing message. It's an incident.

## Does it work?

CriticalSpeechBench v0 — 64 cases, 26 of them controls where the stack behaved
and a good layer should stay quiet.

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.220 | 0.091 |
| Critical semantic error rate | 0.303 | 0.073 |
| **Silent meaning flips** | **35.9%** | **0.0%** |

Caught 100% of the baseline's meaning flips. On the 26 controls: **nothing held,
nothing highlighted.** Zero noise.

```bash
cargo run --bin readback-bench -- run
```

Read the caveat before quoting those numbers: **that dataset is text-level**.
Its recogniser errors are hand-written rather than produced from audio, so it
measures how a reliability layer responds to a given error, not how often that
error happens. The verdict mix is not representative of real traffic either,
since 38 of 64 cases are deliberately dangerous.

### Against a real engine

```bash
scripts/make-fixtures.sh
readback-bench audio --manifest fixtures/manifest.jsonl --vad \
  --asr 'whisper-cli -m ggml-tiny.en.bin -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json'
```

Measured with **whisper.cpp tiny.en** over the 64 generated clips, no cleanup
step:

| Metric | Baseline | Readback |
|---|---:|---:|
| Word error rate | 0.199 | 0.199 |
| Critical semantic error rate | 0.147 | 0.147 |
| **Silent meaning flips** | **14.1%** | **6.2%** |

WER and CSER do not move, and should not: with no cleanup step there is nothing
to repair, only to flag. Caught 55.6% of the engine's meaning flips.

With **base.en** instead, and again with a cleanup step added to the loop:

| | tiny.en | base.en | base.en + cleanup |
|---|---:|---:|---:|
| Meaning flips reaching the user | 6.2% | **3.1%** | **3.1%** |
| Caught | 55.6% | 66.7% | **81.8%** |
| Noise on correct transcripts | 50.0% | 39.1% | 39.0% |

Three findings from those runs matter more than the tables:

- **Readback is blind when the transcript is destroyed.** tiny.en turned
  `abhi mat bhejo` into `Obi Matbijo`, which passed silently — a mangled string
  has no protected tokens to anchor a flag to. The layer assumes mostly-correct
  text with one dangerous word wrong. When an engine has no purchase on the
  language at all, that assumption fails.
- **Flag fatigue is the live risk, and a better model does not fix it.** Half
  the clips tiny.en transcribed *correctly* were still marked up; base.en only
  brought that to 39%.
- **The acoustic-only signal is weakly discriminative.** `--calibrate` sweeps
  the highlight threshold and reports the trade-off, and on these runs there is
  no good cutoff: catching 83% of flips costs marking 46% of what the engine got
  right. The tool says so rather than handing over a number that looks like a
  fix. **A cleanup step to diff against is worth far more than a better
  threshold** — it is the only configuration where Readback repairs rather than
  merely flags.

Every audio report ends with a coverage line stating what evidence your engine
handed over, because one that returns a bare string gives Readback nothing to
work with and it will correctly pass everything.

Details, weights, the dataset format and the full run:
[docs/benchmark.md](docs/benchmark.md).

## Honest limits

- It cannot catch a word the recogniser got **confidently wrong** in a way that
  still sounds plausible — `merge` misheard as `purge` at 0.95 confidence looks
  exactly like a correct transcript from here.
- Recovering a word that was **never transcribed at all** needs the audio. Pass
  voice-activity regions alongside word timings and Readback will flag the gap,
  but it is probabilistic and will fire in a noisy room.
- Numbers are compared by value, so `three` → `3`, `1000` → `1,000` and
  `20 percent` → `20%` pass untouched while `fifteen` → `fifty` and `$50` → `50`
  are still caught. Longer compounds (`twenty five`) are not reduced, and fall
  back to token-by-token comparison.
- **Flag fatigue is the real risk.** If more than about 5% of messages get held,
  people will rip it out. Calibrate for rare, precise flags. The benchmark in
  v0.3 exists to prove the tradeoff rather than assert it.

## Roadmap

| Version | Contents                                                             |
| ------- | -------------------------------------------------------------------- |
| v0.1 ✅ | Core crate: adapters, Cleanup Guard, suspicion, rules scorer, policy  |
| v0.2 ✅ | `readback` CLI — `check`, `diff`, `explain`, `lexicon`                 |
| v0.3 ✅ | CSER metric and CriticalSpeechBench, plus VAD-gap omission detection  |
| v0.4 ✅ | Node binding via napi-rs, published as `@readback/core`               |
| v0.5 ✅ | Reference Tauri app demonstrating the overlay UX                      |

Word error rate treats dropping `um` and dropping `not` as the same mistake,
which is why v0.3 defines a **Critical Semantic Error Rate** instead: errors
weighted by whether they altered intent.

## Status

v0.9.0. Everything on the roadmap is built: the pipeline, the CLI, the
benchmark, the Node binding and the reference app. The API may still move before
v1.0.

What is not done, and is worth knowing before adopting:

- The bundled dataset is text-level. The audio harness lifts that limit, but
  the fixtures it generates are text-to-speech, which is far cleaner than real
  speech.
- **Without a cleanup step, the risk signal is weak.** Raw acoustic confidence
  does not cleanly separate "changed the instruction" from "fine", and no
  threshold fixes that; `readback-bench audio --calibrate` will show you the
  trade-off for your own engine.
- **A destroyed transcript defeats the layer entirely.** With no protected token
  left in the text, there is nothing to flag.
- Multi-word number compounds such as `twenty five` are not reduced to a value.
- There is no Python or Swift binding yet, and no local decision-model scorer.

## License

MIT
