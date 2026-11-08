# Device checklist: voice, video, background, and everything else only a phone can show

What the emulator cannot show: it has no earpiece and no proximity sensor, its PulseAudio is
VirtualBox's, and it has no hardware video encoder. Everything else of this round was checked
there (see docs/STATUS.md, "Feature round 2026-10-29").

Note per step: OK, or what you saw or heard. A screenshot helps for the screens; for sound, a
line of the log is often enough (Settings → Advanced → View log → "Save support package" writes
it to Downloads). The log never holds message text.

Install the fresh **Full** RPM: `devel-su pkcon install-local harbour-threesea-full-….rpm`.
(Lite has no sensor permission and no GStreamer, so at-the-ear playback and video scaling are
not in it at all — that is on purpose.)

## A. Voice messages at the ear

Settings → Chats: "Play voice messages at the ear" is on by default.

1. Open a chat with an unheard voice message and hold the phone to your ear (do not tap play).
   - Expect: it starts at the first unheard one, through the **earpiece**, at call volume; the
     screen goes black and takes no taps.
   - Before the fix it was "scrambled silence" — if anything like that is left, the log line to
     look for is `earpiece pipeline error` (it names the GStreamer element).
2. Take the phone away from your ear.
   - Expect: playback pauses, the screen comes back, and pressing play continues on the
     loudspeaker.
3. Hold it to your ear again in the middle of a message.
   - Expect: it carries on from where it stood, on the earpiece.
4. Turn the setting off and hold the phone to your ear.
   - Expect: nothing happens (the sensor is not even started).

## B. The sound between two voice messages

1. Two voice messages one after the other in a chat, the first unheard: play the first one.
   - Expect: when it ends, a short soft bell, then the second one plays.
   - It should be quiet and round,
     and at the ear only half as loud.
2. Settings → Notifications → In the app → Sound off, repeat.
   - Expect: no bell, and the second message still plays.

## C. In-app sound and vibration

1. With a chat open, have someone write to you (ECHOECHO works: send it a message).
   - Expect: no notification banner, but the soft sound and a short vibration.
2. Turn both off in Settings → Notifications → In the app and repeat.
   - Expect: the message arrives silently.

## D. Playback speed

1. Play a voice message and tap the small "1×" at the right end of the bubble.
   - Expect: it steps to 1.5×, then 2×, then back; the playback follows at once and the next
     message keeps the choice.
2. For an **AAC** message (one recorded on this phone, or from Android/iOS), listen whether the
   voice sounds normal or like a chipmunk at 1.5×.
   - The pitch is kept only if the phone has GStreamer's `scaletempo`. If it sounds high, say
     so — then the pipeline needs a different element.

## E. Video size

Settings → Media & storage → Video size: "Small (480 px)".

