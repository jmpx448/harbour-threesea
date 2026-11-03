# SPDX-License-Identifier: AGPL-3.0-only
# Sailfish OS platform SDK + the Rust toolchain libthreema needs (SFOS ships 1.75; we need 1.95).
# The SDK userland is 32-bit x86, so the host toolchain is i686.
# Build: scripts/sdk-build.sh builds this image automatically. CI pulls the base image by its
# digest first (.github/workflows), so the tag below resolves to that exact image.
ARG SFOS_VERSION=5.1.0.11
FROM coderus/sailfishos-platform-sdk:${SFOS_VERSION}
ARG SFOS_VERSION

ARG RUST_VERSION=1.95.0
# rustup-init pinned with its checksum (security audit 2026-10-27).
ARG RUSTUP_VERSION=1.29.2
ARG RUSTUP_SHA256=01c158bbee6bf70a57bd2642349b6f70e0be8b98b366fc4bbe38dfe7f82e1acb
ENV RUSTUP_HOME=/opt/rust/rustup CARGO_HOME=/opt/rust/cargo
USER root
RUN zypper --non-interactive in libatomic >/dev/null && mkdir -p /opt/rust && \
    curl -sSfL https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/i686-unknown-linux-gnu/rustup-init -o /tmp/rustup-init && \
    echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - && \
    chmod +x /tmp/rustup-init && \
    /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-host i686-unknown-linux-gnu --default-toolchain ${RUST_VERSION} \
        --target aarch64-unknown-linux-gnu,armv7-unknown-linux-gnueabihf,i686-unknown-linux-gnu && \
    rm /tmp/rustup-init && chmod -R a+rX /opt/rust
ENV PATH=/opt/rust/cargo/bin:$PATH
USER mersdk
# Target-side libraries the Rust core links against outside of mb2 (SQLCipher needs libcrypto).
RUN for t in aarch64 armv7hl i486; do \
      sb2 -t SailfishOS-${SFOS_VERSION}-$t -m sdk-install -R zypper --non-interactive in \
        openssl-devel >/dev/null || exit 1; \
    done
