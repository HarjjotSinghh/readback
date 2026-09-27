# readback-cli

Command-line interface for [Readback](https://github.com/HarjjotSinghh/readback),
a local-first reliability layer for speech-to-text.

```bash
cargo install readback-cli
```

```bash
readback check --text "never merge a change like this" \
               --cleaned-text "Merge a change like this."
# HOLD — critical: dropped_negation
```

Exits 0 for pass, 1 for highlight, 2 for hold, so shell scripts can branch on
the verdict directly.

Commands: `check`, `diff`, `explain`, `lexicon`. Full documentation is in
[docs/cli.md](https://github.com/HarjjotSinghh/readback/blob/main/docs/cli.md).