1. Send a video from the Gallery (a long one, taken with the phone's camera) to ECHOECHO.
   - Expect: it is sent noticeably smaller (check the size in the message details), plays with
     sound, and the picture is upright and not stretched.
   - The log says `video scaled down with <element>` — please report that element name, or
     `video scaling: no H.264 encoder on this system` if the phone has none (then videos are
     sent unchanged, which is the intended fallback).
2. Send a video taken in the app (attachment menu → camera) with "Medium (848 px)".
   - Expect: the same, and no leftover files: `ls ~/.cache/org.threesea/harbour-threesea/capture`
     is empty after sending.
3. Set it back to "Original size" and send once more.
   - Expect: the same size as in the Gallery (the video itself isn't re-encoded; its metadata goes).
4. A video already smaller than the setting (a short clip at 480 px or less) with "Small (480 px)".
   - Expect: not re-encoded (no `video scaled down` line in the log).
5. Have the recipient look at these, on Android or another Threesea.
   - Expect: the bubble keeps the video's proportions (16:9 stays 16:9, also filmed upright) and
     it plays with sound. If a video refuses to play as media but plays when sent as a file,
     please keep it: then the core removed too much for that player.
6. A video with a position in it (the camera with location on; `exiftool` or a media info app shows
   `GPS`), sent as media and saved by the recipient.
   - Expect: no position, no phone model and no recording time in the recipient's copy.
7. Share a video to Threesea from the Gallery, into a chat, then send it.
   - Expect: it goes as media with its thumbnail, like one picked in the chat, and steps 5 and 6 hold.
8. Send a `.webm` or `.mkv` through the video picker.
   - Expect: a dialog says the file keeps its metadata, "Send as it is" sends it as a plain file
     with its own name and caption, "Don't send" brings the caption back and sends nothing.
9. The same, leaving the chat right after tapping send.
   - Expect: a notice that it was not sent, the caption is in the chat's draft next time.

## F. Chat background and text size

1. Settings → Appearance → Chat background: pick a picture.
   - Expect: it shows behind the messages in every chat, dimmed enough that the bubbles and
     their text stay easy to read — in a light **and** a dark Ambience.
2. Pull down on the Appearance page → "Use the Ambience's background".
   - Expect: back to the usual background.
3. Settings → Appearance → Message text: Small and Large.
   - Expect: only the message text changes size (day headers, times and names stay).

## G. Naming the devices

With Threema for Desktop linked:

1. Settings → Linked devices → long press this device → Rename, type a name.
   - Expect: the list shows it; the connection reconnects once (the chat list may blink
     "Connecting…"), and **the Desktop's own list of devices shows the new name** for the phone
     (it may need a reconnect there).
2. Long press the Desktop entry → Rename.
   - Expect: our list shows your name for it; the Desktop keeps its own name for itself (this
     one is only ours — a device's label is encrypted with the device group key).
3. Rename once more with an **empty** field.
   - Expect: this device is called "Sailfish OS" again, another device its own name.

## H. Scrolling back through a long chat

The one thing the emulator cannot judge (its software renderer stalls on every flick, whatever
the app does — docs/STATUS.md says what was measured).

1. Open your longest chat and flick back through it, a good way past the first screens.
   - Expect: the scrolling keeps going while older messages come in; no stop of half a second
     when a block arrives, and **the view stays where you are** — it must never jump back to
     the newest message.
2. Keep going to the first message of the chat.
   - Expect: it ends at the beginning, with every message once and in order, and the day band
     at the top always says the day of the topmost message.
3. Do the same in a chat with many photos and GIFs.
   - Expect: the same. The pictures are decoded off the interface's thread now, so they may
     appear a moment after their bubble.
4. In that long chat, scroll a good way back, then use Sailfish's quick scroll (flick hard,
   then tap the arrow at the bottom right) to go back to the newest message.
   - Expect: the newest messages are there within a blink, in the right order, with the input
     below them — not older ones, and no mixture.
5. **If it still stalls:** Settings → Advanced → Log → Detailed, scroll back through that chat
   again, then Settings → Advanced → View log → Save support package and send it. The log then
   holds lines like "chat: interface stopped for 240 ms (320 messages loaded, fetching: false)"
   — that says whether the stop is the loading or the drawing. Set the log back to "Problems
   only" afterwards.

## I. Mark as unread

1. Chat list → long press a read chat → "Mark as unread".
   - Expect: the row looks unread (name and preview in the highlight colour) without a number;
     the cover's count does not change.
2. Open the chat.
   - Expect: the mark is gone when you come back.

---

# Added 2026-10-30 (the round after the cover and media fixes)

Install the fresh RPM as above. A–I are the earlier round; these are new.

## J. The cover names who is waiting

1. Have two people write to you (or write from Threema Desktop to two of your own chats) and
   leave **both** unread. Put the app aside and look at the cover.
   - Expect: the number of unread messages, and under it the name of the chat whose message is
     the **newest**.
2. Open and read only that newest chat, then put the app aside again.
   - Expect: the count drops and the name is now the **other** chat, the one still unread.
     (Before this round the cover kept the name of the chat you had just read.)
3. Open the archive (chat list → pull down → Archived chats) while something is unread, go
   back and look at the cover.
   - Expect: the count is unchanged — the archive never counts towards it.
4. With the app lock on: lock the app, look at the cover.
   - Expect: the count, no name.

## K. Pictures from Threema Desktop

Settings → Media and storage: "Download on mobile data" with pictures ticked, "File size" on
"Any size".

1. Send a picture **from Threema Desktop** to someone, with the phone on mobile data.
   - Expect: the picture appears in the phone's chat by itself, no tap needed. (This is the bug
     from 2026-10-30: it stayed a placeholder.)
2. The same with a file of a few MB, with "File size" set to "Up to 1 MB".
   - Expect: the preview comes, the file waits until you tap it.
3. Switch "File size" back to "Any size" and tap the file.
   - Expect: it downloads.
4. Anything that was left as a placeholder before the update should arrive by itself after the
   next start of the app. If you have sent many pictures from Desktop, do that first start on
   Wi-Fi: the whole backlog comes at once.

## L. Notes (a group with only yourself)

1. Make a group with no other members ("New group", no one picked) and write a note in it.
   - Expect: the message gets its "sent" mark, and it appears in Threema Desktop too.
2. Send a picture into that group.
   - Expect: it arrives on Desktop as well.

## M. Export a chat

1. A chat with pictures → swipe left to its details → "Export chat" → with pictures, no password.
   - Expect: "Saved to Downloads"; the ZIP holds `messages.txt` and the pictures. Open it on a
     computer and read the text: one line per message, times in your own time zone, "Me" for
     yours.
2. The same with a password.
   - Expect: the ZIP asks for it when opening, and the password works.
3. A long chat with many large videos.
   - Expect: it takes a while but the app stays usable, and messages still arrive meanwhile.

## N. Editing a photo before sending

1. In a chat, attach a photo (from the Gallery or the camera) and **tap the small preview**
   above the input.
   - Expect: the picture with a frame around it; dragging the corners cuts it, dragging inside
     moves the cut, and the pull-down menu turns it.
2. "Use", then send it.
   - Expect: what arrives is what the frame showed, in the right orientation; the original
     picture in the Gallery is untouched.
3. Drag the **left** grips, hard, towards the edge.
   - Expect: the dialog stays open (the grips are kept away from the edge where a swipe closes
     it).

## O. Battery and the connection (a whole day)

This is the one measurement the emulator cannot give at all, and the last open item of Phase 4.

1. Settings → Advanced: "Keep running in the background" on. Use the phone as usual for a day
   with the app running.
   - Note: how much battery Sailfish's own battery view ascribes to Threesea, and whether
     messages arrived while the screen was off (and how late).
2. Go into a tunnel / flight mode for a few minutes, come back.
   - Expect: the chat list's header says "Offline", then "Connecting…", then goes quiet, and
     anything sent meanwhile goes out by itself.
3. Leave the phone untouched overnight.
   - Expect: in the morning the messages are there, and the app is still connected (the log
     shows reconnects, not a dead connection).

## P. French

The emulator has only the C locale, so no language can be shown there at all.

1. Settings → Language (of Sailfish OS) → Français, then open Threesea.
   - Expect: the whole interface in French — the chat list, a chat, the settings pages, the
     cover ("messages non lus"), and the plurals ("1 membre", "3 membres").
2. Back to your usual language.
   - Expect: everything in that language again.

## Q. Many favourite GIFs

Worth doing with a lot of favourites. The emulator renders in
software, so it cannot say how this feels on a phone.

1. Open a chat, then the emoji panel's GIF tab.
   - Expect: the tab opens at once, thumbnails first; the GIFs on screen start playing shortly after.
2. Scroll the grid quickly up and down.
   - Expect: it follows the finger; tiles show still pictures while moving and play once it stops.
3. Type in the chat, or open another page, with the tab showing GIFs.
   - Expect: no lag. GIFs playing somewhat slower than usual is acceptable, a stuck interface is not.
4. Long press a GIF far down the list and remove it.
   - Expect: the list stays where it was.
5. A GIF in a chat bubble, then in the full-screen viewer, also pinched.
   - Expect: plays in both, sharp when zoomed; a transparent GIF shows the bubble behind it.
6. Settings: turn off auto-play of animated images, open the GIF tab.
   - Expect: still pictures, nothing playing.

---

# Added 2026-11-02 (black videos, voice that would not go away, media controls)

## R. Playback and the system's media controls

1. Open a video message.
   - Expect: **the picture**, with sound. If it is still black, say whether the time runs and
     whether there is sound, and send the support package: the phone draws video another way than
     the emulator did, so this is the one thing that could only be checked here.
2. Put the app aside while the video plays.
   - Expect: it pauses; coming back, the play button carries on.
   Then, in the same video:
   - Drag the slider somewhere and let go. Expect: the picture goes there, and the handle keeps
     following the playback afterwards.
   - Watch it to the end. Expect: the first frame (or at least a working page) with 0:00, and play
     plays it again. At the end, drag the slider back instead. Expect: playing starts from there.
3. Play a voice message, then lock the phone and wake the screen.
   - Expect: on the lock screen "Voice message", the chat's name, "Threesea" and a pause button that
     works. (The controls are on the lock screen only; Sailfish offers no position slider there.)
4. With a headset or Bluetooth device: its play/pause button during a voice message.
   - Expect: it pauses and plays. Report what a car or Bluetooth device shows for the position.
5. Settings → Privacy → "Hide content on the cover" on, repeat step 3.
   - Expect: no chat name on the lock screen. Turn it back as you had it.
6. Play a long voice message and go back to the chat list.
   - Expect: it plays on and the lock screen controls still work. Pause it there and wait 10 minutes.
   - Expect: the controls are gone.
7. Pause a voice message in the chat and go back to the chat list.
   - Expect: the controls are gone, and the bubble starts from the beginning.
8. Play a voice message and close the app (from the cover or the top edge), then open it again.
   - Expect: **it stops on closing and stays silent on opening.** This is the bug as reported: if it
     still resumes, say what you did just before, and whether music or a call was involved.
9. Play a voice message, start music in another app, then stop the music.
   - Expect: the voice message pauses and stays paused.
10. Pause a voice message by hand and cover the proximity sensor (Full).
    - Expect: nothing starts. Then section A still holds: playing at the ear, pausing off the ear and
      carrying on when back at the ear.
11. Open a chat with an unheard voice message with your hand already over the sensor (Full).
    - Expect: nothing starts until the hand has been taken away and brought back.
12. During the soft bell between two voice messages, press pause on the lock screen.
    - Expect: the second message does not start.

---

# Added 2026-11-03 (Threema's conditions: client-info, license, icon)

## S. What Threema's conditions changed

1. Install the update and open the app with a working connection.
   - Expect: straight to the chat list, connected. This start asks the directory about the license
     for the first time since setup; a valid key shows nothing at all. If the license page opens,
     note the red text and send the support package.
2. Settings → Advanced → View log → Save support package; read the header.
   - Expect: your real device model and Sailfish OS version, not `unknown`. The same values go into
     the client-info. (The emulator has no `/etc/hw-release`, so only a phone can show this.)
3. With Threema for Desktop linked: look at its list of linked devices, after the phone has
   reconnected once.
   - Expect: the phone listed with your label, and `Threesea (<your model>)` with the app version
     wherever Desktop shows those details. This is also the only live test of the mediator handshake
     with the new client-info: the phone must simply stay connected and exchange messages.
4. The new icon (restart the home screen first: `systemctl --user restart lipstick`, or reboot; the
   launcher keeps the old picture until then), in a light **and** a dark Ambience: launcher, cover, a notification, the app's
   switcher.
   - Expect: sharp at every size, the bubble's dots readable, and it looks at home beside Phone and
     Messages.
5. Settings → About, in German and in French.
   - Expect: the new text about Threema reads well and fits on the page.

6. Report spam: a chat from an unknown, unverified contact shows a banner under its header; a chat
   you have answered, or a contact you named, doesn't. Tap it and read the dialog, then cancel.
   - Only accept for real spam: it sends a real report to Threema. If you get some, this is the only
     end-to-end test of it there is; note whether "Reported to Threema as spam." appears.
   - The × hides the banner for good in that chat.

The refusal itself (license page, "Threesea is offline" notification in the background) cannot be
triggered with a valid key. The page was tested on the emulator against the real directory with a
temporary build; the notification was not. Testing it needs such a build for the phone.

## T. Older phone-only items collected from docs/STATUS.md

Calls (Full):

1. 1:1 audio call, both directions, with another Threema app.
   - Expect: it rings, sound both ways without echo; speaker and earpiece switch; hang up on either
     side ends it.
2. 1:1 video: camera on and off, switch camera, hide own picture.
   - Expect: picture both ways, upright; note how warm the phone gets over a few minutes.
3. Group call with camera: start one, and join one from the chat's banner.
   - Expect: tiles for everyone, with sound; note CPU and heat with three or more people.

Camera:

4. Attachment menu → camera: a photo and a video, sent to ECHOECHO, then the same with the front
   camera.
   - Expect: both arrive upright and in focus, with sound on the video, also from the front
     camera (`metaData.orientation` follows the sensor; the front one's sign is what to look at),
     and the front camera's picture isn't mirrored.

Notifications:

5. Message arrives with the app in the background: tap it; then reply from the notification.
   - Expect: the tap opens that chat; the reply is sent and appears in the chat.
   - Mark as read from the notification, also with the app in the background.
     Expect: the chat is read and the notification gone.
   - Close the app (Full: background running off) with a notification showing, then Reply from
     it; again with Mark as read.
     Expect: the app starts without a window, the reply is sent, the chat is read.
   - Both from the lock screen of a locked phone.
     Expect: the same after unlocking.
   - With a notification showing, end the app (`pkill harbour-threesea` over SSH) and open it
     again, leave it running in the background, wait more than 12 hours, then Reply from that
     notification.
     Expect: the reply is sent.
   - Type a reply in a notification on the lock screen, a new message arrives in that chat, the app
     lock's grace time passes, then unlock the phone.
     Expect: the reply waits for the PIN and goes out after it. If it arrives without a valid token
     anyway: "Your reply was not sent", nothing silent. This timing is what lipstick's order of
     closing and calling decides, so look at which of the two it is.
   - With "Encrypt stored data" on and a notification showing, end the app the same way, then
     Reply from the notification; again with Mark as read.
     Expect: after a Reply, "Threesea is locked" says that the reply waits. After entering the
     passphrase, the reply is sent and the chat is read. Until then nothing is sent and the chat
     stays unread.
   - With a notification showing a message, have the sender delete it; again, edit it.
     Expect: the notification says "Message deleted", or the new text, without a sound, a vibration
     or a banner, and the next message alerts as usual. Mind the light: it may blink for this.
   - App lock on, app in the background past the grace time, with a notification showing.
     Expect: it closes and "Threesea is locked" takes its place, without a sound. A new message
     then gives that one notification with the usual sound and vibration, no name, no text, no
     Reply; tapping it opens the PIN page. After the PIN it is gone.
   - The same with the phone asleep through the grace time: wake it without opening the app.
     Expect: the chat's text is gone within about 30 seconds.
   - Keep running off, app lock on: close the window with a notification showing.
     Expect: its text is gone from Events too.
   - "Encrypt stored data" on, a notification showing: `pkill -9 harbour-threesea`, then start it.
     Expect: before the passphrase, name and text are gone from Events; "Threesea is locked" stands
     for them. After a restart of the phone nothing can do this until the app is started, so the
     last text stays until then.
   - Type a reply in a notification on the lock screen, let the app lock's grace time pass before
     unlocking the phone.
     Expect: "Threesea is locked" says that the reply waits, its text is in the chat's draft in the
     chat list; it goes out after the PIN and leaves the draft.
   - The same, then `pkill harbour-threesea` (and again with `-9`) before the PIN, then start it.
     Expect: "Your reply was not sent", its text in the chat's draft. With "Encrypt stored data"
     before the passphrase the same notice, without a draft. No "to send your reply" stays behind,
     and nothing goes out.

Threema Web:

6. Pair with web.threema.ch and leave it open for a while.
   - Expect: chats and sending work; the browser shows the phone's battery level, and it reconnects
     after the phone has slept.
   - With the browser open, mark a group private on the phone, then take the mark off.
   - Expect: the group leaves the browser's list and comes back with its name, members and chat.

Display:

7. On a phone with a display cutout: chat header, a contact's card, PIN entry, the
   call's video, the camera's clock, the media viewer.
   - Expect: nothing under the cutout; media viewers use the full screen.

Threema Safe and backups:

8. Threema Safe: make a backup, then restore it on a test ID, not one in daily use.
   - Expect: contacts, groups and settings come back.
9. Android data backup: `docs/android-backup-device-checklist.md`.

Lite:

10. Install the Lite RPM instead of Full and repeat a short message exchange.
    - Expect: connected once opened, no call buttons, no background service, and chats kept from Full.

## U. Linking a number or address, and the address book sync

Linking sends a real request to Threema, and possibly a real SMS or e-mail. Use your own number and
address only.

1. Settings → My profile → Phone number: type a national number.
   - Expect: the hint asks for the country code, and the dialog can't be accepted.
2. Type your number as `+49 (0) 171 …`, with your own country and number.
   - Expect: "The code goes to +…" with no extra zero. Accept: the SMS arrives and the code links it.
3. Repeat with the same number.
   - Expect: "already linked" and no SMS.
4. E-mail address: link one, open the link in the mail. Then repeat with the same address.
   - Expect: first "Waiting for you to confirm…", then the address alone; the repeat says "already
     linked" and sends no mail.
5. Remove both links from the dialogs' pulleys.
   - Expect: no notice, both show "Not linked".
6. Settings → Privacy → "Find contacts from the address book", with a real address book.
   - Expect: "%n new contacts…" or "%n address book entries checked, no new contacts", with a count
     near the number of address book entries that have a phone number or an e-mail address. The
     notice "No phone numbers or e-mail addresses found" with a filled address book is a bug in
     `AddressBookReader.qml`.
7. Contacts: pull to refresh.
   - Expect: the same notice.
8. With no network, pull to refresh.
   - Expect: an error notice.

## V. The app lock

Settings → Security → App lock on, with a PIN. `tests/lockstate` covers the page stack without a
device; what is left here is the real cover, the real window manager and the real sleep.

1. Lock after "Immediately". Put the app aside and look at the cover.
   - Expect: no + on the cover, and no name. Tap the cover: the PIN page.
2. Lock after "30 seconds". Put the app aside for a minute, with the screen on, and look at the
   cover without opening the app.
   - Expect: no + and no name once the 30 seconds are over.
3. The same, with the phone sleeping for ten minutes and the screen locked, then wake it.
   - Expect: no name on the cover at the latest when you come back to the app, and with "Keep
     running in the background" on already within five minutes.
4. Play a long voice message, put the app aside and lock the phone.
   - Expect: it plays on past the 30 seconds, without the chat's name on the lock screen. Pause
     it there: the controls go away after a second.
5. Lock after "30 seconds", put the app aside for a minute, open https://threema.id/ with a
   made-up ID in the browser.
   - Expect: the PIN page stays up, and after the PIN the contact (or the error) shows.
6. During a call (Full), tap a message's notification.
   - Expect: the call stays on screen, a short notice says the chat opens when the call is over,
     and it does.
7. During a call with lock "Immediately": put the app aside, hang up on the cover, come back and
   enter the PIN.
   - Expect: the chat list or the chat, no empty call page.
8. App lock switched off, but with a PIN set for the private chats. Mark a chat private, open it
   from the contacts, enter the PIN and put the app aside past the grace time.
   - Expect: coming back, the chat is closed.
9. A forgotten PIN, the way the app lock page and the README describe it, on a test ID with a
   backup, in both flavours. Lock on with a PIN, close the window, then in a terminal
   `rm -rf ~/.local/share/org.threesea/harbour-threesea`, and open the app again before restarting.
   - Expect: the same lock page (the process is still running). This is why the restart is a step.
   Now restart the phone and open the app.
   - Expect: the license page, no lock; in Full the login service has started the app in the
     background first, without a window. Restoring the ID works, and there is no lock afterwards.
10. The same with a passphrase on under Settings → Security → Encryption of stored data.
    - Expect: after the restart the passphrase page, also with the folder gone. A wrong passphrase
      keeps you there; the right one leads to the license page, with no app lock.

## How calls end

Only a phone with a real network shows these (Full).

1. In a 1:1 call, turn off Wi-Fi and mobile data on the other phone (or walk out of range).
   - Expect: after about 10 s "Connection interrupted…" instead of the time, after about 30 s
     "Connection failed" and a "Call failed" line in the chat. Back within the 30 s: the time
     returns and the call goes on. In a call with someone muted nothing of this shows.
2. The same in a group call, once with others speaking and once alone with everyone else muted.
   - Expect: a notice "The connection to the group call failed." after about a minute of no
     connection, and **no** such notice while the connection is fine, however quiet the call is.
     If one comes, the log line to look for is `group call failed: silence`.
3. Accept a call with the phone offline (airplane mode right after it rang).
   - Expect: "Connecting…", then "Connection failed" after a minute; the chat says "Call failed",
     not "You declined the call".
4. Call someone who has video calls off, as a video call.
   - Expect: it connects as an audio call: no camera, no video buttons, no preview.
5. While in a call, have someone else call.
   - Expect: they are told you are busy, and their chat shows "Missed call" with a notification.
6. End a call, and have someone else call within two seconds, while "Call ended" shows.
   - Expect: their name and picture on the answer screen; after answering not muted, no camera.
7. Hang up a call on both phones at the same moment.
   - Expect: one line in each chat, no "Missed call".
8. Start a video call and watch the phone's memory during a group video call with three or more
   people for a few minutes, then repeat it a few times.
   - Expect: pictures keep coming, and memory goes back down after each call (a picture left
     behind by a tile no longer stays until the app exits).
9. Set the clock two minutes ahead by hand, and have someone call.
   - Expect: it rings.
10. Start a call in the moment someone else's call comes in (try a few times; easier on a slow
    network), and hang up a just started call at once.
    - Expect: the incoming call rings on, perhaps after a short "Call failed"; hanging up the
      started call leaves the other phone not ringing, and one line in the chat.

## One call has the microphone

Two phones needed for the 1:1 parts; a group with at least one other member for the rest.

1. Open a group call and tap mute while it still says "Joining…".
   - Expect: the button stays highlighted, your tile shows the crossed-out microphone once in the
     call, and the others hear nothing and see you as muted.
2. Accept a 1:1 call and tap mute on the call page at once, or start one and tap mute while it
   says "Calling…".
   - Expect: the peer hears nothing and sees you as muted.
3. Join a group call with four or more others already in it.
   - Expect: you start muted, the others hear nothing until you unmute. With three others you
     start unmuted.
4. In a group call, have someone call you; answer on the call page, once more from the cover.
   - Expect: you are out of the group call the moment you answer (the others see you leave and
     hear nothing of the private call), only the caller's voice plays, and after the 1:1 call you
     land in the chat, not on a stale group call page.
5. In a 1:1 call, tap a group chat's notification.
   - Expect: the call stays on screen, a notice says the chat opens afterwards, and no group call
     starts next to it. (The call's page stays on top, so the bridge's rule that joining ends a
     1:1 call has no way in from the UI now; `tests/lockstate` covers the page.)
6. Go to the home screen during a group call.
   - Expect: the cover shows the group's name and the time, with mute and hang up acting on the
     group call. "Hide content on the cover" and a private group hide the name. A 1:1 call ringing meanwhile takes
     over the cover with answer and decline.
7. Close the window (not minimize) during a 1:1 call, then during a group call.
   - Expect: the call ends in both cases and the others see you leave. Closing while a call only
     rings leaves it ringing.
8. Take away the microphone in a group call if you can (another app holding it exclusively, or
   the Microphone permission off).
   - Expect: the call ends with a notice instead of showing the microphone on. In a normal group
     call, no such notice comes out of nowhere: if one does, the log line to look for is
     `group call pipeline error` (it names what failed).
9. With more logging on (Settings → Advanced) and "Always relay calls" off, call an unverified
   contact and look at the log.
   - Expect: `call candidates: relay` only, no `host` or `srflx`, on both an outgoing and an
     incoming call. A verified contact also shows `host` and `srflx`.

## Ringing, routes and the screen in calls

Full, unless it says otherwise. With a second phone calling.

1. Incoming call on a locked phone, once with the screen off and once with it on.
   - Expect: the display comes on; whether the call page comes over the lock screen, or only the
     notification with the caller's name, Decline and Answer shows. Say which. Note if the phone
     asks for the code on Answer.
2. Answer from that notification; next time Decline; next time let it ring out; next time open the
   app while it rings.
   - Expect: Answer opens the running call, Decline ends it, and the notification goes in all four
     cases. No notification while the call page is in view.
3. The same with the app lock on, and from a private chat's contact.
   - Expect: the notification says only "Incoming call", without a name.
   - Lock while it rings: a notification with the name goes to the one without.
4. Close the app's process while it rings (`kill` over SSH).
   - Expect: the notification goes by itself after about a minute, and at the next start at once.
5. A group member starts a call with the phone in your pocket.
   - Expect: "<name> started a group call." in the chat, a notification with it, and "The group
     call ended." once within a few seconds of its end, without the chat open. Nothing but the
     lines for a muted group; a group on mentions only does notify. For a private group, only
     "Private chat". With notification preview off: "A group call started.", no name.
   - With the network off when the announcement comes, and back within a few minutes: the line
     and the notification still come.
6. Start a group call yourself.
   - Expect: "You started a group call." and no notification on this phone.
7. Settings → Notifications → Calls: another ringtone; then Vibrate off.
   - Expect: the call rings with it, at the ringtone's volume; no vibration. **If the system's tone
     still plays**, ngfd's profile lookup wins over `audio` for this event: say so, and the
     ringtone setting goes (it would be a control that does nothing); the vibration switch stays
     if it works.
   - Choose a music file, delete it, get called: the system's tone, not silence.
8. In a 1:1 audio call, tap the speaker button. **Nothing about the switch has been run yet.** The
   SDK shows the route manager's interface but not two names in `bridge/callroute.cpp`, which come
   from memory: the feature `speaker` and the device `earpiece`. During a call, please send the
   output of `gdbus call --system -d org.nemomobile.Route.Manager -o
   /org/nemomobile/Route/Manager -m org.nemomobile.Route.Manager.FeaturesAllowed`, and the same with
   `…Route.Manager.ActiveRoutes`. How each failing looks:
   - `speaker` is wrong, or not offered outside phone calls: no button at all, in any call.
   - `earpiece` is wrong: the button works, but a camera doesn't move the call to the loudspeaker
     by itself, and the button can light with the sound still at the ear.
   - The manager takes it but doesn't move phone-role streams: tap, and three seconds later "The
     sound could not be switched" with the button dark (if the route doesn't change, the sound
     stays), or a lit button with no change in sound (if the route changes without the streams).
     Then the engines must change the stream's role instead.
   - Expect: the sound moves to the loudspeaker and back, the button lit only while it is there.
     Listen for echo on the other side while on the loudspeaker.
