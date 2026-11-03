# Screenshots

Taken in the official emulator with made-up demo data, so they show no real person's chats. Update
the table whenever one is retaken; an older version in it means that picture may be out of date.

| File | Shows | Version |
|---|---|---|
| `01-chats.png` | Chat list: pinned chat, day sections, unread counts, previews | 0.2.0 |
| `02-chat.png` | A chat: location, quote, edited and deleted messages, reactions | 0.2.0 |
| `03-chat-media.png` | The same chat further up: GIF, voice message, file, video | 0.2.0 |
| `04-group.png` | Group chat: a mention, emoji, a picture with a caption | 0.2.0 |
| `05-contact.png` | Contact page: card, notifications, privacy, public key | 0.2.0 |
| `06-message-menu.png` | A message's long-press menu | 0.2.0 |
| `07-emoji.png` | The emoji panel | 0.2.0 |
| `08-cover.png` | The cover on the home screen: unread count and who wrote | 0.2.0 |
| `09-launcher.png` | The launcher icon among system icons (top of the screen only) | 0.2.0 |
| `10-settings.png` | Settings, scrolled past "My profile" | 0.2.0 |
| `11-spam-banner.png` | The spam report's banner in a chat from an unknown contact | 0.2.0 |
| `12-spam-report.png` | The report dialog, saying what is sent | 0.2.0 |

All were taken on the Full flavour, i486, Sailfish OS 5.1.0.11, at 768×1280 with the Ambience "water".
The wallpaper in `08-cover.png` is the Ambience's. `09-launcher.png` is cropped; `docs/emulator.md`
explains how to run and drive the emulator.

## Retaking them

**The emulator's app holds a real test ID and real chats. Nothing of that may appear in a picture.**
Check every picture before committing it. In particular:

- "My profile" at the top of Settings shows the ID, so scroll past it.
- Real chats stay in the chat list next to the demo chats, and real contacts in the contact list.

1. Build and install: `scripts/sdk-build.sh i486 && scripts/emulator-deploy.sh`.
2. Stop the app and copy its data aside:
   `cp -a ~/.local/share/org.threesea/harbour-threesea ~/threesea-backup` on the emulator.
3. Start the app, confirm the Secrets prompt at once if it appears (left waiting, it times out and
   the app reports a missing database key), and run `scripts/emulator-demo.sh remove`, then `add`.
4. Long-press each real chat in the list, Delete chat. This changes the copy in use only.
5. Choose the Ambience, as `defaultuser` on the emulator:
   `gdbus call --session --dest com.jolla.ambienced --object-path /com/jolla/ambienced --method
   com.jolla.ambienced.setAmbience file:///usr/share/ambience/water/water.ambience`
6. Open what each picture shows, then `scripts/emulator.py shot docs/screenshots/<file>.png`. Chats
   open over D-Bus with the `openChat` method (`docs/emulator.md`):
   - `DEMO0006`, one of every kind of message, for `02`, `03` and `06`;
   - `DEMO0001`, swipe left, for `05`; `DEMO0002`, the smiley, for `07`;
   - `g:DEMO0001:dea0000000000001` for `04`; `DEMO0009`, the unknown contact, for `11` and `12`.
   Take `01` before opening the chats, so that they are still unread, and after deleting the
   `DEMO0009` chat, so that the overview doesn't lead with a spam message. Take `08` after an edge
   swipe (`scripts/emulator.py swipe 2 640 600 640`).
7. **Never accept the report on a real ID.** Accepting it sends a real report to Threema. `DEMO0009`
   doesn't exist, so the server refuses that one.
8. Stop the app, put the copy back, and restore the Ambience.
