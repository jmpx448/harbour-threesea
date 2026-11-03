# 0002 — Own Rust 1.95 toolchain inside the Sailfish SDK

**Context.** libthreema (pinned upstream 6.5.3-1216) requires Rust 1.95 / edition 2024. SFOS 5.2's
build target ships Rust **1.75** (`rust-1.75.0+git2`). `github.com/sailfishos/rust` has a 1.95 tag
(2026-10-01) but no released SFOS ships it yet. The platform SDK userland is 32-bit x86.

**Decision.** Build a derived image (`scripts/sdk/Dockerfile`) = `coderus/sailfishos-platform-sdk`
+ rustup **i686** host toolchain 1.95.0 with the aarch64/armv7hf/i686 std. `scripts/build-core.sh`
cross-compiles the core to a static library using Jolla's `/opt/cross` gcc + binutils and the
target sysroot (rustc's default `-fuse-ld=lld` is filtered out). `mb2 build` then only runs qmake
and links the prebuilt `.a` (the spec does not run cargo).

**Consequences.** Works with Docker only, no SDK installer. The spec cannot rebuild Rust on its
own (no OBS/Chum build yet). When SFOS ships Rust ≥ 1.95, move cargo into `%build` and drop the
custom image. Note: linking against the 5.2 sysroot pulls in `GLIBC_2.39` symbol versions, so the
build target defines the minimum OS (see open question in CLAUDE.md §11).
