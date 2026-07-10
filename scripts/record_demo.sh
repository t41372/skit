#!/usr/bin/env bash
# Record the skit demo videos (English + 繁體中文) in a hermetic container.
#
# Requires Docker or OrbStack on the host. vhs / ttyd / ffmpeg live only inside the image,
# so nothing is installed on your machine. One tape drives both locales: SKIT_LANG sits at
# the top of skit's locale chain, and each language's demo scripts are mounted at record
# time, so nothing is baked and no rebuild is needed to iterate.
#
#   bash scripts/record_demo.sh
#
# Output: docs/demo-en.mp4, docs/demo-zh.mp4 (MP4 — pausable / seekable, unlike a GIF).
set -euo pipefail
cd "$(dirname "$0")/.."   # repo root (build context)

IMAGE=skit-demo

echo "==> building demo image (vhs + uv + skit)…"
docker build -f docs/demo/Dockerfile -t "$IMAGE" .

record() {   # $1 = SKIT_LANG   $2 = scripts subdir (en/zh)   $3 = output basename
  echo "==> recording docs/$3  (SKIT_LANG=$1)…"
  docker run --rm -e "SKIT_LANG=$1" \
    -v "$PWD/docs:/out" \
    -v "$PWD/docs/demo/demo.tape:/demo/demo.tape:ro" \
    -v "$PWD/docs/demo/scripts/$2/my_script_1.py:/demo/my_script_1.py:ro" \
    -v "$PWD/docs/demo/scripts/$2/my_script_2.py:/demo/my_script_2.py:ro" \
    -v "$PWD/docs/demo/scripts/$2/my_program.sh:/demo/my_program.sh:ro" \
    "$IMAGE" /demo/demo.tape
  mv docs/demo.mp4 "docs/$3"
  echo "    wrote docs/$3"
}

record en    en demo-en.mp4
record zh-TW zh demo-zh.mp4

cat <<'EOF'
==> done — docs/demo-en.mp4, docs/demo-zh.mp4

    Mouse-operability cameo (optional, separate): VHS can't show a cursor. Record a ~5s
    QuickTime clip clicking a footer chip + a table row and keep it as its own short clip.
EOF
