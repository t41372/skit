#!/usr/bin/env bash
# Record the walkthrough tapes: every screen the TUI can reach, in three languages.
#
# Requires Docker or OrbStack on the host. vhs / ttyd / ffmpeg live only inside the image,
# so nothing is installed on your machine. Each tape drives every locale: SKIT_LANG sits at
# the top of skit's locale chain, and each language's demo scripts are mounted at record
# time, so nothing is baked and no rebuild is needed to iterate on tapes or scripts.
#
# scripts/record_demo.sh renders the tracked README assets from demo.tape and shots.tape.
# This script is different: its frames are NOT tracked. Some frames raster one beat late
# while the terminal itself is correct, so a committed frame would fail for a reason that
# has nothing to do with skit. Read these frames, do not diff them against a baseline.
#
#   bash docs/assets/demo/record_walkthrough.sh          # everything
#   bash docs/assets/demo/record_walkthrough.sh shots    # one screenshot per TUI state
#   bash docs/assets/demo/record_walkthrough.sh story    # the continuous take
#   bash docs/assets/demo/record_walkthrough.sh modals   # pickers, editors, and guards
#   bash docs/assets/demo/record_walkthrough.sh drafts   # the kept-draft path
#   bash docs/assets/demo/record_walkthrough.sh clip     # the short-terminal clip evidence
#
# A second argument sets the output directory. It defaults to a path outside the repository,
# so a recording can never leave an untracked file in the working tree.
set -euo pipefail
cd "$(dirname "$0")/../../.."   # repo root (build context)

MODE="${1:-all}"
OUT="${2:-${TMPDIR:-/tmp}/skit-walkthrough}"
IMAGE=skit-walkthrough

echo "==> building demo image (VHS + Rust skit + uv)…"
docker build -f docs/assets/demo/Dockerfile -t "$IMAGE" .

# --network none proves the recording never reaches the network. The image pins
# `skit config mirror off` and every demo script is dependency free.
record() {   # $1 = tape file   $2 = SKIT_LANG   $3 = scripts subdir   $4 = output subdirectory
  local dir="$OUT/$4"
  echo "==> recording $1  (SKIT_LANG=$2) -> $dir"
  mkdir -p "$dir"
  rm -f "$dir"/*.png "$dir"/*.mp4
  docker run --rm --network none -e "SKIT_LANG=$2" \
    -v "$dir:/out" \
    -v "$PWD/docs/assets/demo/$1:/tape/tape.tape:ro" \
    -v "$PWD/docs/assets/demo/scripts/$3/greet.py:/demo/greet.py:ro" \
    -v "$PWD/docs/assets/demo/scripts/$3/banner.py:/demo/banner.py:ro" \
    -v "$PWD/docs/assets/demo/scripts/$3/names.txt:/demo/names.txt:ro" \
    -v "$PWD/docs/assets/demo/scripts/$3/pep723.py:/demo/pep723.py:ro" \
    "$IMAGE" /tape/tape.tape
}

# Record the screenshot tape twice and compare. Two runs of one tape are the only way to
# tell a stable frame from one the capture stack rasterises a beat late.
compare_runs() {
  local a="$OUT/shots-en" b="$OUT/shots-en-again" name
  echo "==> comparing two runs of walkthrough-shots.tape [en]"
  for frame in "$a"/*.png; do
    name="$(basename "$frame")"
    if cmp -s "$frame" "$b/$name"; then
      echo "    stable    $name"
    else
      echo "    late      $name  (capture artifact; check the text, not the bytes)"
    fi
  done
}

if [[ "$MODE" == "all" || "$MODE" == "shots" ]]; then
  record walkthrough-shots.tape  en    en    shots-en
  record walkthrough-shots.tape  en    en    shots-en-again
  record walkthrough-shots.tape  zh-CN zh-CN shots-zh-CN
  record walkthrough-shots.tape  zh-TW zh    shots-zh-TW
  compare_runs
fi
if [[ "$MODE" == "all" || "$MODE" == "story" ]]; then
  record walkthrough.tape        en    en    walkthrough-en
  record walkthrough.tape        zh-CN zh-CN walkthrough-zh-CN
  record walkthrough.tape        zh-TW zh    walkthrough-zh-TW
fi
if [[ "$MODE" == "all" || "$MODE" == "modals" ]]; then
  record walkthrough-modals.tape en    en    modals-en
  record walkthrough-modals.tape zh-CN zh-CN modals-zh-CN
  record walkthrough-modals.tape zh-TW zh    modals-zh-TW
fi
if [[ "$MODE" == "all" || "$MODE" == "drafts" ]]; then
  record walkthrough-drafts.tape en    en    drafts-en
fi
if [[ "$MODE" == "all" || "$MODE" == "clip" ]]; then
  record settings-clip.tape      en    en    settings-clip-en
fi

echo "==> done. Frames are in $OUT"
