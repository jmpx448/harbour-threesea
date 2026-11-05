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

All checks in one go: `scripts/check.sh` (private data, logging rules, format, clippy, tests, the
UI's commands and imports against the core and the Jolla Store's list (`scripts/check-qml.py`),
that the fuzz targets build, how many texts await translation, `cargo audit` if installed);
`--full` adds libthreema's own tests (`scripts/libthreema-tests.sh`) and every test in the SDK
(`scripts/run-sdk-tests.sh`: each `scripts/test-*.sh` in the SDK, the call engine's built only)
and the C ABI under AddressSanitizer (`scripts/test-ffi-asan.sh`, nightly), and fails without the
SDK image; `--release` fails while a text is missing from `translations/`;
`--quick` does only the first three. CI (`.github/workflows/ci.yml`) runs the default checks,
libthreema's tests, the ASan run, the SDK tests and builds of both flavours for all three architectures on
every push; `advisories.yml` runs `cargo audit` weekly too. `scripts/test-lockstate.sh` runs the real
window QML on the real Silica with a fake backend and no window system (`tests/lockstate`): the
app lock, calls and the page stack, one scenario per process, no device or emulator needed.
`scripts/test-pages.sh` opens every page and dialog under `qml/pages` the same way, for both
flavours, builds all rows of its lists, opens each row's menu, and fails on any warning from Qt or
the QML engine (`tests/pageload`; the fake C++ side is generated from the bridge's headers, the
fake core's answers are in `Results.js`). `git config core.hooksPath scripts/hooks` runs the quick checks on
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

`STORE=1` makes a store's build, without the GitHub update check (ADR 0014). The Jolla Store's
validator (`scripts/validate-rpm.sh`, for a Lite build with `FLAVOUR_SUFFIX=0`) and the store
submission: docs/release.md. `scripts/check-package.sh` looks inside built RPMs: the licence
texts are installed, a phone build (aarch64, armv7hl) has no demo data, and with `--store` no
update check.

`THIRD-PARTY-NOTICES.txt` (installed with `LICENSE`, shown under About) lists every crate the
core links and their licence texts. After a change to `Cargo.lock`, run
`python3 -I scripts/make-notices.py`; `scripts/check.sh` fails while it is out of date.

## Install on a device

Enable developer mode, then `scp RPMS/full/harbour-threesea-full-*.aarch64.rpm defaultuser@<phone>:`
and `devel-su pkcon install-local harbour-threesea-full-*.rpm` (or `rpm -U`).

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

## The C ABI under AddressSanitizer (development only)

`scripts/test-ffi-asan.sh` runs the tests of `core/src/ffi.rs` (every function the bridge calls,
with NULLs, short buffers and the buffers released the bridge's way) under AddressSanitizer, with
the nightly toolchain: a length that disagrees between a buffer and its release, or a write past
the room a caller gave, only shows there. Run it after changing `ffi.rs`.

## Fuzzing (development only)

`core/fuzz/` holds coverage-guided fuzz targets for the decoders that take input from outside.
It is its own workspace and never part of the app. From contacts and call peers: call signals
(`call_signal`), the call's data channel (`call_channel`), group call handshakes and frames
(`group_call_handshake`, `group_call_frames`), call SDP (`sdp_patch`), polls (`poll_json`), voice
messages (`voice_decode`), quotes and mentions (`message_text`). From files: the data backup's
container (`backup_archive`), CSV tables (`backup_csv`) and their typed records and conversion
(`backup_records`, with the seeds of `backup_csv`), Threema Safe (`safe_backup`), videos about to
be sent (`video_clean`, `video_size`, with the seeds of `video_clean`). From the browser:
Threema Web (`web_arp`, `web_chunks`, `web_images`), and WebP pictures it sends or that get
forwarded (`webp_clean`), GIFs about to leave (`gif_clean`). From the user's other devices: reflected
messages (`d2d_envelope`). Scanned and pasted: `codes`, `contact_codes`. From GitHub, when the
update check is on: `update_check`.

```sh
cargo install cargo-fuzz            # once; needs the nightly toolchain
cd core
export PROTOC=$(../scripts/fetch-protoc.sh)
cargo +nightly fuzz run voice_decode fuzz/corpus/voice_decode fuzz/seeds/voice_decode \
    -- -max_total_time=600
```

`fuzz/seeds/` holds valid starting inputs (a voice message from our own encoder, an SDP offer,
a PNG, the synthetic backup fixture and its tables, call signals, polls); without them the fuzzer
stalls at the container headers. The voice seed is rewritten by
`cargo test -p threesea-core --lib write_voice_seed -- --ignored`, the backup seeds by
`write_backup_seeds`, the video seed by `write_video_seed`, the WebP seeds by `write_webp_seeds`, the GIF seeds by `write_gif_seeds`. Crashes land in `fuzz/artifacts/` (ignored by git). Where a seal stands in
front of a decoder, the target seals the input itself (`group_call_handshake`) or starts behind it
(`d2d_envelope`).

`scripts/check.sh` (and so CI) builds every target with `cargo check` on the pinned toolchain, so a
target that stops compiling shows at once; fuzzing itself stays a manual run. `gif_clean` was
fuzzed for 15 minutes on 2026-11-05: it found the reordering of a control block in front of a
loop block, fixed in `core/src/gif.rs`, and then ran 43 million inputs without a finding.
