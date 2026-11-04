# 0023 — One gate for showing contacts' profile pictures

Status: accepted (2026-11-05).

## Context

"Accept profile pictures" stopped asking for and downloading pictures, and nothing else. Pictures
already here stayed on show, so the list ended up partly with and partly without them; pictures
synced from another device came in anyway; one announced while it was off was lost for good, and
turning it back on didn't bring the newer pictures of contacts with an older one. We had called
this Android's behaviour, but Android downloads whatever it is sent and only decides what it shows
(`getProfilePicReceive` is read in `IdentityAvatarFetcher`, `AvatarEditView` and `ContactUtil`,
never when a picture arrives). Android asks for pictures only in `ApplicationSetupSteps`; we also
ask when a chat opens and when a message comes.

## Decision

As Android: pictures that arrive are always downloaded and stored, however they come, and the
setting, now called "Show profile pictures", decides what is shown. Ours differs in one thing: with
it off we send no requests, as those are ours to send, and there is nothing to show them with. One
gate in the core does the showing for every consumer: `Store::shown_contact_picture_version(s)` for what the lists, pages and calls say
(0, as for no picture), `MediaStore::read_picture_for_ui` for the UI's image provider, and the same
setting in Threema Web's `request/avatar`. Our own picture and groups' pictures aren't affected.
Anything that isn't showing (linking a device, data backups) reads the pictures as they are.

## Consequences

With it off, the device still fetches the pictures contacts announce, as Android does. Turned on
again, those are all there at once, and contacts without one are asked the next time a chat opens or
a message comes, within the request's backoff. The user may want no fetching at all; it then needs an
honest text and a way to catch up when turned on.
