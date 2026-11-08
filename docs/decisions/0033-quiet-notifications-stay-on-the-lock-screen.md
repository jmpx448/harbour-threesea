# 0033 — Quiet notifications stay on the lock screen

## Context
A chat's notification is updated without sound or vibration for the next message of a burst, the
backlog after a login, an edited or deleted message and the app lock's notification, and always
when message sound is off. Those updates use lipstick's category `im.received`, which only has the
LED. It sets no priority, so lipstick gives the notification its default of 50, and every update
replaces all hints. The lock screen's icons list only priority 100 and up
(`NotificationColumn.qml`), the events view everything: the icon went while the LED went on
blinking and the events view still showed the notification. 0.2.0 didn't have the quiet updates.

## Decision
`backend.keepOnLockScreen()` sets `x-nemo-priority` to 120, the value of `x-nemo.messaging.im`,
on every chat notification before it is published. Lipstick lets a hint the app set stand over the
category's (`applyCategoryDefinition`).

It is in C++ because `setHintValue` isn't available to QML. That links
`libnemonotifications-qt5.so.1`, which the Jolla Store's validator allows.

Not chosen: a category of our own, as lipstick reads those from a system folder that the Lite
package can't install to; of lipstick's and commhistory-daemon's categories, none has a priority
of 100 or more without also a sound or waking the display.

## Consequences
- Both flavours keep the icon, also for those who have message sound off.
- The notifications that aren't about messages (a failed start, the license…) have no category and
  stay off the lock screen's icons, as before.
