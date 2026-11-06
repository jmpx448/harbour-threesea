# 0031: Sharing a file leaves a short-lived copy in memory

## Context

Other apps can only take a file by path: the system's share dialog (`Sailfish.Share`,
`ShareAction`) runs outside our sandbox and opens the path, and some of what it hands to read it
later, such as an e-mail being written or a queued transfer. Our media are encrypted, so a share
needs a decrypted copy. Until now the only way out of the store was Save to Downloads, which stays
on the flash until the user removes it.

`ShareAction` also has `replaceFileResourcesWithFileDescriptors`, but `trigger()` doesn't call it,
no system QML uses it, and whether the dialog copes was not confirmed, so paths are used, like
Gallery and the file manager.

The only folders the dialog can read and that keep their files in memory are the system's
temporary folder and `/dev/shm`. Neither is private: Sailjail's `Base` permission gives an app no
`/tmp` of its own, and all apps run as the same user, so any of them can list it while a copy is
there.

The user decided on 2026-11-06 that Share is built: what is shared goes to the other app
unencrypted anyway, so it is less confidential than what stays in the store.

## Decision

- The core writes the copy, only on the user's word for those files, into a memory-backed folder:
  the system's temporary folder or else `/dev/shm`, checked against `/proc/self/mountinfo` to be
  tmpfs. With neither, Share of a file is refused; there is no fallback to the flash.
- A folder per share with the deadline and a random part in its name, owner only (0700, files
  0600); each file under its own name in a folder of its own. A private chat's files go as
  `file.<ext>`, so a listing names nothing of them.
- All copies together stay within one file of Threema's largest size (`shareTooLarge` otherwise):
  it is the phone's memory.
- A copy goes 10 minutes after it was made, by the wall clock, which counts the time the device
  sleeps: the deadline is in the folder's name, and while copies are there the core looks at the
  wall clock every 30 seconds the device is awake, as well as on every wake-up, when the app comes
  to the front and in maintenance, removing what is due (and anything dated further ahead, after
  the clock went back). After a sleep a copy goes within 30 seconds of the device being up again.
  Also at once when a chat is marked private, here or on a linked device, at "Delete ID and data",
  when the process ends (a share still being written then removes itself) and when the app starts
  again, before the store is opened.
- The lock doesn't remove it. With the PIN asked at once, the lock comes as the share dialog opens;
  removing it then would break every share before the other app reads it. What was shared has
  left our protection with the user's choice of app; the deadline bounds the rest.
- A share asked for before a lock, a chat marked private or the data deleted is refused when its
  turn comes, and if it was being written then, its copies go again and the answer names nothing.
  A chat hidden meanwhile does the same.
- Text goes as data in the share request, with no file at all.

## Consequences

- An exception to CLAUDE.md §7.1, stated there: until the deadline, the copy is readable by the
  user's other apps, which can see the folder; they can list the share folders' names in any case.
  Nothing of a share reaches the flash through us, and a restart ends it at the latest.
- Only our process removes copies. If it crashes or is killed, its copies stay until the app is
  started again or the device restarts, whichever comes first.
- What the chosen app and the share dialog do with their copy is out of our hands, as with anything
  shared.
- The image's fstab mounts `/tmp` as tmpfs, and `ShareAction` was seen in the SDK taking these
  resources. The dialog itself, whether a target reads within 10 minutes, and the sandbox's view of
  the folder were not run on a device.
