# Readback demo

The reference desktop app for
[Readback](https://github.com/HarjjotSinghh/readback).

```bash
cargo run
```

**No microphone.** This is a demo for the SDK, not a dictation product. It shows
the decision layer and the overlay UX — the part Readback owns — using scenarios
loaded from CriticalSpeechBench. Point the clip field at a WAV file to see
omission detection work on real audio via a small built-in energy VAD.

Outside the Cargo workspace on purpose, so Tauri's dependency tree does not slow
`cargo test` at the repository root.

Full notes:
[docs/demo.md](https://github.com/HarjjotSinghh/readback/blob/main/docs/demo.md).
