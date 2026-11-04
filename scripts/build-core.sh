#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-only
# Build the Rust core (and libthreema) as a static library for one Sailfish architecture.
# Runs *inside* the Sailfish platform SDK (container), where Jolla's cross gcc and target sysroots
# live, using the rustup toolchain pinned in rust-toolchain.toml (SFOS ships Rust 1.75, libthreema
# needs 1.95 - see docs/decisions/0002-rust-toolchain.md). Normally called via scripts/sdk-build.sh.
#
# Usage: scripts/build-core.sh <aarch64|armv7hl|i486>
set -eu
ARCH=${1:?usage: $0 <aarch64|armv7hl|i486>}
SFOS=${SFOS_VERSION:-5.1.0.11}
ROOT=$(cd "$(dirname "$0")/.." && pwd)
SYSROOT=/srv/mer/targets/SailfishOS-$SFOS-$ARCH

case "$ARCH" in
  aarch64) TRIPLE=aarch64-unknown-linux-gnu;     GCC=aarch64-meego-linux-gnu-gcc ;;
  armv7hl) TRIPLE=armv7-unknown-linux-gnueabihf; GCC=armv7hl-meego-linux-gnueabi-gcc ;;
  i486)    TRIPLE=i686-unknown-linux-gnu;        GCC=i486-meego-linux-gnu-gcc ;;
  *) echo "unknown arch $ARCH" >&2; exit 1 ;;
esac

# Linker / C compiler for the target, pointed at the SFOS sysroot and Jolla's binutils. Recent
# rustc asks gcc for its bundled rust-lld (-fuse-ld=lld, -B.../gcc-ld); drop that so the SFOS
# binutils do the linking.
WRAP="$ROOT/target/cross-$ARCH-cc"
# gcc/collect2 look for plain "as"/"ld" in -B dirs, so give them the target binutils.
LDDIR="$ROOT/target/cross-$ARCH-bin"
mkdir -p "$LDDIR"
for tool in as ld; do ln -sf "/opt/cross/bin/${GCC%gcc}$tool" "$LDDIR/$tool"; done
cat > "$WRAP" <<WRAPPER
#!/bin/sh
for a in "\$@"; do
  shift
  case "\$a" in -fuse-ld=lld|-B*/gcc-ld) ;; *) set -- "\$@" "\$a" ;; esac
done
exec /opt/cross/bin/$GCC -B$LDDIR/ --sysroot=$SYSROOT "\$@"
WRAPPER
chmod +x "$WRAP"

TRIPLE_ENV=$(echo "$TRIPLE" | tr 'a-z-' 'A-Z_')
export "CARGO_TARGET_${TRIPLE_ENV}_LINKER=$WRAP"
TRIPLE_US=$(echo "$TRIPLE" | tr '-' '_')
export "CC_$TRIPLE_US=$WRAP"
export "AR_$TRIPLE_US=/opt/cross/bin/${GCC%gcc}ar"
# Bundled SQLCipher is the core's C code: the hardening flags the Rust around it doesn't need.
export "CFLAGS_$TRIPLE_US=-fstack-protector-strong -D_FORTIFY_SOURCE=2"
# It builds against the target's OpenSSL (libcrypto).
case "$ARCH" in aarch64) LIBDIR=usr/lib64 ;; *) LIBDIR=usr/lib ;; esac
export OPENSSL_INCLUDE_DIR="$SYSROOT/usr/include" OPENSSL_LIB_DIR="$SYSROOT/$LIBDIR"
export PROTOC="${PROTOC:-$("$ROOT/scripts/fetch-protoc.sh")}"
# The SDK image ships exactly this toolchain (scripts/sdk/Dockerfile); do not let rustup fetch.
export RUSTUP_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.95.0}"
export CARGO_HOME="${CARGO_HOME_OVERRIDE:-$ROOT/target/cargo-home}"

cd "$ROOT"
# The emulator build (and only that one) carries the made-up demo chats, `demoData`: no x86
# device runs Sailfish OS, so this never reaches a phone. See core/src/demo.rs.
case "$ARCH" in i486) FEATURES="--features demo" ;; *) FEATURES="" ;; esac
# shellcheck disable=SC2086 # FEATURES is empty or one flag pair
cargo build --locked --release --target "$TRIPLE" -p threesea-core $FEATURES
echo "$ROOT/target/$TRIPLE/release/libthreesea_core.a"
