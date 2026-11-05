# 0027 — The app notices its package changing under it

Status: accepted (2026-11-05).

## Context

The app keeps running without a window by default, in Lite too. Lite has no package scripts, as
Harbour restricts them, and Full's only act at the end of a transaction. So after an update the old
binary ran on against the new QML on disk, which pages load lazily, and a tap on the icon raised
that mix. After a removal the process stayed connected and notifying until a reboot, with no way
to reach its settings.

## Decision

At the start the app notes device, inode, size and time of its main QML file under
/usr/share/harbour-threesea, and compares them on the wake job's tick, every five minutes while the
device is up, when the app comes to the front and for a call wanting the window. Not the executable:
Sailjail passes `--private-bin`, so inside the sandbox /usr/bin holds a copy that never changes, while
/usr/share/<app> is the real directory. It says nothing unless sure: a path that can't be looked at
leaves it running as it is.

- No window up: it ends, cleanly, through the same way out as any quit, after 30 s for the package
  manager to finish. Before the end of an update it publishes a notification if Keep running was on,
  since it stops receiving; a removal closes all its notifications.
- Called for what needs no window (Reply, Mark as read, background()): it does that, and then ends.
- Called for the window with none up (the icon, a chat's notification, a link): it ends at once and
  publishes a notification that opens the new version. It doesn't start that version itself: the
  launcher refuses a second start while this process and its sandbox are alive, whereas a tap on the
  notification or the icon after the end simply starts it.
- A window up: it stays, since the user is in the middle of something, and a notification says to
  close it and open it again. Closing then ends the process whatever Keep running says.

The D-Bus name stays until the process is gone, also while the core stops. So no second process can
start beside an ending core: a start from a shell sees the name and exits, and a tap on the icon in
those seconds gets no answer and takes another tap. Releasing the name first would be worse: the bus
would begin an activation that the launcher's lock refuses, and hold it pending for up to two
minutes. No lock file is needed in the data directory for this.

## Consequences

A Lite user with Keep running on stops receiving messages after an update until opening the app, and
is told so. It could start itself in the background, as Full's script does. That is left out: whether
a Lite app may do so in the Jolla Store is not known, and a tap brings it back.

The first tap on the icon right after an update only ends the old process and leaves a notification.
A link or a chat that tap asked for is not opened, and until the window is closed, pages loaded from
the new package can still meet the old binary.
