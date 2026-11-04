# Building

## Pinned versions

| What | Version | Where |
|---|---|---|
| Upstream threema-android (libthreema) | tag `6.5.3-1216`, commit `d44a2b53b2ea2c0bf1065bdaa96987570e5b4e6d` | `third_party/threema-android` (shallow submodule) |
| Rust | 1.95.0 | `rust-toolchain.toml`, `scripts/sdk/Dockerfile` |
| protoc (for libthreema's prost-build) | 29.3, sha256 `3e866620…0021a` | `scripts/fetch-protoc.sh` |
| Sailfish OS build target | 5.1.0.11 (Qt 5.6.3, glibc 2.41) | `coderus/sailfishos-platform-sdk:5.1.0.11` |
| libthreema | vendored in `third_party/libthreema` + our commits (ADR 0006) | `scripts/update-libthreema.sh` |
| webrtc-audio-processing (Full: call echo cancellation, BSD) | tag `v2.1`, commit `846fe90a` | `third_party/webrtc-audio-processing` (submodule; built by the spec's `%build` with meson against SFOS's abseil-cpp, linked statically) |

## Prerequisites

Linux host with Docker (non-rootless, user in `docker` group), git, curl, unzip. For host-side
development also rustup. No Sailfish SDK installer needed.

```sh
git clone --recurse-submodules --shallow-submodules <repo>
cd harbour-threesea
```

## Host development (Rust core only)

```sh
export PROTOC=$(scripts/fetch-protoc.sh)
cargo test
cargo clippy --all-targets
cargo test -- --ignored              # live tests against the real Threema servers
```

All checks in one go (what CI is to run): `scripts/check.sh` (private data, logging rules,
format, clippy, tests, `cargo audit` if installed); `--full` adds libthreema's tests and the
tests in the SDK, `--quick` does only the first three. `scripts/test-lockstate.sh` runs the real
window QML on the real Silica with a fake backend and no window system (`tests/lockstate`): the
app lock, calls and the page stack, one scenario per process, no device or emulator needed. `git config core.hooksPath scripts/hooks` runs the quick checks on
what is about to be committed, before every commit. The private-data check reads
`.local/private-patterns` (local, ignored by git: one extended regex per line, e.g. test
credentials) and reports only file names, never the match.

## Device RPMs

```sh
scripts/sdk-build.sh                # aarch64 armv7hl i486
scripts/sdk-build.sh aarch64        # just one
scripts/sdk-build.sh                # Full flavour (default; background service, calls) into RPMS/full
FLAVOUR=lite scripts/sdk-build.sh   # Lite flavour (Jolla Store rules) into RPMS/lite
                                    # FLAVOUR=lite|full scripts/emulator-deploy.sh installs one
ls RPMS/
```

What it does (see docs/decisions/0002-rust-toolchain.md): builds the image
`harbour-threesea-sdk:5.1.0.11` once, cross-compiles `core` to
`target/<triple>/release/libthreesea_core.a` with Jolla's cross gcc, then runs `mb2 build` on a
copy of the tree inside the container and extracts the RPMs.

Harbour check:

```sh
docker run --rm -v $PWD/RPMS:/rpms:ro harbour-threesea-sdk:5.1.0.11 \
  sh -c 'cp /rpms/*.aarch64.rpm ~ && mb2 -t SailfishOS-5.1.0.11-aarch64 check ~/*.rpm'
```

## Install on a device

Enable developer mode, then `scp RPMS/harbour-threesea-*.aarch64.rpm defaultuser@<phone>:` and
`devel-su pkcon install-local harbour-threesea-*.rpm` (or `rpm -U`).

## Hardening

Full RELRO for the binary (`-Wl,-z,relro,-z,now`), and `-fstack-protector-strong
-D_FORTIFY_SOURCE=2` for all C and C++ we compile: the bridge and the AEC wrapper
(`harbour-threesea.pro`), webrtc-audio-processing (`rpm/harbour-threesea.spec`) and the bundled
SQLCipher (`scripts/build-core.sh`). Check a build with `readelf -d` on the binary (`BIND_NOW`, flag
`NOW`) and `readelf -l` (`GNU_RELRO`), `nm -D` for `__stack_chk_fail` and `__*_chk`, and `-O2` in
the bridge's compile lines of the build log. rpm's `%build` starts with `CFLAGS` and `CXXFLAGS`
exported as `%{optflags}`, and `%qmake5` uses them for the bridge: the spec adds to them for the
`meson setup` command alone. Replacing them loses `-O2` and the target's CPU flags, and the library
then doesn't compile on i486, whose SSE comes from there.

## Gotchas

- Opening the repo in RustRover while rustup installs the pinned toolchain can corrupt the
  install (both run rustup at once). If `cargo` errors with odd rename/"deleted" linker errors:
  `rustup toolchain uninstall 1.95` and reinstall with RustRover closed.
- Our workspace `exclude`s `third_party`, otherwise libthreema becomes an implicit member and
  inherits our (non-existent) lint table.

## Fuzzing (development only)

`core/fuzz/` holds coverage-guided fuzz targets for the decoders that take input from outside:
voice messages (`voice_decode`), call SDP (`sdp_patch`), Threema Web chunks and images
(`web_chunks`, `web_images`), scanned codes (`codes`), data-backup CSV (`backup_csv`) and
quotes/mentions (`message_text`). It is its own workspace and never part of the app.

```sh
cargo install cargo-fuzz            # once; needs the nightly toolchain
cd core
export PROTOC=$(../scripts/fetch-protoc.sh)
cargo +nightly fuzz run voice_decode fuzz/corpus/voice_decode fuzz/seeds/voice_decode \
    -- -max_total_time=600
```

`fuzz/seeds/` holds valid starting inputs (a voice message from our own encoder, an SDP offer,
a PNG); without them the fuzzer stalls at the container headers. The voice seed is rewritten by
`cargo test -p threesea-core --lib write_voice_seed -- --ignored`. Crashes land in
`fuzz/artifacts/` (ignored by git).
