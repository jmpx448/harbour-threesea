# 0018: Removing the ID replaces the database and its key

## Context

"Remove ID from this device" grew table by table: empty the tables, delete media. Outside that
stayed the old database file under the unchanged key (any earlier copy of it stays readable, and
the running file until a checkpoint), logs, captures, the settings file with chat IDs and the
address book opt-in, notifications, process-wide state, running backups and the device group.

## Decision

One ordered wipe, like Android's `ResetAppTask`, in two halves.

The core (`deleteAllData`): a running backup gives up and takes its partial file along; calls end;
browsers stop, and the connection gets to send a call's end while the contact is still there; the
device group is left while a connection can still do that, with a limit, and failing that the ID
goes all the same and the reply says linked devices remain; the connection stops; a mark file goes
into the data directory; the store empties in one transaction; media and logs go; nothing of the
identity stays in the core's memory or its process-wide state. Only a refused PIN stops this, before
anything has happened. A store that refuses doesn't: on a full disk there may be no room for the
write-ahead log, and replacing the file needs none. Shrinking the emptied file is best effort.

The bridge, after a successful reply: marks the removal in the settings, stops the core, tells the
window and the players, clears all settings and the cache, empties the data directory
(`threesea_data_remove`), replaces the key in Sailfish Secrets with a new one (a wrapped key goes
with its passphrase) and starts a new core on a new, empty database. A start that finds the core's
mark file or the bridge's mark in the settings does the rest; the settings' mark goes only once the
new core has started. An old key still in Secrets after deleting it is a failure, not a success.

A new core instead of resetting each field means the App's state can't carry over by a field
somebody forgot. The core resets its fields as well, since its half must hold on a failed
replacement, and process-wide statics survive a new core.

## Consequences

The old file and any copy of it can't be read any more once the key is gone. A failure in the
bridge's half shows on the start page with "Try again", and closing the window then ends the
process, whatever the settings say, so that the next start tries too. Settings are device-wide, so
background running and the log level return to their defaults too. Backups in the user's folder,
saved pictures and Threema Safe's copy stay, and the dialog says that.
