# readback-bench

CriticalSpeechBench and the Critical Semantic Error Rate, for
[Readback](https://github.com/HarjjotSinghh/readback).

Word error rate treats dropping "um" and dropping "not" as the same mistake.
CSER weights each error by how much meaning the word carried, so a dropped
negation costs fifty times a dropped filler.

```bash
cargo run --bin readback-bench -- run
```

```
                                 baseline   readback
  critical semantic error rate      0.328      0.080
  silent meaning flips              39.0%       0.0%
```

v0 is a text-level benchmark: the recogniser errors are hand-written rather than
produced from audio, so it measures how a reliability layer responds to a given
error, not how often that error occurs.

Full documentation:
[docs/benchmark.md](https://github.com/HarjjotSinghh/readback/blob/main/docs/benchmark.md).
