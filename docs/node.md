# The Node binding

```bash
npm install @readback/core
```

Prebuilt binaries are published for macOS (x64, arm64), Linux (x64, arm64) and
Windows (x64). The package is a native addon built with
[napi-rs](https://napi.rs); there is no runtime dependency on Rust.

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

TypeScript definitions ship with the package, generated from the Rust source, so
the doc comments you see in your editor are the ones in the crate.

## `new Readback(options?)`

Build one and reuse it; construction loads the lexicon.

| Option        | Type                | Meaning                                                       |
| ------------- | ------------------- | ------------------------------------------------------------- |
| `locales`     | `string[]`          | `'en'`, `'hinglish'`. Defaults to `['en']`.                    |
| `vocabulary`  | `string[]`          | Always-protected terms: names, projects, environments, jargon. |
| `recommended` | `boolean`           | Use the built-in per-app routing.                              |
| `apps`        | `JsAppPolicy[]`     | Per-app overrides, checked in order. First match wins.         |
| `hold`        | `number`            | Default hold threshold for apps with no rule.                  |
| `highlight`   | `number`            | Default highlight threshold for apps with no rule.             |

```js
const rb = new Readback({
  locales: ['en', 'hinglish'],
  vocabulary: ['Klaviyo', 'Recharge', 'prod'],
  recommended: true,
  apps: [
    { pattern: 'Ghostty|Terminal|Cursor', hold: 0.3, highlight: 0.15 },
    { pattern: 'Obsidian|Notes', hold: 1.01, highlight: 0.5 },
  ],
})
```

Rules you pass in `apps` are checked before the recommended set, so they
override it. A `hold` above `1` never holds, since risk is clamped to `1`.

An unknown locale throws, rather than being silently ignored.

## `rb.check(input)`

Only `text` is required. Every other field turns on another stage, so adoption
can be incremental: ship the Cleanup Guard today, add word confidence later.

| Field          | Type                  | Turns on                                        |
| -------------- | --------------------- | ----------------------------------------------- |
| `text`         | `string`              | —  (the recogniser's output)                     |
| `cleaned`      | `string`              | the Cleanup Guard                                |
| `words`        | `JsWord[]`            | acoustic suspicion                               |
| `alternatives` | `string[]`            | cross-engine disagreement                        |
| `signals`      | `JsSignals`           | decoder hallucination tells                      |
| `speech`       | `JsSpeechRegion[]`    | omission detection (needs `words` with timings)  |
| `app`          | `string`              | per-app policy routing                           |
| `vocabulary`   | `string[]`            | extra protected terms, for this call only        |

```js
const verdict = rb.check({
  text: asr.text,
  words: asr.words.map((w) => ({
    text: w.word,
    startMs: Math.round(w.start * 1000),
    endMs: Math.round(w.end * 1000),
    confidence: w.probability,
  })),
  signals: {
    avgLogprob: asr.avg_logprob,
    noSpeechProb: asr.no_speech_prob,
    compressionRatio: asr.compression_ratio,
  },
  cleaned: polished,
  app: frontmostApp,
})
```

### The verdict

```ts
{
  action: 'pass' | 'highlight' | 'hold'
  text: string        // insert this; reverted spans are already restored
  flags: JsFlag[]
  risk: number        // 0..1
  stakes: number      // 0..1, how costly a wrong word would be here
  suspicion: number   // 0..1, how shaky the recognition looked
  provenance: { cleanupGuardRan, cleanupReverted, omissionCheckRan, scorer, primaryProvider }
}
```

```js
switch (verdict.action) {
  case 'pass':
    paste(verdict.text)
    break
  case 'highlight':
    pasteWithMarks(verdict.text, verdict.flags)
    break
  case 'hold':
    await confirm(verdict)
    break
}
```

### Flag spans are byte offsets

`flag.start` and `flag.end` are **byte** offsets into `verdict.text`, not
JavaScript string indices. For ASCII they are the same; for anything else they
are not. Slice through a Buffer:

```js
const bytes = Buffer.from(verdict.text, 'utf8')
const word = bytes.subarray(flag.start, flag.end).toString('utf8')
```

To convert a byte offset to a UTF-16 index for something like
`Range.setStart`, decode the prefix:

```js
const charIndex = bytes.subarray(0, flag.start).toString('utf8').length
```

### Flag kinds

`dropped_negation`, `altered_negation`, `changed_temporal`, `changed_number`,
`changed_direction`, `changed_quantifier`, `changed_modality`,
`changed_environment`, `changed_protected_term`, `low_confidence`,
`hallucination_signal`, `possible_omission`.

Severities: `critical`, `high`, `medium`, `low`, `none`. The strings match the
Rust JSON and the CLI output exactly, so the three can be compared directly.

## `checkCleanup(raw, cleaned, options?)`

The Cleanup Guard on its own, for sanitising an LLM rewrite without running the
rest of the pipeline.

```js
import { checkCleanup } from '@readback/core'

const { text, reverted, flags } = checkCleanup(
  'deploy this to staging',
  'Deploy this to production.',
)
// text: 'Deploy this to staging.'   reverted: true
```

When nothing protected was touched, `cleaned` comes back byte for byte and
`reverted` is `false`.

## Building from source

```bash
cd bindings/node
npm install
npm run build     # napi build --platform --release
npm test
```

The addon is built per platform and published as an npm artifact; it is not
committed to the repository.
