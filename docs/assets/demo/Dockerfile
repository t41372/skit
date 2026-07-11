# Hermetic recording environment for the skit demo videos.
#
# Base is VHS's own image (bundles vhs + ttyd + ffmpeg + JetBrains Mono / Nerd Fonts), so
# none of that heavy tooling ever touches your host — the whole point of recording in a
# container. We add just uv (copied from its official image, no curl-pipe-sh) and skit.
#
# The image is only the ENVIRONMENT. The tape and the per-language demo scripts are mounted
# in at record time (see scripts/record_demo.sh), so switching languages / editing a script
# never needs a rebuild. `mirror off` is pinned so a bare `skit` never hits the first-run
# network probe/prompt.
#
# Pin both bases to a digest once the first build succeeds (docker inspect --format
# '{{index .RepoDigests 0}}'); :latest here only so the very first build can't fail on a
# guessed tag.
FROM ghcr.io/charmbracelet/vhs:latest

# uv, straight from the official image — no install script executed.
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /usr/local/bin/

ENV PATH="/root/.local/bin:${PATH}"

# Install skit from source (this repo is the build context — see scripts/record_demo.sh).
# Only the packaging inputs are copied so an edit to tests/docs doesn't bust this layer.
COPY pyproject.toml README.md LICENSE /src/
COPY src /src/src
RUN uv tool install --python 3.12 /src \
    && uv python install 3.12          # cache a Python so `skit run` needs no network at record time

# Fancy recording shell + i18n glyphs (late layer, so the skit-install layer stays cached):
#  - bat: syntax-highlighted source display. Debian ships the binary as `batcat` (name
#    clash with another package), so symlink it back to `bat`.
#  - fonts-noto-cjk: the zh recordings need real Han glyphs or every character is a tofu box
#    (JetBrains Mono covers Latin; chromium falls back to Noto CJK for Han codepoints).
#  - demo.bashrc: colored prompt + bat defaults, appended to root's bashrc (the tape sources it).
RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-noto-cjk bat \
    && ln -sf /usr/bin/batcat /usr/local/bin/bat \
    && rm -rf /var/lib/apt/lists/*
COPY docs/assets/demo/demo.bashrc /tmp/demo.bashrc
RUN cat /tmp/demo.bashrc >> /root/.bashrc

# Pin the mirror off so a bare `skit` never triggers the first-run network probe/prompt.
RUN skit config mirror off

# The demo runs from /demo; record_demo.sh mounts the tape + the language's scripts in here.
WORKDIR /demo

# VHS's entrypoint is `vhs`; scripts/record_demo.sh passes the tape path as the argument.
