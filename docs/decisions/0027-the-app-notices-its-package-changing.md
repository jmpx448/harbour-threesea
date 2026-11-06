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
  notification or the icon after the end simply starts it. What the tap was for opens there (user
  decision 2026-11-06): the notification's action makes the same D-Bus call again (a chat, the first
  link through `openLink`, the update, Threema Web or Safe page), and the new version, finding that
  notification at its start, makes the call itself on the icon's tap, then closes it; any other way
  in, or dismissing it, drops it. Only what the tap carried is kept, in that notification, as the
  chat's own notification held it already: no message content, no key, nothing in a file of ours.
- A window up: it stays, since the user is in the middle of something, and a notification says to
  close it and open it again. Closing then ends the process whatever Keep running says.

The D-Bus name stays until the process is gone, also while the core stops. So no second process can
start beside an ending core: a start from a shell sees the name and exits, and a tap on the icon in
those seconds gets no answer and takes another tap. Releasing the name first would be worse: the bus
would begin an activation that the launcher's lock refuses, and hold it pending for up to two
minutes. No lock file is needed in the data directory for this.

The end goes in this order: window hidden, pages destroyed (a closing chat page hands the core its
waiting files and its draft), files that are being prepared delivered for up to 3 s, core stopped
(5.5 s at most), workers waited for (3 s at most). Waiting for the core's lock takes up to 2 s more, as
a media read can hold it; without it the process leaves with the core running, as after a crash, once
what must arrive has had 3 s more. So the end stays under the 15 s after which Full's update kills the
process. File work on a blocking thread has the core's limit like any command: 4 s, then it gets no
answer, and its thread is left to the end of the process, touching nothing of the bridge. What the
pages handed over and files that were on their way without a good answer are said by a notification,
the window being gone by then: the one notification published from C++.

## Consequences

A Lite user with Keep running on stops receiving messages after an update until opening the app, and
is told so. It could start itself in the background, as Full's script does. That is left out: whether
a Lite app may do so in the Jolla Store is not known, and a tap brings it back.

The first tap on the icon, a notification or a link right after an update only ends the old process
and leaves a notification; the next tap, on it or the icon, opens what the first was for. A link
goes into that notification whole, so a compose link's text sits in Lipstick's notification store
until the notification is tapped or dismissed, as the link itself was handed to us in plain. Until the
window is closed, pages loaded from the new package can still meet the old binary.