9. A voice call from **Threema for Android**, answered here (it says "video" on every call).
   - Expect: "Incoming call", the earpiece, the screen allowed to blank, black at the ear; the
     camera button is there. Then turn a camera on, on either side.
   - Expect: it moves to the loudspeaker with the button lit, and stays there when the camera goes
     off. A group call does this only for our own camera.
10. After a call on the loudspeaker: make a normal phone call; and once kill the process during such
    a call first.
    - Expect: the phone call is on the earpiece. Say if it starts on the loudspeaker after the kill.
11. A wired headset, then a Bluetooth headset, connected before a call and during one, also with the
    speaker button.
    - Note where the sound goes each time, and what the output is named
      (`…Route.Manager.ActiveRoutes`, as above). Say if there is a route you'd want to choose and
      can't: a Bluetooth entry depends on this.
12. Bluetooth headset goes out of range or off during a call.
    - Note where the sound goes. Android has a setting that ends the call then; we only add one if
      the sound ends up somewhere you wouldn't want.
13. Audio call with the phone at the ear (speaker off), 1:1 and group.
    - Expect: the screen goes black and takes no touches, comes back when lowered. Nothing of this
      on the loudspeaker or with a camera on.
14. Leave a call with a camera on running past the display timeout, then an audio call with the display
    allowed to blank, for several minutes each.
    - Expect: the screen stays on with the camera; the audio call's sound goes on with the screen off
      (no gap: the CPU is held awake). An ended call lets the screen blank again.

