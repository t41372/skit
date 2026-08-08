FROM rust:1.97.1-bookworm AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
COPY skills ./skills
RUN cargo build --locked --release -p skit-cli-rs

FROM ghcr.io/charmbracelet/vhs:v0.11.0

COPY --from=ghcr.io/astral-sh/uv:0.12.3 /uv /uvx /usr/local/bin/
COPY --from=builder /src/target/release/skit /usr/local/bin/skit

RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-noto-cjk bat vim \
    && ln -sf /usr/bin/batcat /usr/local/bin/bat \
    && uv python install 3.14 \
    && rm -rf /var/lib/apt/lists/*

COPY docs/assets/demo/demo.bashrc /tmp/demo.bashrc
RUN cat /tmp/demo.bashrc >> /root/.bashrc
COPY docs/assets/demo/demo.vimrc /root/.vimrc

RUN skit config mirror off

WORKDIR /demo
