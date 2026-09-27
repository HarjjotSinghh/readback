#!/usr/bin/env bash
#
# Generates audio fixtures for CriticalSpeechBench by speaking every reference
# sentence in the text dataset.
#
# These are synthetic. Text-to-speech is clean, evenly paced and never mumbles,
# so an engine will score far better here than on real speech. Treat the numbers
# as a floor and a regression guard, not as a claim about field accuracy — and
# replace the clips with real recordings when you have them. The manifest format
# does not care where a clip came from.
#
# Usage:
#   scripts/make-fixtures.sh [output-dir]        # default: fixtures
#   READBACK_VOICE=Daniel scripts/make-fixtures.sh
#   READBACK_RATE=160 scripts/make-fixtures.sh   # words per minute
#
set -euo pipefail

OUT_DIR="${1:-fixtures}"
CLIP_DIR="$OUT_DIR/clips"
MANIFEST="$OUT_DIR/manifest.jsonl"
VOICE="${READBACK_VOICE:-Samantha}"
RATE="${READBACK_RATE:-175}"

if ! command -v say >/dev/null 2>&1; then
  echo "This script uses macOS 'say'. On Linux, espeak-ng is the closest" >&2
  echo "equivalent; adapt the speak() function below." >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$CLIP_DIR"

echo "Building the manifest from the text dataset..."
(cd "$REPO_ROOT" && cargo run -q --bin readback-bench -- manifest --clips clips) > "$MANIFEST"

COUNT=$(grep -c . "$MANIFEST")
echo "Speaking $COUNT references as $VOICE at $RATE wpm..."

# Read id and reference out of the manifest and speak each one.
python3 - "$MANIFEST" "$CLIP_DIR" "$VOICE" "$RATE" <<'PY'
import json, subprocess, sys, pathlib

manifest, clip_dir, voice, rate = sys.argv[1:5]
clip_dir = pathlib.Path(clip_dir)
made = skipped = 0

for line in pathlib.Path(manifest).read_text().splitlines():
    if not line.strip():
        continue
    case = json.loads(line)
    out = clip_dir / f"{case['id']}.wav"
    if out.exists():
        skipped += 1
        continue
    subprocess.run(
        ["say", "-v", voice, "-r", rate,
         "--data-format=LEI16@16000", "--file-format=WAVE",
         "-o", str(out), case["reference"]],
        check=True,
    )
    made += 1

print(f"  {made} clip(s) written, {skipped} already present")
PY

echo
echo "Fixtures ready: $CLIP_DIR"
echo
echo "Run the benchmark against them with an ASR of your choice, for example:"
echo
echo "  readback-bench audio --manifest $MANIFEST --vad \\"
echo "    --asr 'whisper-cli -f {wav} --output-json-full -oj -of /tmp/rb && cat /tmp/rb.json'"
echo
