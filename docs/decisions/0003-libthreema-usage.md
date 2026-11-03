# 0003 — Use libthreema as a Rust crate, pinned via the upstream submodule

**Context.** Upstream's uniffi bindings (what Android uses) do not expose CSP, CSP-E2E or D2M;
Android still runs its connection, E2E processing, groups, PFS, blobs and Safe in Kotlin/Java.
libthreema itself does contain sans-I/O CSP, partial CSP-E2E, D2M, D2D rendezvous, ID backup,
identity creation, directory lookups.

**Decision.** Depend on `domain/libthreema/lib` directly as a path dependency from the
`third_party/threema-android` submodule pinned to tag `6.5.3-1216`
(commit `d44a2b53b2ea2c0bf1065bdaa96987570e5b4e6d`), no uniffi. Repeat libthreema's
`[patch.crates-io] blake2` in our root workspace (patches only apply from the root) and exclude
`third_party` from our workspace so libthreema keeps its own lint config. Missing protocol pieces
(groups, PFS, blob, Safe, most message types, the Libre license parameters for identity creation)
are implemented in `core/` with libthreema's primitives and its `TaskLoop`/provider patterns, kept
upstream-shaped so they could be offered back.

**Consequences.** Upstream updates are deliberate submodule bumps (record each in docs/build.md).
We own a sizeable protocol surface; every piece gets tests and a mapping entry in
docs/android-mapping.md.
