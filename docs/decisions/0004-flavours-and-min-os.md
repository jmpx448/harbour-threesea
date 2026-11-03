# 0004 — Minimum SFOS 5.1; Harbour and Chum flavours; no OpenRepos

> Flavour names since 2026-10-27 (ADR 0014): **Lite** (was "Harbour", the Jolla Store rules) and
> **Full** (was "Chum", background service and calls). Below, "Harbour" and "Chum" mostly name
> the flavours by their old names.

**Context.** Linking against a target pins the minimum OS (glibc symbol versions etc.). The Jolla Store (Harbour) forbids
daemons, systemd units, D-Bus service files and autostart, which limits "works in the
background" to an app that keeps running after its window is closed. Chum allows a systemd user
service (Whisperfish's approach).

**Decision** (user, 2026-10-21).
- Minimum Sailfish OS is **5.1** (first decided as 5.2, lowered the same day so the official
  emulator, whose newest image is 5.1.0.11, can run it). We build against the 5.1.0.11 target;
  those builds run on 5.2. No compatibility work for 5.0 or older.
- Ship **two flavours from one tree**:
  - **Harbour**: Jolla Store rules only. Connection lives in the app process; the window can be
    closed without quitting (`quitOnLastWindowClosed = false`), `Nemo.KeepAlive` `BackgroundJob`
    for wakeups. Launching goes through D-Bus (`X-Maemo-Service/Object-Path/Method` in the
    .desktop + `ExecDBus=harbour-threesea` in X-Sailjail, which Sailjail turns into a D-Bus
    activation file): the launcher calls `activate()` on the running, windowless process instead
    of hitting mapplauncherd's single-instance lock (held by firejail). Notification taps use the
    same service (`openChat`). Implemented in Phase 2 (bridge/main.cpp, bridge/dbusinterface.cpp).
  - **Chum**: everything in Harbour plus a systemd user service that starts the core at login and
    keeps the connection when the UI is not running, an optional jolla-settings entry, and a
    Transfer Engine share plugin.
- **No OpenRepos.**

**Consequences.** One qmake/spec switch (`CONFIG += full` / `%bcond_with full`) selects
the flavour. Flavour-specific code is kept small and in `bridge/` (platform integration). Every
feature declares which flavours get it; the Harbour build must always pass `mb2 check`.
