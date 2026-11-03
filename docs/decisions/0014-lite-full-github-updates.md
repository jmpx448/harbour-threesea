# 0014 — Flavours "Lite" and "Full", releases on GitHub, an optional update check

Status: accepted (2026-10-27). Renames the flavours of ADR 0004/0010; adds an exception to
CLAUDE.md §7.6.

**Context.** The two flavours were named after their stores, "Harbour" (Jolla Store) and "Chum".
Neither will build us soon: Chum's OBS needs Rust ≥ 1.95 (CLAUDE.md §11), and the Jolla Store
review is not started. So both are distributed as RPMs on the project's GitHub releases page
first. Store names describe that badly, and users who install from GitHub get no updates from a
store.

**Decision.**
- The flavours are **Lite** (the Jolla Store's rules: same process only, no calls, no service at
  login) and **Full** (a systemd user service that starts at login, calls). Build: `FLAVOUR=lite`
  or `FLAVOUR=full` (the default) for `scripts/sdk-build.sh`; `%bcond_with full` in the spec,
  `CONFIG+=full` for qmake, `THREESEA_FULL` in C++. The package stays `harbour-threesea` in both
  (one or the other is installed); the RPM files are named `harbour-threesea-<flavour>-…rpm`.
- Builds for GitHub (both flavours) can **check for updates** (`core/src/updates.rs`):
  - off until the user agrees; asked once when the chat list first shows after setup
    (`UpdateCheckDialog`), switchable in Settings → Advanced;
  - at most one attempt a day (the core's hourly tick; the attempt is recorded before the
    request, so failures count too), or on "Check for updates now"; the request runs as a task
    of its own and reports back by events, so the core doesn't wait for GitHub;
  - one HTTPS GET to `api.github.com/repos/penttimus/harbour-threesea/releases/latest`, ordinary
    WebPKI (GitHub's keys are not pinned), no redirects, at most 1 MiB read; nothing about the
    user in it but the app version (User-Agent). GitHub sees the IP address, which the texts say;
  - a newer version shows a notification once; Settings → Advanced lists it with a button to its
    release page (only pages of this repository's releases are accepted as links). Nothing is
    downloaded or installed: the user installs the RPM.
- A store's build (`--with store` in the spec, `CONFIG+=store`) has no update check; the store
  updates it.

**Consequences.** CLAUDE.md §7.6 names this the second exception to "no third-party endpoints"
(after user-allowed self-hosted Threema Web servers, ADR 0013). Releases must be tagged
`v<version>` with the version in `rpm/harbour-threesea.spec` and `core/Cargo.toml`; drafts and
pre-releases are not offered. The older ADRs keep "Harbour"/"Chum" with a note on top.
