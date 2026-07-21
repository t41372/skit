#!/usr/bin/env bash
# Render the skit demo assets (English + 繁體中文) in a hermetic container: the README
# videos and the four-screen TUI screenshot grid, all from one image and one set of tapes.
#
# Requires Docker or OrbStack on the host. vhs / ttyd / ffmpeg live only inside the image,
# so nothing is installed on your machine. Each tape drives every locale: SKIT_LANG sits at
# the top of skit's locale chain, and each language's demo scripts are mounted at record
# time, so nothing is baked and no rebuild is needed to iterate on tapes or scripts.
#
#   bash scripts/record_demo.sh          # everything: 2 videos + 8 screenshots
#   bash scripts/record_demo.sh videos   # docs/assets/demo-en.mp4, docs/assets/demo-zh.mp4
#   bash scripts/record_demo.sh shots    # docs/assets/tui-{library,form,add,settings}-{en,zh}.png
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root (build context)

MODE="${1:-all}"
IMAGE=skit-demo

echo "==> building demo image (vhs + uv + skit)…"
docker build -f docs/assets/demo/Dockerfile -t "$IMAGE" .

# The tape is mounted at /tape, NOT /demo: since the path picker browses the working
# directory on camera, /demo must hold only the demo's own scripts — a stray demo.tape in
# that listing shows the recording rig to the viewer.
run_tape() {   # $1 = SKIT_LANG   $2 = scripts subdir (en/zh)   $3 = tape file in docs/assets/demo/
  docker run --rm -e "SKIT_LANG=$1" \
    -v "$PWD/docs/assets:/out" \
    -v "$PWD/docs/assets/demo/$3:/tape/demo.tape:ro" \
    -v "$PWD/docs/assets/demo/scripts/$2/greet.py:/demo/greet.py:ro" \
    -v "$PWD/docs/assets/demo/scripts/$2/banner.py:/demo/banner.py:ro" \
    -v "$PWD/docs/assets/demo/scripts/$2/names.txt:/demo/names.txt:ro" \
    "$IMAGE" /tape/demo.tape
}

record() {   # $1 = SKIT_LANG   $2 = scripts subdir   $3 = output basename
  echo "==> recording docs/assets/$3  (SKIT_LANG=$1)…"
  run_tape "$1" "$2" demo.tape
  mv docs/assets/demo.mp4 "docs/assets/$3"
  echo "    wrote docs/assets/$3"
}

shoot() {   # $1 = SKIT_LANG   $2 = scripts subdir   $3 = filename suffix (en/zh)
  echo "==> screenshotting the TUI  (SKIT_LANG=$1)…"
  run_tape "$1" "$2" shots.tape
  rm -f docs/assets/shots.mp4   # VHS insists on a video output; only the PNGs matter here
  for shot in library form add settings; do
    mv "docs/assets/shot-$shot.png" "docs/assets/tui-$shot-$3.png"
    echo "    wrote docs/assets/tui-$shot-$3.png"
  done
}

if [[ "$MODE" == "all" || "$MODE" == "videos" ]]; then
  record en    en demo-en.mp4
  record zh-TW zh demo-zh.mp4
fi
if [[ "$MODE" == "all" || "$MODE" == "shots" ]]; then
  shoot en    en en
  shoot zh-TW zh zh
fi

cat <<'EOF'
==> done.

    Mouse-operability cameo (docs/assets/demo-mouse.gif) — VHS can't show a cursor, so it's
    hand-recorded separately (recipe in CONTRIBUTING.md, "The mouse-operability GIF").
EOF