Screen, in both flavours:

15. A video longer than the display timeout; filming in the camera page; a long voice recording.
    - Expect: the screen stays on for all three; a paused video lets it blank.

Lite:

16. Install Lite after using Full, and have a contact call within the day.
    - Expect: their app says calls are off; a "Missed call" line and notification here.

## Recording and camera stop with the app

Voice recording:

1. Start a recording and leave the phone untouched for two minutes.
   - Expect: the screen stays on and the recording runs on.
2. Group chat → swipe to its page → a member → record there and tap send.
   - Expect: it goes to the member's chat and nothing to the group.
3. Record, then swipe to the contact page and back; the same with a held recording (the bar shows its time and a slider).
   - Expect: "Recording stopped and discarded" on the way there, the recording is gone and the mic
     is back; nothing was sent.
4. Start a recording and have someone call the phone (the phone app, not Threesea).
   - Expect: the microphone is free for the call. If the call only took the focus, back in the
     chat "Recording stopped. Send it or discard it."; if it left the app (the cover showed, or the
     display went off), "Recording stopped and discarded", and
     `ls ~/.cache/org.threesea/harbour-threesea/` shows no `voice-recording.m4a`. Nothing is sent
     by itself; the bar still records if the call took no focus.
5. Record, then close the window (Full), or let the app lock engage.
   - Expect: the microphone is free at once (Jolla's Voice Recorder can record) and no message goes
     out.
6. Record, and have someone call you in Threesea; also with a voice message or video playing.
   - Expect: the recording goes with a notice, what plays pauses and stays paused after the call.
7. Tap the mic and tap again at once (under a second).
   - Expect: nothing sent, no `voice-recording.m4a` in the cache.
8. With no app lock: start a recording, go to the home screen and leave the phone untouched.
   - Expect: the microphone is free at once (Jolla's Voice Recorder can record) and no
     `voice-recording.m4a` is left in the cache; the screen blanks after the usual time and the
     device locks as usual. Back in the chat, "Recording stopped and discarded", nothing sent.
8a. Leaving is the cover showing (a completed side swipe, the home screen) or the display going
    off (the power key): each must stop the recording and throw it away, also a held one (listen to
    it, then leave), with "Recording stopped and discarded" on the way back. Something that only
    takes the focus (an alarm, a low-battery or USB dialog, an incoming phone call shown over the
    app) must stop it and hold it: back in the chat, "Recording stopped. Send it or discard it.",
    and send delivers it. Pulling down the top menu takes no focus: the recording must go on, the
    bar still counting.
8b. Let a recording reach the limit (15 minutes) untouched, then leave the phone alone.
    - Expect: the screen stays on until send or discard is tapped, and the recording isn't lost.
    Then leave the app with it waiting: it is discarded.
Camera and QR scanner:

9. Open the in-app camera, go to the home screen, start Jolla Camera, then come back.
   - Expect: Jolla Camera works; back in the app the viewfinder returns. The same on the QR scanner,
     which goes on recognising codes and keeps the viewfinder smooth while it looks. Closing the
     window (Full) frees the camera too.
10. Record a video, then try to swipe back.
    - Expect: the page doesn't go while recording; the shutter stops it and attaches the video.
11. Record a video and go to the home screen.
    - Expect: back in the app the viewfinder shows, "Recording stopped and discarded" comes up, and
      `ls ~/.cache/org.threesea/harbour-threesea/capture` is empty.
12. Leave a video running without touching the phone for two minutes.
    - Expect: the screen stays on and the video is complete.
13. Take a photo, tap send and go straight back to the chat list; the same with a video.
    - Expect: both arrive, and the capture folder is empty afterwards.

## Files from other people in other apps

Tapping a downloaded file saves it to Downloads, and hands documents, media and contact cards to
the system's app. Send these from another phone as files.

1. A contact card (`.vcf`, Android's "send contact"), then a calendar entry (`.ics`).
   - Note what Sailfish does with each: which app opens, and whether it adds anything without
     asking.
2. A PDF, a JPG, an MP3 and a TIFF.
   - Expect: each opens in its app, or nothing happens if the phone has none for it.
3. A `.desktop`, a `.sh` and an `.rpm`, each with the media type `application/pdf`.
   - Expect: the plain file icon and the real name; a tap saves it to Downloads and nothing opens.
4. A `.txt` whose content is a desktop entry.
   - Note what opens: the text or anything else.
5. A BMP or HEIC photo sent as media, the way Android sends photos.
   - Expect: a file row, or its preview if it came with one; a tap opens it like any other file,
     no empty picture and no black page.

## Large transfers on the real servers

The time limits and retries were only tried against a local server, without TLS. Lower the phone's
network to 3G if it can, or find a weak signal, and use a video that takes more than two minutes.

1. Send it to a contact, then to a group.
   - Expect: it arrives, and the message's clock ends without a warning sign in between.
2. Have the same video sent to you.
   - Expect: it arrives by itself if Settings → Media lets videos come on this network.
3. With a linked device (Threema for Desktop), send a small photo and a large video.
   - Expect: both arrive on the other phone and on the desktop. This goes through the mirror.
4. During an upload, switch on flight mode for one minute, then off.
   - Expect: the upload carries on, and the file arrives once.
5. During another, switch on flight mode for three minutes, then off.
   - Expect: the warning sign, then a new try by itself within a few minutes of the connection coming
     back; a tap on the message tries at once.
6. Close the app during an upload and start it again.
   - Expect: the warning sign for up to a minute, then it goes out by itself.
7. Do the same as in 5 while pictures come in.
   - Expect: they arrive by themselves a little after the connection is back, no tap needed.

## Recording: pause, listen, discard

1. In a chat with an empty input, tap the microphone and speak; tap the pause button after a few
   seconds; wait; tap the microphone in the bar and speak again. Tap send.
   - Expect: the time stands still during the pause, and the message plays both parts without the
     pause and without a glitch at the join.
   - Qt reports a pause at once whatever the pipeline does, so the app watches the recorder's time.
     If it goes on or isn't reported, the log says `the platform did not pause` about a second after
     the tap and the recording stops and waits, with the notice "Recording stopped. Send it or
     discard it."; then please report it: the pause button would need to go. Please also report if
     the time stands in the app but the pause can be heard in the message, and a message that sends
     but doesn't play at the join.
2. Record, pause, tap play.
   - Expect: the bar shows the slider; it plays through the loudspeaker what was said; pause,
     dragging and playing to the end and again work; send then sends all of it.
3. Tap a voice message's play button while listening.
   - Expect: listening pauses; the message plays.
4. Discard a recording over 10 seconds with the ✕: a remorse timer appears and recording pauses;
   cancel it, and send what is there. One under 10 seconds goes at once.
5. While listening, press the power key.
   - Expect: it stops sounding; the recording still waits when the app is back.
6. Send several recordings of different lengths, and listen to one right after stopping it.
   - Expect: each is complete to its last word, at the recipient too, and listening starts at once.
     The file is used only once the platform has finished it. If the log says `the platform did not
     finish the file` (after 5 seconds), the recording waits to be sent or discarded instead of
     going out: please report that, and whether what it holds was cut off.
7. Move the slider of a held recording before the first play, then play.
   - Expect: it starts where the slider is.
8. First emoji search after starting the app: tap the search key and type a letter.
   - Note whether the page hitches. The terms are read in the UI thread at the first search only; it
     stays so unless this shows on the phone.

## Sending a location

1. Turn location off in Settings → Location, then open Send location in a chat.
   - Note what the dialog says at once: "Positioning is not available…" means Sailfish reports the
     switch through `sourceError`. If it keeps saying "Finding your location…", the text about
     checking location must still come after 30 seconds. Send stays unavailable throughout.
2. Turn location on while the dialog is open, outdoors.
   - Expect: the coordinates appear without reopening the dialog.
3. Get a fix outdoors, walk indoors to where no fix comes, wait some minutes, and open the dialog.
   - Expect: no old coordinates; Send stays unavailable. Check that real fixes pass at all: if an
     outdoor fix never shows, the platform's fixes have no valid or a much older time, or the phone's
     clock is more than a minute ahead of them.
4. Stay in the dialog with a fix showing, then cover the antenna or go indoors for over a minute.
   - Expect: the coordinates go away again and Send becomes unavailable.
5. Lie the phone still indoors with GPS off and only network positioning, and open the dialog.
   - Note whether a fix comes at all: only here can it be seen whether network positioning reports
     new fixes to a phone that doesn't move. If none comes, after 30 seconds the dialog offers the
     last known position with its time, if there is one. Choosing it enables Send, and the place row
     says "Your last known position".
6. With no fix showing, tap Place.
   - Expect: the row already says what the map server gets. The list opens and shows places nearby.
     If nothing can be found and the page says the server did not answer although the phone is
     online, see step 9.
7. Search for something by name, also before any position has come.
   - Expect: results with distances; text typed before a position is searched for once it comes.
8. Choose a place and send it, then change it back with "Use your position".
   - Expect: it arrives at the recipient with its name and coordinates, and opens in a maps app.
9. TLS: `poi.threema.ch`'s own certificate hasn't been seen. Our client wants the pin on the leaf,
   where Android accepts one anywhere in the chain, so a leaf outside the set fails only here, as
   "did not answer". If that happens online, report it: the log has the network error's kind.
10. Lock the app with the place list open, wait, and unlock (app lock on).
    - Expect: the location indicator goes off while locked, and old coordinates don't flash on return.

---

# Added 2026-11-05 (what the fixes after the audit of 2026-11-04 leave to a phone)

Short: what to do, then what you should see. Some need a second phone with Threema, Threema for
Desktop linked, or Threema Web open in a browser; it says so. The emulator can take the ones that
need neither a sensor nor a real network.

## Writing and sending

1. Paste over 6000 bytes of text and send; the same with an over-long caption on a photo.
   - Expect: a notice; the text (and the photo with its caption) stays, also after reopening the
     chat. Sending normally empties the input at once, the keyboard stays, the button doesn't flicker.
2. Pick a photo, then Edit an earlier message and send.
   - Expect: the message is edited; the photo stays picked with its caption back in the input.
3. Quote, then attach a file.
   - Expect: the quote bar goes; with a file picked the bubble menu has no Quote, and still lays out well.
4. Open a chat that has a draft.
   - Expect: the other side sees no "typing"; typing for real still sends it.
5. In a group, type a mention, leave, come back, send.
   - Expect: it arrives as a mention; the chat list's draft preview shows @Name.
6. Type, wait a second, kill the app (`kill` over SSH).
   - Expect: the text is back when the chat opens again.
7. With a photo picked, or recording, or a recording held: try the back swipe.
   - Expect: nothing happens, and the page doesn't look stuck; after the X it can be left. Picked
     capture, then a notification tap to another chat: the discard notice shows.
8. Quote, type, leave, come back.
   - Expect: text and quote bar are back, and the keyboard doesn't open by itself.
9. Film a video, send, go back within a second; then the same in a private chat opened from a
   notification.
   - Expect: it arrives once, with its caption, and nothing is left in the capture cache; the
     private chat asks for the PIN when opened again.
10. Group chat → group page → a member's chat → back twice, then have a group message arrive.
    - Expect: the quiet in-app sound for it.
11. Type a draft, delete the contact (or group) from its page, write to them again later.
    - Expect: the input is empty.
12. Type in front of many emoji in the input.
    - Expect: the pictures stay in place and aligned, no lag.
13. Landscape: open the emoji panel with the smiley and with a bubble's +, turn the phone with it
    open, then open it in portrait after a landscape keyboard.
    - Expect: messages and input stay visible, the switch from keyboard to panel doesn't jump.
14. Emoji panel: search, pick from the results, use the skin-tone menu in the two-row grid; close it
    with a swipe down and with the keyboard button; the same when adding a reaction.
    - Expect: keyboard and panel lay out well and the input keeps the focus.
15. Bold, italic and struck-through messages, also with links and emoji.
    - Expect: link colour, emoji alignment and the time's place look right.
16. The attachment menu on a small phone and in landscape.
    - Note whether six items fit well; the user decides whether six stay.

## Reading a chat

1. Emulator: in the ECHOECHO chat, more than a screenful, at the newest end with the keyboard
   closed, long-press the newest bubble and (a) tap the dimmed area, (b) choose Copy, (c) tap a quick
   reaction. Then scroll a few screens back and repeat (a).
   - Expect: after about a second the input and the microphone are fully in view and the bubble is
     where it was; scrolled back, the view doesn't move.
2. React to, or get a read receipt for, a message far back.
   - Expect: it stays in its place and the view doesn't move.
3. Scrolled back, with and without the keyboard, have a message arrive.
   - Expect: the view stays, a "New message" notice shows, the keyboard stays; typing or tapping
     the input brings you to the newest. The chat list shows it unread when you leave.
4. Minimise on an open chat, receive several messages, come back (also over the attached details
   page).
   - Expect: the unread line at the top of the view.
5. Tap a quote of a message older than what is loaded, and one whose message was deleted.
   - Expect: the first opens there and lights up; the second says "Message not available".
6. A search hit far back in a long chat.
   - Expect: it opens quickly; scrolling forward reaches the newest.
7. Scroll back, leave, reopen.
   - Expect: the same place (on a tall screen too); with something unread, the line. Forward or
     share into that chat: it opens at the input with the X in view.
8. Edit or delete for everyone a message someone quoted.
   - Expect: the quote changes at once, also when the original is out of the loaded part.
9. Landscape, keyboard open, in a chat whose last message was yesterday.
   - Expect: the day band says Yesterday.
10. Leave a busy chat open for a long time.
    - Expect: memory stays flat.
10a. Full: a voice message downloads while its chat is open; raise the phone to the ear.
    - Expect: it starts.
11. Flick through a chat with many photos, also ones without size or thumbnail.
    - Expect: thumbnails while moving, sharp pictures soon after it stops, no flash or jump in
      height at the swap.
12. Delete a message, load older pages, receive new ones.
    - Expect: day headers and bubble runs stay right; a GIF stops when scrolled out and plays when back.
13. Chat background with a landscape and a portrait photo, in both orientations.
    - Expect: sharp, filling the page.
14. Change emoji style, text size and GIF playing in Settings with a chat open; then remove the ID.
    - Expect: the chat follows at once; after removing the ID they are back at their defaults.
15. First opening of the smileys and people tabs; the app's start on a phone.
    - Expect: the tiles appear without popping in one by one, no hitch at the first emoji shown.

## Message actions

1. Tap and use the menu on a failed text, poll and file.
   - Expect: it goes again; recipient and linked device show one message.
2. Stop a large upload and a large download on a real network.
   - Expect: data really stops flowing; the viewer has no stop item.
3. Tap a cleaned-up file that the server no longer has.
   - Expect: one notice.
4. Select messages: the five icons and the fading count on a small screen and in German; Select all
   over several pages; save and delete many.
5. Share a message's text through the share dialog.
   - Expect: the text arrives in the chosen app.
6. Long press, by real touch, on a reaction chip (also in a group with former members), on a starred
   row, and on a quote.
   - Expect: who reacted (chip), Remove star (starred list), the bubble's menu (quote), each with
     its highlight.
7. A blocked contact's chat and a left group: long press, chips, the poll page.
   - Expect: only what such a chat can do, no push-up menu.
8. A distribution list: long press a message.
   - Expect: no reaction row.
9. Remove a link on your profile.
   - Expect: the remorse shows on the profile page after the dialog closed.
10. Tap a received file named like `Invoice #12.pdf`.
    - Expect: it opens in Documents.
11. A chat with polls: the banner and the polls page; tap a row to vote, Show in chat.
    - Expect: the banner's count follows.

## With another app on the other side

1. Notes group: react with a heart, edit an old note, delete one; with Desktop linked too.
   - Expect: each change reaches Desktop; Delete for everyone only then.
2. Edit a message while the connection drops in the middle of sending.
   - Expect: you and the other side end up with the same text.
3. Edit a message that quotes another, towards Android.
   - Expect: no quote marker in their bubble; the quote stays.
4. An Android contact edits a photo's caption.
   - Expect: the new text with Edited on the bubble.
5. Only if an old client or a gateway is at hand: their picture, voice and video messages.
   - Expect: they open.
6. A thumbs up in a group with a member on an old client.
   - Expect: it arrives there as an acknowledgement.
7. Forward secrecy, with forward secrecy off or Desktop linked: a contact on an old app sends.
   - Expect: their message comes again without forward secrecy.
8. A contact links Desktop, later unlinks and writes.
   - Expect: one "off" line, then plain messages that get delivered; after unlinking it comes back.
9. Someone new writes first from Threema for Android.
   - Expect: no security notice; their first receipt and your reply arrive.
10. An Android member's group message after a lost session.
    - Expect: Android offers to send it again.
11. A chat with a forward secrecy session: the status lines read right. A backlog after being
    offline, from such contacts: all arrive, no notices.

## Media, viewer and pictures

1. A real camera JPEG sent as a file.
   - Expect: right in the bubble, the gallery and the viewer.
2. Play a voice message from the media gallery, go back.
   - Expect: the bubble no longer glows and at-the-ear playback skips it.
3. A damaged picture in the viewer.
   - Expect: a readable, centred label.
4. Hide videos in one chat's gallery, then open a chat with only videos.
   - Expect: the pull-down offers Show, and it brings them back.
5. The picture viewer by real touch: swipe between pictures, swipe down for back, pinch and double
   tap (zoom doesn't move the slideshow), pinch back to fit, the overlay in a light Ambience, big
   photos, a video's play button and the way back, Show in chat from a chat and from the gallery,
   opening it in a chat with thousands of pictures.
6. Photos, stickers, profile pictures, GIFs and WebP from others, in chats and the gallery; a BMP or
   HEIC sent as media.
   - Expect: they show and play; the BMP or HEIC is a file row or its preview, and opens on a tap.
7. A chat's gallery with several hundred media.
   - Expect: scrolling loads more without a jump; thumbnails appear while downloading; delete removes
     the row; a new file appears; the filter still works.
8. Receive and send a video near 100 MiB; then send a file and lock; close the window right after
   sending.
   - Expect: the chat list and scrolling stay smooth, the connection stays up; memory on an older
     phone with this and three large downloads stays bearable.
9. Open a large video.
   - Expect: the page at once with a busy indicator, then it plays; leaving meanwhile works; a call
     or a lock during the read stops it.
10. Draw on a photo: lines under the finger, text, turning; then "Draw on it" on a received picture,
    with its countdown, cancelling it (back to the editor) and leaving a private chat during it; the
    colour page returns to the editor.
    - Expect: the sent picture looks like the preview.
11. A large camera photo through the editor, and one taken in the camera page.
    - Expect: a busy indicator over the attachment while it is edited.
12. A profile picture and a group picture: from the gallery and with the camera, cut to a square;
    on a bad connection for the group picture; an undecodable picture.
    - Expect: an error notice on failure, the new picture at once on success.
13. Forward a photo with a changed caption, and one with a mention.
    - Expect: the caption arrives; the mention stays a mention.
14. Share a picture and a video into two chats, and into several with a private chat among them;
    close the window during it with "Keep running" off.
    - Expect: thumbnails and lengths right; the PIN for the private chat, which is hidden again
      afterwards; the rest still arrives, then the process ends; a failure with the window closed
      shows a notification.
15. Share a video and a GIF from Gallery.
    - Expect: they play at the recipient, without metadata.
16. Share a link from the Browser.
    - Expect: offered once; the share page opens with the address; other apps' text still works.
17. With a memory card: the file picker lists it, files on it can be attached, a data backup on it
    imports.

## Chat list, cover, contacts and search

1. Type in each search field (search page, contacts, share page, member picker).
   - Expect: the keyboard stays, rows narrow, the page rests at its top; found chats and contacts
     open; picked members stay picked while filtered.
2. A search result with joined emoji and a long text.
   - Expect: pictures in place, the match highlighted, three lines.
3. Lock now with a long grace time.
   - Expect: the unlock page at once. Offline with a lock: New group is on the Contacts pulley.
4. Mute and unmute from a chat's menu.
   - Expect: the silent icon follows.
5. The cover with something unread.
   - Expect: both actions; the right one opens the chat. With the lock or "Hide content on the
     cover" only the new message action.
6. A chat pinned long ago.
   - Expect: its date shows and the name still fits.
7. The chat list itself.
   - Note how the taller rows, larger pictures and three lines with emoji look; the user may prefer
     two lines.
8. With the window closed (Full) and the list hidden: messages, reads, renames.
   - Expect: the cover's count and name follow; the list is current on coming back, with the newest
     delivery icon. A chat marked private on a linked device while the app is in the background is
     not shown on coming back.
9. Delete a contact with a chat, from the list and from its page.
   - Expect: the chat goes, the page returns past it, the cover's count falls.
10. Edit the name of a blocked, pinned contact with a picture.
    - Expect: the page keeps all of it.
11. Contacts named with accents (an Ä, É, Ö), an emoji name and a name starting with a digit.
    - Expect: the accented ones under their letter, one # section at the end.
12. Tap a group member's picture.
    - Expect: Add to contacts works, no Delete; after leaving the group, Delete is back.
13. Show profile pictures off, then on.
    - Expect: all lists and pages lose and regain the pictures at once, group call tiles too.
14. Address book sync on: delete a matched contact, pull to refresh; then add it by hand.
    - Expect: it stays away, then it stays.
15. Settings → Privacy → Numbers without a country code: open it, choose, change.
    - Expect: a page showing Automatic (+code); the choice is kept; a change starts a sync with its
      notice.
16. Change a matched contact's name in People and sync; rename it in Threesea and sync.
    - Expect: the first follows, the second stays.
17. An address book entry with only a company and a linked number.
    - Expect: added under the company's name.
18. First sync of a large address book; a manual sync offline.
    - Expect: it finishes quickly; offline it says why.
19. Add members to a group that already holds some or all contacts.
    - Expect: no empty letters; a placeholder when everyone is in.
20. New group and New distribution list with many contacts, also in landscape; both name dialogs in
    landscape with the keyboard open.
    - Expect: title and name field visible, keyboard on the name, Create reachable, the field
      reachable.
21. On a group page, pull down without flicking first.
    - Expect: the menu with Leave group opens.
22. During a member's or browser's removal countdown, let a contact or browser change arrive.
    - Expect: undo still works.
23. A 24-hour phone: Last active of a linked device.
    - Expect: 24-hour time.
24. Your own QR code with the phone sideways.
    - Expect: the whole code shows and scans; the text below is reached by scrolling.
25. The photo editor: pull down from its margins.
    - Expect: the menu opens; dragging the frame and its grips works as before.
26. Long press a joined group, a left group and a list in the chat list.
    - Expect: the menu and its countdown read right for each.
27. A German phone: older day headers.
    - Expect: "Mittwoch, 14. Oktober".
28. Without a camera (emulator): the QR page.
    - Expect: only the middle text.
29. Scan a contact's code whose key differs from the one stored.
    - Expect: the longer notice reads fully.
30. With a real directory: let the daily contact refresh run.
    - Expect: `contacts refreshed` in the log; feature masks and states stay right.

## Groups

1. Be added to a group by someone who isn't a contact; then by someone on your block list.
   - Expect: the group appears with them as a contact; from the blocked one nothing appears.
2. Offline while a stranger sets up a group and writes in it.
   - Expect: after reconnecting the group and all messages arrive.
3. A group with a revoked ID in it: write.
   - Expect: your text goes from clock to sent; with one member stuck, Delete for everyone appears.
4. Offline: create a group and write; offline: write, then leave.
   - Expect: the members get the text in both cases.
5. Be removed by an Android creator.
   - Expect: "You are no longer a member of this group."
6. Add someone to a group with a picture; during a group call, add a member who can do group calls.
   - Expect: they see the picture; they see the banner and can join.
7. Block a member of a shared group, then write and rename.
   - Expect: the text reaches the others, not them; the rename reaches them too.
8. With "Block unknown senders" on, call someone who isn't a contact.
   - Expect: the end screen gives the reason at once.
9. A blocked contact writes with forward secrecy.
   - Expect: no chat moves up, no status line.
10. A chat with a revoked ID.
    - Expect: "This Threema ID was revoked" in place of the input, no Call in the pulley.
11. During a 1:1 call, open a group chat from its notification.
    - Expect: the group call items and banner are off.

## Linked devices (Threema for Desktop)

1. Scan a contact's code, add a contact, delete one, run the address book sync, link and unlink a
   number.
   - Expect: each shows on Desktop; a revoked ID shows as revoked there.
2. Import a data backup.
   - Expect: its contacts and groups appear on Desktop, and a later message in such a group lands in it.
3. With Privacy, the block list and the profile open on the phone, change the same on Desktop.
   - Expect: the open page follows.
4. A member sends a group sync request; add someone to a group with a picture.
   - Expect: they get the picture (also Desktop's, if Desktop set it); a removed picture doesn't
     come back.
5. Drop the connection while Desktop changes a picture; set a picture offline on the phone.
   - Expect: after reconnecting the other side has it; a contact you write to from Desktop gets the
     phone's own picture.
6. Desktop changes the members of someone else's group; Desktop leading, someone renames a group
   or creates a new one.
   - Expect: the creator stays on the phone and their messages arrive; the changes reach the phone
     through Desktop alone, and the phone sends no answers.
7. Close Desktop; meanwhile be added to a group with someone new, renamed, given a picture, leave
   it, be removed from another. Reopen Desktop.
   - Expect: Desktop shows all of it.
8. Link Desktop with someone else's group, with a removed group, and with a left group whose
   member's contact was deleted.
   - Expect: linking works; the creator shows once; removed shows as removed, not left.
9. Change members on the phone and on Desktop.
   - Expect: each side shows the lines once.
10. Delete a left group on the phone, and one on Desktop.
    - Expect: it disappears on the other side.
11. Desktop accepts what the phone reflects (nonces, receipts, pictures); messages still arrive.
12. Delete a contact on the phone with Desktop leading; have them write.
    - Expect: chat and contact come back on the phone.
13. With Desktop leading and a chat open on the phone, the contact types.
    - Expect: "typing" shows; nothing stale after logging in.
14. Read a chat offline and reconnect; read a group and a chat with receipts off.
    - Expect: the sender sees read where receipts are on; Desktop shows all of them read.
15. Airplane mode while messages arrive, then back.
    - Expect: the notification shows; the sender sees delivered after reconnecting; Desktop still
      gets what came during the stalled link.
16. Reply in an archived chat.
    - Expect: it leaves the archive, here and on Desktop.

## Threema Web (a real browser)

1. On the phone: add a contact and a group, rename, block, make a chat private and back.
   - Expect: the browser shows each at once.
2. Delete messages and empty a chat on the phone.
   - Expect: they leave the browser's open chat.
3. A contact's new picture; your nickname and picture changed on the phone and in the browser.
   - Expect: the browser follows, the other pictures stay; profile and edit pages show yours.
4. Mute a chat; a group on mentions only.
   - Expect: the icon in the browser, no desktop notification.
5. Mark as unread on the phone; open that chat in the browser; read a chat in the browser, switch
   away and back.
   - Expect: the mark clears on the phone; the unread divider stays gone.
6. Edit a contact's first name in the browser.
7. A GIF, a photo sent as a file, a video and a voice message in the browser.
   - Expect: the GIF plays, the file saves with its name, both lengths show; the message history
     entry shows correct times.
8. Send a JPEG from the browser to Android and to Threesea; a video, a GIF and an animated WebP;
   forward a received picture; forward a HEIC; forward many pictures.
   - Expect: upright, without EXIF, at the Image size setting; the rest plays; the refusal's alert
     stays visible; the app stays responsive.
9. Connect a browser (also after a push wake-up), a second one, dismiss and "Clear all".
   - Expect: one notification, the count follows, it comes back once, stays under the app lock,
     closes with the last browser, the switch and (Lite) the window; tapping it opens the sessions.
10. Allowed servers: the list, its menu and remorse; with a self-hosted server, a renewed
    certificate with a fresh scan and with a stored session woken by push: accept and decline.
11. Delete a session.
    - Expect: the browser forgets it.
12. A real disconnect; the browser's network changing.
    - Expect: a readable notice; the session waits and reconnects.
13. The sessions page: hint and header; rename a session; Remove all with a browser connected, and
    leaving the page during its countdown.
    - Expect: the new name; the browser shows it was removed, the notification closes, the browsers
      are removed even when you left.
14. Threema Web open while messages come in.
    - Expect: the chat list updates, thumbnails arrive, and the phone stays cooler than before.
15. A real server notice, if one ever comes.
    - Expect: it shows in the browser.

## Notifications and alerts

1. App lock on: let the grace time pass with a notification showing; then a new message.
   - Expect: the notification turns into "Threesea is locked" without a sound; the new one alerts
     with no name, text or Reply. The phone asleep through the grace: the text is gone within about
     30 s of waking.
2. Keep running off, app lock on: close the window.
   - Expect: the text leaves Events.
3. Passphrase on, `kill -9`, start.
   - Expect: name and text gone before the passphrase.
4. A reply typed on the lock screen, a new message in that chat, the grace ends, the phone unlocked.
   - Note whether the reply waits for the PIN or says it wasn't sent.
5. Reply from a notification after `pkill` and `pkill -9` before the PIN or passphrase.
   - Expect: "Your reply was not sent", nothing promised.
6. Reply to a blocked contact from a notification; the same while that chat has an edit open.
   - Expect: a notification with the reason; the chat stays unread; the text is in the chat's input
     after a tap (after the edit ends).
7. The sender deletes or edits the shown message.
   - Expect: the text changes without sound, vibration or banner; the next message alerts as usual.
8. Restart with a notification showing, then a new message.
   - Expect: it replaces the old one in place, and Reply works.
9. A backlog after being offline; a busy group; a missed call right after a message; a new message
   after dismissing a chat's notification.
   - Expect: one sound for the backlog, one per burst in the group with count and text updating,
     the missed call still alerts, the next message alerts again.
10. A refused login or a server alert, with the window closed.
    - Expect: the text under the chat list's header, a notification (phone in the pocket too), one
      per text. Long texts fit the header.
11. Two devices taking turns with one ID.
    - Expect: the header's text, one alert only; the device cookie notification reads fully.
12. The cover in each state.
    - Note whether "Connecting" flickers on a phone that reconnects often.
13. A storage problem while receiving (a nearly full disk).
    - Expect: the notice and the chat list's line, gone again once storage works.

## Connection, battery and transfers

1. Leave Wi-Fi while connected; come out of a dead zone; unlock after a long sleep.
   - Expect: Offline or reconnected within about 20 s; reconnected within seconds; and with the
     screen off a network change keeps the device up through the login.
2. The ID active on another device.
   - Expect: no login from Threesea for 10 minutes on its own; Connect now still connects.
3. Pocket test: a socket the network has dropped.
   - Expect: back online within the same wake-up.
4. A whole day: battery and awake time; a message with its notification still arrives within a
   wake-up; a larger picture on a slow link finishes over several; the license check and Keep
   messages still run overnight.
5. A network that drops IPv6.
   - Expect: connected over IPv4 after about 6 s. With IPv6 off on an IPv6 network, a 1:1 call
     connects over IPv4.
6. The blob server out of reach while the chat server works.
   - Expect: messages go at once; contacts get your picture once it is reachable again.
7. Send a large video, delete it during the upload, send another at once.
   - Expect: the recipient gets only the second.
8. A backlog of pictures after a long time offline.
   - Expect: each downloads once, none turns failed.
9. Clean up → media files, restart.
   - Expect: nothing downloads again; tapping one fetches it.
10. A group file you sent, fetched by a member long after another member did.
    - Expect: it still downloads.
11. A sent file.
    - Expect: it goes out once its upload finishes.
12. A stalled mobile link.
    - Expect: the start page ends at once; notifications and ringing still arrive during a Threema
      Safe backup, a linking attempt and a chat export; linking and joining a group call still work.
      A sleeping Full phone with an overdue round stays up to the 45 s limit.
13. Cold start with a long history, and the upgrade of a real database.
14. Search for a common word in a long history.
    - Expect: typing stays responsive and the keyboard stays.
15. Import a backup with a large list of used nonces and a long chat while connected.
    - Expect: the connection holds, Stop answers.

## Start, storage, lock and the end of the process

1. Decline or break Sailfish Secrets at the start.
   - Expect: the page with the secrets text and a code; Try again recovers; closing ends the
     process; a start in the background leaves a notification.
2. Kill the app while Sailfish Secrets' confirmation prompt is showing at the start (Full: the
   background instance after boot), then start it again.
   - Expect: the prompt again, and the app starts once it is confirmed. Note anything else, such as
     `secrets read` in the log with no prompt (docs/emulator.md has the emulator's case).
   - Full, with the background run up: `systemctl --user restart lipstick` in the terminal.
   - Expect: Threesea is running again without a window shortly after the home screen is back, and
     no second prompt. The same after `pkill -KILL lipstick`.
3. A damaged database.
   - Expect: the chat list, an unreadable chat and scrolling into a damaged part each say so once.
4. Open with the phone's storage full.
   - Expect: the full-storage text; Try again recovers after freeing space.
5. Upgrade from 0.2.0 with a large store.
   - Expect: a hint while it upgrades, gone once started.
6. Encrypt stored data on and off.
   - Expect: it works and asks for nothing extra.
7. A start-up passphrase of digits only; and the PIN setup's note on the keypad and keyboard steps.
   - Expect: title and wrong-code text fit their purpose; the note reads well.
8. A PIN set with "Lock the app" off.
   - Expect: Settings, Security and App lock read "PIN for private chats", and the value fits.
9. Update with `rpm -U` while the app runs without a window, and with one; remove Lite while it
   runs without a window.
   - Expect: without a window it ends within minutes and a notification opens the new version;
     with one it says so and ends when closed; after a removal the process is gone within about
     35 s, with no notifications left. Note whether the launcher refuses a second start while the
     old process lives.
10. Keep running off, app not running: Reply on a left-over notification; once in flight mode.
    - Expect: it sends and the process ends within seconds; in flight mode it says the reply goes
      out at the next open, and it does.
11. Swipe the window closed, and minimise it to the cover.
    - Expect: closing ends (Keep running off) or keeps running; minimising never ends it.
12. Keep running off: close during linking, and right after sending several photos.
    - Expect: gone within about 6 s; the next tap starts normally; all photos and the last text are
      there at the next start. A file taking over 3 s to prepare shows a notification.
13. Remove the ID, with and without the passphrase, also killing the app during it, also with a full
    disk.
    - Expect: the app starts as new with a new database; a copy of the old file doesn't open; a kill
      finishes at the next start; on failure Try again, and closing ends the process.
14. After removing the ID.
    - Expect: Events and the lock screen show nothing of the old ID; the settings are back at their
      defaults; a new ID doesn't use the address book until switched on.
15. Remove the ID with Desktop linked, online and in flight mode.
    - Expect: online the desktop is unlinked; offline the ID goes, a notice says the desktop stays.
16. Remove the ID during a data backup with large media, and during a 1:1 and a group call (Full).
    - Expect: nothing (not even a `.part`) stays in the backup folder; calls end at once for both
      sides; the next ID can call.
17. The removal dialog in portrait and landscape.
    - Expect: it reads well and scrolls.

## Backups, Threema Safe and the ID

1. Turn Threema Safe off online, then in flight mode (once staying on the page, once swiping back).
   - Expect: online the backup is gone (a restore says not found); offline a notice both times, and
     Safe is off.
2. Restore a Threesea-written Safe backup on Android, and an Android one here.
   - Expect: chats with contacts listed there; empty chats in order here, private ones only in the
     private list.
3. Restore with groups.
   - Expect: Android members see the update; no wave of new chats or forward secrecy lines;
     contacts' first messages arrive without a rejected re-send; pictures come back for contacts with
     chats, with one request each.
4. Change the linked number.
   - Expect: "+new (not confirmed)"; cancelling leaves it not linked.
5. Enable and disable Safe from Settings → Backup, and let an upload fail.
   - Expect: the row follows on swiping back; "Last attempt failed" reads well. The change-password
     dialog reads well.
6. Create an ID.
   - Expect: nickname, then Threema Safe, then the update question, one at a time. Enabling Safe with
     no network: a notice, the row says Off and the question comes again next start. The reminder
     notification, its tap to the Safe page (also with the app closed and with a PIN), and its
     removal after a backup.
6a. Threema Safe failing for over a week (no network to its server).
    - Expect: a notification with the days since the last backup; its tap opens the Safe page (also
      with the app closed, and with a PIN first); it goes after a successful backup, also after
      "Back up now".
7. On a phone with no ID: restore from a data backup.
   - Expect: file picker, restore, progress over the chat list, results page, back to the chat list,
     then the nickname question. Kill the app during the import: a hint at the next start, which
     opens Data backup or can be dismissed.
8. Find my ID on the Safe restore page, with a linked number and address.
   - Expect: the ID lands in the field.
9. Scan an ID backup's code at the first start; share a backup by e-mail, Messages and Bluetooth;
   cancel the share sheet; scan one from Threema for Android.
   - Expect: it goes as text and arrives intact; the Android one restores. Note what a cancelled
     share leaves.
10. Copy an ID; hold the ID on a contact or your profile; share it by SMS and e-mail.
    - Expect: a "Copied" notice; the menu opens; the link arrives intact.
11. Write message from a contact page opened on its own, and for a private chat's contact while
    hidden.
    - Expect: the chat opens; the PIN first for the private one.
12. Both restore pages while working.
    - Expect: the back swipe is blocked.
13. The pull-down back to the license page, and a `threema://license` link on the setup page.
    - Expect: the page, and the notice.
14. After an ID backup restore.
    - Expect: the nickname dialog once, also after the update question.
15. Threema's verification SMS.
    - Note whether Jolla Messages makes its link tappable, and whether a tap confirms the number.
16. Restore an ID offline with the profile open.
    - Expect: "Not checked yet", which changes when the connection returns.
17. Data backup: create one with long rows on the result page; on a nearly full disk; reopen the
    page during a backup; passwords that don't match; Daily with nothing set.
    - Expect: "Import failed" or "no room" where it should; the progress bar on reopening; the
      mismatch and the disabled choice look right; Daily stays Off with its text.
18. Stop an import of a backup without media, import again, tap a file.
    - Expect: it tries to download.
19. A contact added by an import.
    - Expect: edit and delete shortly after, when online.
20. A backup with a skipped group.
    - Expect: the result page's rows and headline say so.
21. Clean up a lot of messages.
    - Expect: the Messages figure drops, a busy label meanwhile; note the time and memory on armv7.
22. Clean up with a pinned and a drafted chat.
    - Expect: both stay listed, the draft in the input.
23. Keep messages: pick 1 week (countdown, old value shown, cancel); 1 week then For good during the
    countdown.
    - Expect: unchanged after cancel; For good stays.
24. The Clean up dialog's longer texts.
    - Expect: they fit and read well.
25. The chat list's day sections and times with real data.
26. Check for updates now while offline.
    - Expect: a notice; the daily check stays silent.
27. Save the support package with Downloads missing or full; a failed photo and a failed recording.
    - Expect: the fixed sentences, and the log line has the code.
28. Settings → About.
    - Expect: the two licence buttons open readable pages; the source link opens the browser.
