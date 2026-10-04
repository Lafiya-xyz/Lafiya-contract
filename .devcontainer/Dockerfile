# Reproducible Lafiya contract dev environment. Every download is pinned by
# version and verified by SHA-256; the base image is pinned by digest.
FROM mcr.microsoft.com/devcontainers/base:ubuntu-24.04@sha256:d94c97dd9cacf183d0a6fd12a8e87b526e9e928307674ae9c94139139c0c6eae

ARG STELLAR_CLI_VERSION=28.0.0
ARG STELLAR_CLI_SHA256=207544486734fccb4df1afc4a7745478f9f1e21688b2f9506f0ef36f60ce3fdc
ARG NODE_VERSION=22.23.3
ARG NODE_SHA256=df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de
ARG PNPM_VERSION=12.6.0
ARG NEXTEST_VERSION=0.9.146
ARG MUTANTS_VERSION=27.1.0
ARG FUZZ_VERSION=0.13.2
ARG JUST_VERSION=1.58.0

# System packages: build deps, Python 3.12 (Ubuntu 24.04 default, ships tomllib),
# and shellcheck for the smoke-test lint.
RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
    && apt-get install -y --no-install-recommends \
        build-essential pkg-config libssl-dev libdbus-1-dev libudev-dev \
        python3 python3-pip python3-venv shellcheck xz-utils curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Python tooling (pinned in requirements.txt).
COPY requirements.txt /tmp/requirements.txt
RUN pip3 install --break-system-packages --no-cache-dir -r /tmp/requirements.txt && rm /tmp/requirements.txt

# Node LTS + pnpm.
RUN curl -fsSL "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" -o /tmp/node.tar.xz \
    && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum -c - \
    && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && npm install -g "pnpm@${PNPM_VERSION}"

# stellar-cli (same pinned version as the CI docs; bump version + checksum together).
RUN curl -fsSL "https://github.com/stellar/stellar-cli/releases/download/v${STELLAR_CLI_VERSION}/stellar-cli-${STELLAR_CLI_VERSION}-x86_64-unknown-linux-gnu.tar.gz" -o /tmp/stellar.tar.gz \
    && echo "${STELLAR_CLI_SHA256}  /tmp/stellar.tar.gz" | sha256sum -c - \
    && tar -xzf /tmp/stellar.tar.gz -C /usr/local/bin stellar \
    && rm /tmp/stellar.tar.gz

# Rust: the channel from rust-toolchain.toml plus its wasm target and
# components, a minimal nightly for cargo-fuzz, and the cargo tools.
USER vscode
ENV CARGO_HOME=/home/vscode/.cargo RUSTUP_HOME=/home/vscode/.rustup PATH=/home/vscode/.cargo/bin:$PATH
COPY --from=root rust-toolchain.toml /tmp/rust-toolchain.toml
RUN curl -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain none \
    && CHANNEL="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' /tmp/rust-toolchain.toml)" \
    && rustup toolchain install "$CHANNEL" --profile minimal --component rustfmt,clippy --target wasm32v1-none \
    && rustup default "$CHANNEL" \
    && rustup toolchain install nightly --profile minimal \
    && cargo install --locked \
        "cargo-nextest@${NEXTEST_VERSION}" \
        "cargo-mutants@${MUTANTS_VERSION}" \
        "cargo-fuzz@${FUZZ_VERSION}" \
        "just@${JUST_VERSION}" \
    && rm -rf "$CARGO_HOME/registry" "$CARGO_HOME/git"
