# Releases

Releases are built by GitHub Actions from the `release` branch; nobody builds or uploads RPMs by
hand. `main` is where work happens; `release` only ever receives merges from `main`.

## Making a release

1. On `main`: raise the version in all three places, `rpm/harbour-threesea.spec` (`Version:`),
   `Cargo.toml` (`[workspace.package] version`) and `harbour-threesea.pro` (`VERSION`), and add a
   section `## [<version>] - <date>` to `CHANGELOG.md` (it becomes the release notes).
   `scripts/release-version.sh --new-tag` checks all of this locally.
   If the store's schema changed since the last release (`MIGRATIONS` in `core/src/store.rs` got
   longer), keep the new release's schema as a fixture, so that every later version is tested
   on a database with its data in it:
   `THREESEA_SCHEMA_OUT=$PWD/core/tests/fixtures/store-schema-<version>.sql cargo test -p
   threesea-core --lib write_a_releases_schema -- --ignored`, and add the version, its
   migration count and the file to `RELEASED` in `core/src/store_upgrade_tests.rs`.
2. Open a pull request from `main` into `release`. CI runs the checks (now also requiring every
   text in `translations/`, `scripts/check.sh --release`), libthreema's and the SDK tests, and
   builds both flavours for all three architectures, as it does on every push.
3. Merge it. `.github/workflows/release.yml` then (only on `release`, also when started by hand):
   - runs all of CI's checks and tests again (the same workflow, without its builds); nothing is
     built without them,
   - checks the version again (and that the tag `v<version>` doesn't exist yet),
   - builds Full and Lite for aarch64, armv7hl and i486,
   - writes `SHA256SUMS` and a build provenance attestation per RPM,
   - creates the tag `v<version>` on the merged commit and the GitHub release with the 6 RPMs.

Releases are never marked as pre-releases or drafts: the app's update check (ADR 0014) ignores
those. Say "experimental" in the release notes instead.

A failed release workflow changes nothing (the tag and release are made last); fix the cause on
`main` and merge again. A published version is never rebuilt: a fix is a new version.

## One-time setup on GitHub

- Settings → Actions → General → Workflow permissions: "Read and write permissions" (the release
  job creates the tag and the release; the other jobs are read-only by their own settings).
- Settings → Code security: enable "Private vulnerability reporting" (SECURITY.md points there),
  and "Dependabot alerts" (advisories for the crates in Cargo.lock; `.github/dependabot.yml`
  only proposes updates of the workflows' actions).
- Your account: Settings → Emails → "Keep my email addresses private" and "Block command line
  pushes that expose my email".

### Protecting the `release` branch

Settings → Rules → Rulesets → **New ruleset** → **New branch ruleset**:

1. Name: `release`; Enforcement status: **Active**.
2. Bypass list: leave empty (also the owner goes through a pull request), or add yourself only
   if you want an emergency way around it.
3. Target branches → Add target → Include by pattern: `release`.
4. Rules:
   - **Restrict deletions** and **Block force pushes** (on by default).
   - **Require linear history**: off (merging `main` into `release` makes merge commits).
   - **Require a pull request before merging**; required approvals: 0 (you are the only
     reviewer; with collaborators, 1).
   - **Require status checks to pass** → Add checks: `checks` and the two `sdk-build` jobs
     (`sdk-build (full)`, `sdk-build (lite)`); they can be picked once they have run once.
     Tick "Require branches to be up to date before merging".
5. Create.

For `main`, a second ruleset with only "Restrict deletions" and "Block force pushes" keeps its
history safe while you can still push to it directly (required status checks would block direct
pushes).

## Installing a release on a phone

Developer mode on (Settings → Developer tools), and "Allow untrusted software" (Settings →
Untrusted software). Download the RPM for the phone's architecture (`aarch64` for current
devices, `armv7hl` for older ones), then in the terminal:
`devel-su pkcon install-local harbour-threesea-full-….rpm`. To switch between Full and Lite,
remove the installed one first (`devel-su pkcon remove harbour-threesea`; the data stays in the
home directory), then install the other.

An app that keeps running notices a new or removed package by itself, by a file of the package in
/usr/share: without a window it ends within minutes or at the next tap on its icon (a notification
then opens the new version, or another tap), and with a window it says so and ends when that is
closed. Lite has no package scripts for this (Harbour restricts them); Full's stop and restart the
running instance at the end of the transaction, too. Versions before this keep running on the old
files until they are closed or the phone restarts.

The launcher keeps an app's icon in memory, so a new icon in an update shows only after the home
screen restarts: reboot, or `systemctl --user restart lipstick` in the terminal.

## The Jolla Store (Harbour)

The **Lite** flavour is the one the store takes: no systemd unit, no calls, only libraries
Harbour allows. Jolla's own validator says it is in order
(`scripts/validate-rpm.sh`, which runs `rpmvalidation` inside the SDK image):

```sh
FLAVOUR=lite FLAVOUR_SUFFIX=0 scripts/sdk-build.sh aarch64 armv7hl
scripts/validate-rpm.sh RPMS/lite/harbour-threesea-<version>-1.aarch64.rpm
```

`FLAVOUR_SUFFIX=0` keeps the plain file name: the store insists on
`harbour-threesea-<version>-<release>.<arch>.rpm`, while a release on GitHub carries the
flavour in the name (`…-lite-…`) so both can sit side by side. CI runs the validator on every
Lite build, for every push, so a change that would be refused shows up at once.

What the validator checks and passes today (2026-10-30): file name and package name, the
`.desktop` file, the icons, the installed paths, the libraries linked, the RPATH, sandboxing
(Sailjail permissions), dependencies, scriptlets and the architecture.

Still to be done by hand, and only by the maintainer:

1. A Jolla account and the store's "Submit an app" form (harbour.jolla.com); the app's name,
   description, screenshots and the category, in English and German.
2. Decide what the store page says about the Threema licence: the app needs a Threema Libre
   licence bought in the Threema Shop, and it is not affiliated with Threema AG (the About
   page and the README already say so).
3. The store reviews by hand as well; expect questions about the Threema name. The launcher
   name is "Threesea" for exactly that reason.
4. Chum (for the Full flavour) stays blocked on Rust: OBS builds from source and the SFOS
   5.1/5.2 repositories only have Rust 1.75, while libthreema needs 1.95 (docs/decisions,
   CLAUDE.md §11).

`rpmlint` (which `mb2 build` runs, not the store) still says `no-changelogname-tag`: the spec
has no `%changelog`. Sailfish packages normally do not carry one — OBS writes it from a
`.changes` file — and the store does not ask for it, so it stays as it is.
