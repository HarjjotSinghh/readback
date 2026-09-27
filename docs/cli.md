# The `readback` CLI

Install from the workspace:

```bash
cargo install --path crates/readback-cli
```

The binary is `readback`. It runs the same pipeline the library does, so
anything it reports is what a host application would see.

## Commands

| Command             | Purpose                                                        |
| ------------------- | -------------------------------------------------------------- |
| `readback check`    | Run the pipeline and report a verdict.                          |
| `readback diff`     | Show only what the Cleanup Guard changed, and why.              |
| `readback explain`  | Show the arithmetic behind a verdict, for tuning thresholds.    |
| `readback lexicon`  | Inspect the protected lexicon.                                  |

## `check`

```bash
readback check --text "never merge a change like this" \
               --cleaned-text "Merge a change like this."
```

```
   HOLD   risk 0.94   stakes 0.90   suspicion 0.00

  text     never Merge a change like this.
  engine   text
  guard    ran, reverted a span
  scorer   rules

  1 flag(s)
  critical dropped_negation — "never"
      cleanup dropped "never"; restored from the raw transcript
```

### Exit codes

`check` exits with the verdict, so a shell script can branch without parsing
anything:

| Code | Meaning                                    |
| ---- | ------------------------------------------ |
| 0    | `pass`                                     |
| 1    | `highlight`                                |
| 2    | `hold`                                     |
| 70   | the command itself failed (bad input, IO)  |

```bash
if readback check --raw asr.json --cleaned-text "$polished" --quiet; then
  paste "$polished"
else
  confirm_with_user
fi
```

Pass `--exit-zero` when you only want the report and always want a success
status — useful in CI, where a held verdict is data rather than a failure.

### Input

| Flag              | Meaning                                                     |
| ----------------- | ----------------------------------------------------------- |
| `--raw <FILE>`    | Transcript file, or `-` for stdin.                           |
| `--text <STRING>` | Transcript as a literal string.                              |
| `--from <ENGINE>` | How to parse `--raw`. Defaults to `auto`.                    |
| `--cleaned <FILE>`| The LLM-polished rewrite, or `-` for stdin.                  |
| `--cleaned-text`  | The polished rewrite as a literal string.                    |

`--from auto` identifies the engine from the shape of the JSON: a
`transcription` key means whisper.cpp, `segments` (or a bare array) means
faster-whisper, `hypotheses` means Parakeet/NeMo, a lone `text` key means the
generic adapter, and anything that is not JSON is treated as plain text.

```bash
whisper-cli -f clip.wav --output-json-full -of out
readback check --raw out.json --cleaned-text "$(cat polished.txt)"

# or stream it
python transcribe.py | readback check --raw - --from faster-whisper
```

### Configuration

| Flag                  | Meaning                                                   |
| --------------------- | --------------------------------------------------------- |
| `--config <FILE>`     | A JSON config file matching `readback_core::Config`.       |
| `--locale <LOCALE>`   | `en` or `hinglish`. Repeatable. Replaces the default set.  |
| `--vocab <TERM>`      | An always-protected term. Repeatable.                      |
| `--vocab-file <FILE>` | Protected terms, one per line. `#` comments are ignored.   |
| `--app <NAME>`        | Frontmost application, used to select a policy.            |
| `--recommended`       | Use the built-in per-app routing.                          |

```bash
readback check --raw asr.json --cleaned-text "$polished" \
  --app Ghostty --recommended \
  --locale en --locale hinglish \
  --vocab Klaviyo --vocab Recharge
```

A config file looks like this:

```json
{
  "locales": ["en", "hinglish"],
  "vocabulary": ["prod", "staging", "Recharge"],
  "policy": {
    "default": { "hold": 0.7, "highlight": 0.35 },
    "apps": [
      { "pattern": "Terminal|Ghostty|Cursor", "hold": 0.3, "highlight": 0.15 },
      { "pattern": "Notes|Obsidian", "hold": 1.01, "highlight": 0.5 }
    ]
  },
  "suspicion": {
    "low_confidence": 0.55,
    "max_no_speech": 0.6,
    "max_compression_ratio": 2.4,
    "min_avg_logprob": -1.0
  },
  "risk": { "floor": 0.4 }
}
```

### JSON output

`--json` prints the `Verdict` verbatim, which is the same structure the library
returns:

```bash
readback check --text "never merge this" --cleaned-text "Merge this." --json
```

```json
{
  "action": "hold",
  "text": "never Merge this.",
  "flags": [
    {
      "kind": "dropped_negation",
      "severity": "critical",
      "span": { "start": 0, "end": 5 },
      "evidence": "cleanup dropped \"never\"; restored from the raw transcript",
      "suggestion": "never"
    }
  ],
  "risk": 0.94,
  "stakes": 0.9,
  "suspicion": 0.0,
  "provenance": {
    "cleanup_guard_ran": true,
    "cleanup_reverted": true,
    "scorer": "rules"
  }
}
```

Spans are **byte offsets into `text`**, so a host application can highlight them
without re-tokenising.

## `diff`

Runs the Cleanup Guard alone. Use it to decide whether your polish prompt is the
thing breaking your transcripts.

```bash
readback diff --text "deploy this to staging" \
              --cleaned-text "Deploy this to production."
```

```
  raw      deploy this to staging
  cleaned  Deploy this to production.
  final    Deploy this to staging.

  the guard reverted:
  critical changed_environment — "staging"
      cleanup rewrote "staging" as "production"; restored from the raw transcript
```

Exits 1 when something was reverted, 0 when the polish was kept intact.

## `explain`

Same inputs as `check`, but prints how the risk number was assembled. Use it
when a verdict surprises you, or when calibrating thresholds for an app.

```bash
readback explain --text "never merge this" --cleaned-text "Merge this." \
                 --app Ghostty --recommended
```

```
  evidence
    suspicion              0.000   how shaky the recognition looked
    semantic severity      1.000   worst meaning-changing edit
    max(evidence)          1.000

  stakes
    stakes                 0.900   scorer: rules
    floor                  0.400   risk that applies regardless of stakes
    scale = floor + (1-floor)*stakes
                           0.940

  risk
    risk = max(evidence) * scale
                           0.940

  policy
    app                    Ghostty
    highlight at           0.150
    hold at                0.300
    decision               Hold
```

Read it as: evidence is the *worst* signal available, and stakes *scale* it.
That is why a shaky word in "lol sounds good" passes while the same shakiness in
"don't delete production" holds.

## `lexicon`

```bash
readback lexicon classify never staging fifteen laptop
```

```
  never                negation         stakes weight 1.00
  staging              environment      stakes weight 0.85
  fifteen              number           stakes weight 0.70
  laptop               unprotected
```

```bash
readback lexicon list --class negation --locale en --locale hinglish
```

Lists every word currently loaded for a class, including the terms added by
`--vocab` and `--vocab-file`. `list` with no `--class` prints every class, plus
the destructive verbs that raise stakes without being protected themselves.

This is the fastest way to debug a false positive: if a word is being protected
that shouldn't be, it will be in one of these lists.

## Colour

Colour is on when stdout is a terminal, off when piped, and off whenever
`NO_COLOR` is set. Override with `--color always` or `--color never`.
