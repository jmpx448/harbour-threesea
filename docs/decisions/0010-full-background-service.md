# 0010 — Full flavour (was "Chum"): start in the background at login

> Flavour names since 2026-10-27 (ADR 0014): **Lite** (was "Harbour", the Jolla Store rules) and
> **Full** (was "Chum", background service and calls). Below, "Harbour" and "Chum" mostly name
> the flavours by their old names.

## Context
ADR 0004 plans a Chum flavour that keeps the connection without the app having been opened.
The Harbour flavour already keeps running after its window is closed, but only once the user
has started it after each boot. Sailjail sandboxes the app: it only runs the `.desktop` file's
exact `Exec` line (an extra argument such as `--background` is refused: "Command line does not
match templates"), and every launch already goes through D-Bus activation (ADR 0004).

## Decision
- A systemd **user** service, `harbour-threesea.service`, wanted by `post-user-session.target`,
  enabled for all users by the package (a symlink in `post-user-session.target.wants`). It only
  calls `org.threesea.harbourthreesea.background()` over D-Bus (oneshot). D-Bus activation then
  starts the app exactly like the launcher does, in the same Sailjail sandbox.
- The app no longer shows its window when it starts; the D-Bus call that started it decides
  (`activate`/`openChat`/links/sharing show it, `reply`/`markRead`/`background` don't). Started
  from a shell without any call, it shows after 2 s. With "Keep running in the background" off,
  `background()` quits the app again.
- Build switch: `%bcond_with full` in the spec (`CONFIG+=full` for qmake, `THREESEA_FULL` in C++);
  `FLAVOUR=full scripts/sdk-build.sh` (the default) builds it into `RPMS/full`. The Harbour RPM contains no
  systemd files and passes `mb2 check`; the Chum RPM fails it only on the two service files.

## Consequences
- After a reboot the connection comes up at login, before the user opens the app.
- Battery: the same as leaving the app running after closing it; still to measure on a device.
- Sailfish Secrets asks "Authorize" when a new build first reads the database key, and in the
  emulator after every user-session restart, also at login for the background start (on the lock
  screen). Whether a phone with a device code asks after each boot is still to be checked.
- With "Encrypt stored data" (ADR 0009) the background start can only show "Threesea is
  locked" until the passphrase is entered.
- Chum itself builds from source on OBS, which needs Rust ≥ 1.95 (open question in CLAUDE.md §11).
