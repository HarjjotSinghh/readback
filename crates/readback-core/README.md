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
readback-core = "0.1"
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

## Honest limits

- It cannot catch a word the recogniser got **confidently wrong** in a way that
  still sounds plausible — `merge` misheard as `purge` at 0.95 confidence looks
  exactly like a correct transcript from here.
- It cannot yet recover a word that was **never transcribed at all**. That needs
  the audio, and lands in v0.3 via voice-activity gaps against word timestamps.
- **Flag fatigue is the real risk.** If more than about 5% of messages get held,
  people will rip it out. Calibrate for rare, precise flags. The benchmark in
  v0.3 exists to prove the tradeoff rather than assert it.

## Roadmap

| Version | Contents                                                             |
| ------- | -------------------------------------------------------------------- |
| v0.1    | Core crate: adapters, Cleanup Guard, suspicion, rules scorer, policy  |
| v0.2    | `readback` CLI — `check`, `diff`, `explain`                            |
| v0.3    | CSER metric and CriticalSpeechBench, plus VAD-gap omission detection  |
| v0.4    | Node binding via napi-rs, published as `@readback/core`               |
| v0.5    | Reference Tauri menu-bar app demonstrating the overlay UX             |

Word error rate treats dropping `um` and dropping `not` as the same mistake,
which is why v0.3 defines a **Critical Semantic Error Rate** instead: errors
weighted by whether they altered intent.

## Status

v0.1.0. The core pipeline is implemented and tested; the API may still move
before v1.0.

## License

MIT
