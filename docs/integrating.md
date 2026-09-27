# Integrating Readback

This is written for people who maintain a dictation app. It starts with a check
you can run in five minutes against your own data, before adding any dependency
at all.

## What this is actually offering you

If your app runs an LLM cleanup step over the raw transcript — "polish",
"format", "clean up" — then some fraction of the time that step deletes a word
that carried the meaning. `never merge this` becomes `Merge this.` The result is
fluent, confident, and the opposite of what the user said. Nothing downstream
notices, because there is nothing wrong with the text.

The Cleanup Guard catches exactly that. It aligns the raw transcript against the
polished one, and reverts any edit that removed a protected token — negations,
temporal anchors, numbers, direction verbs, environments, the user's own
vocabulary. The rest of the polish is kept, byte for byte.

It is deterministic, needs no model, runs in microseconds, and is the part of
this project with the strongest evidence behind it: on the bundled benchmark it
catches 100% of meaning-changing edits and raises **zero** false positives on
the controls.

**If your users write Chinese, Japanese, Korean or Thai, this has nothing to
offer them.** Every stage splits on whitespace and those scripts do not use it,
so a whole sentence arrives as a single word and nothing can be compared. A
deleted `不要` passes silently. `readback audit` detects this and says so instead
of reporting a clean result, but detection is all it can do.

**If your app has no cleanup step, this has nothing to offer you.** The other
stages need acoustic evidence and are measurably noisier; see
[docs/benchmark.md](benchmark.md) before turning any of them on. Being told that
up front is better than finding out after you have merged something.

## Step 1: check it against your own history, five minutes

Most dictation apps keep a history with both the raw transcript and the polished
result. Export it as JSON Lines — one object per line:

```json
{"raw":"never merge a change like this","cleaned":"Merge a change like this."}
{"raw":"um so we need to ship it tomorrow","cleaned":"We need to ship it tomorrow."}
```

Then:

```bash
cargo install readback-cli
readback audit --pairs history.jsonl
```

```
  7 pairs audited
  4 had their meaning changed by the cleanup step (57.1%)

  changed_environment        1
  changed_number             1
  changed_temporal           1
  dropped_negation           2

  examples

  line 1
    said     never merge a change like this
    became   Merge a change like this.
    restored never Merge a change like this.
  critical dropped_negation — "never"
      cleanup dropped "never"; restored from the raw transcript
```

Nothing is sent anywhere. No dependency is added. If the count is zero, your
cleanup step is behaving and you can close this document.

Field names are configurable: `--raw-field transcript --cleaned-field polished`.
Unparseable lines are skipped rather than aborting the run. `-` reads stdin.
Exit code is 1 when there are findings, so it drops into CI.

### Tuning it before you judge it

Two flags matter for a fair test:

```bash
readback audit --pairs history.jsonl \
  --locale en --locale hinglish \
  --vocab Klaviyo --vocab Recharge --vocab prod
```

`--vocab` is your users' dictionary: project names, environments, people. A
cleanup step mangling those is a real defect, and without the list the guard
cannot know they matter. `--locale hinglish` loads romanised Hindi negations
(`nahi`, `mat`), which nothing else in this category handles.

Run `readback lexicon list` to see exactly which words are protected. If
something is being protected that should not be, that list is where to look.

## Step 2: wire it in

### Rust

```toml
[dependencies]
readback-core = "0.14"
```

The guard on its own, which is all most apps want:

```rust
use readback_core::{check_cleanup, Lexicon, Locale};

let mut lexicon = Lexicon::new(&[Locale::En]);
lexicon.protect(user_dictionary);           // whatever your app already has

let outcome = check_cleanup(&raw_transcript, &polished, &lexicon);

insert(&outcome.text);                       // reverted spans already restored
if outcome.reverted {
    show_marks(&outcome.text, &outcome.flags);
}
```

`outcome.text` is the polished text with any meaning-changing edit undone. When
nothing protected was touched it is `polished` byte for byte, so the common path
is unchanged and you can ship this without touching your UI at all.

`outcome.flags` carries a byte span, a severity and a human-readable reason per
finding, if you want to mark them.

### Node

```bash
npm install @readback/core
```

```js
import { checkCleanup } from '@readback/core'

const { text, reverted, flags } = checkCleanup(rawTranscript, polished, {
  locales: ['en'],
  vocabulary: userDictionary,
})
```

Note that `flag.start` and `flag.end` are **byte** offsets into `text`, not
JavaScript string indices. See [docs/node.md](node.md).

This is a native addon, so it works in Electron and Node but not in a browser
build. If you need the browser, say so on the issue tracker — a WASM binding is
straightforward but has not been built because nobody has needed it yet.

### The rest of the pipeline, if you want it

`Readback::check` runs the full thing: acoustic suspicion, stakes scoring and
per-app policy on top of the guard, returning `pass` / `highlight` / `hold`.
It is worth reading [docs/benchmark.md](benchmark.md) first. The short version:
the guard is strong, and the acoustic stages are a real improvement over nothing
while still being too noisy to turn on by default.

## Step 3: what to expect in review

Things a maintainer reasonably asks, answered ahead of time:

**Does it slow anything down?** The guard is token alignment over one sentence.
No model, no network, no allocation of consequence.

**Does it change output when nothing is wrong?** No. When no protected token was
touched, the polished text is returned unchanged, byte for byte. There is a test
for exactly this.

**What about false positives?** Zero on the bundled benchmark's 26 controls. The
one class that used to fire — a cleanup step rewriting `three` as `3` — was
fixed in 0.6.0 by comparing numbers by value. `20 percent` and `20%` also
compare equal.

**What is the licence?** MIT. It imports cleanly into GPL and AGPL projects.

**What are the dependencies?** `serde`, `serde_json`, `similar`, `thiserror`.
No network, no audio, no models.

**What does it not do?** It cannot catch a word your recogniser got confidently
wrong in a way that still reads correctly, and it cannot see a word that was
never transcribed at all. Both limits are documented in the README rather than
discovered later.
