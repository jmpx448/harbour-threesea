# 0006 — Vendored libthreema with a vendor branch (supersedes the patch files of 0005)

**Context.** We do not send changes upstream (user decision, see 0005). Phase 3 needs large
libthreema changes (forward secrecy, more message types, groups, blobs); a growing pile of
patch files is hard to edit, review and rebase.

**Decision.** `third_party/libthreema/` is a vendored copy of upstream `domain/libthreema`
(2 MB, 176 files), built from there. Classic vendor-branch workflow:
- Branch `vendor/libthreema` only ever contains *unmodified* upstream imports (one commit per
  upstream tag) and is merged into `main`.
- Our changes are normal commits on `main` touching `third_party/libthreema/` (commit messages
  start with `libthreema:`). The public history starts with one commit on top of the vendor
  import (2026-10-27; the earlier, private history was not published): it already contains all
  changes up to then, the former patches 0001–0003 included.
- Upstream update: `scripts/update-libthreema.sh <tag>` moves the reference submodule, imports
  the pristine tree on the vendor branch and merges it, so git combines upstream's and our
  changes (conflicts are resolved like any merge).
- `third_party/threema-android` stays as a pristine, read-only submodule: reference for the
  Android implementation and the source of shared assets (bad-password list, Safe schema).
- `git diff vendor/libthreema main -- third_party/libthreema` shows all our changes.

**Consequences.** One repository, ordinary history and review, no patch tooling. Our changes
keep libthreema's style, lints and tests (`cargo test -p libthreema` in `third_party/libthreema`)
so the code stays mergeable with upstream releases.
