# The reference app

```bash
cd apps/demo
cargo run
```

A small Tauri window that shows what Readback decides and what the overlay
should look like.

## What it is, and what it deliberately is not

**It has no microphone.** This is a demo for the SDK, not a dictation product.
Audio capture, global hotkeys, accessibility permissions and text injection are
the host application's job, and none of them teach you anything about the
reliability layer. What the app shows is the part Readback actually owns: the
decision, the flags, and the UX for surfacing them.

Scenarios are loaded from CriticalSpeechBench rather than hard-coded, so the
demo and the benchmark cannot drift apart. Selecting `neg-001` in the sidebar
shows exactly the case the benchmark scores.

## The window

```
┌────────────────┬──────────────────────────────────────────────┐
│ ‸ Readback     │  Heard     never merge a change like this    │
│ [filter…]      │  Polished  Merge a change like this.         │
│                │  App       Ghostty     Vocab  prod, Recharge │
│ SOMETHING      │                                              │
│ WENT WRONG     │  ┌────────────────────────────────────────┐  │
│ • neg-001 …    │  │ HOLD   Don't insert until confirmed.   │  │
│ • env-001 …    │  └────────────────────────────────────────┘  │
│ • num-001 …    │                                              │
│                │  never Merge a change like this.             │
│ CONTROLS       │  ─────                                       │
│ • ben-001 …    │  without Readback: Merge a change like this. │
│ • ben-007 …    │                                              │
│                │  risk 0.94   stakes 0.90   suspicion 0.00    │
│                │                                              │
│                │  CRITICAL dropped_negation                   │
│                │    cleanup dropped "never"; restored from…   │
└────────────────┴──────────────────────────────────────────────┘
```

Every field is editable. Type into **Heard** or **Polished** and the verdict
updates as you go, which is the fastest way to build intuition for what trips a
hold and what does not. The strikethrough line underneath shows what the stack
would have pasted without Readback in the loop.

The sidebar separates cases where something went wrong from the **controls**,
where the stack behaved. Click through the controls: a reliability layer that
holds them is unusable, and that is easier to feel than to read in a table.

## Loading a clip

The one thing text cannot show you is a word that was never transcribed. Point
the **WAV clip** field at a file and the app runs a small energy-based
voice-activity detector over it, then passes the speech regions to Readback the
way a real host would.

```
/path/to/clip.wav → 2400 ms at 16000 Hz — 1 speech region(s)
```

Gaps are only found where speech is *not* covered by a transcribed word, so word
timings are needed too. The bundled `omi-*` scenarios carry them.

The VAD is short-time energy with hysteresis, written in
[`src/vad.rs`](../apps/demo/src/vad.rs). It is deliberately simple so the demo
has no model to download, and it is nowhere near good enough for a noisy room —
a real host should use Silero or WebRTC VAD. Readback's core never touches audio
either way; it only ever receives regions.

There is no file picker, just a path field, so the app needs no dialog plugin.

## How the overlay is drawn

Worth copying, because two details are easy to get wrong:

**Flag spans are byte offsets.** `flag.start` and `flag.end` index into the
UTF-8 bytes of `verdict.text`, not into a JavaScript string. The app converts
through `TextEncoder`/`TextDecoder`; anything else breaks the moment someone
dictates a word with an accent in it.

**Zero-width spans are a different thing.** A flag whose start equals its end is
not a range to underline — it marks a position where a word appears to have gone
missing. The app draws a caret (`‸`) there instead of a highlight.

Severity drives colour, and overlapping flags resolve to the worst severity
covering each byte, so a critical flag is never hidden underneath a low one.

## Design notes

Tokens are shadcn/ui's default neutral theme (MIT), adapted to plain CSS custom
properties — there is no bundler, no framework and no build step for the
frontend. Motion is limited to a 160 ms meter transition; nothing animates in
response to a keystroke.

## Building

```bash
cd apps/demo
cargo run              # dev
cargo build --release  # release binary
```

The demo is **outside the Cargo workspace** on purpose: Tauri pulls a large
dependency tree, and keeping it separate means `cargo test` at the repository
root stays fast.

To produce a signed, bundled application, install the Tauri CLI and generate
platform icon formats first:

```bash
cargo install tauri-cli --version "^2"
cargo tauri icon icons/icon.png   # writes .icns and .ico
cargo tauri build
```
