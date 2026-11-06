# UI guidelines — making Threesea feel like a Jolla app

Goal: Threesea should feel at home among Sailfish's own apps. No custom
widgets, colours, fonts or animations where Silica has an answer. When in doubt, copy what Jolla
does. Sources are listed at the end; re-check them before building a new page.

## Ground rules

- **Silica only.** `Page`, `Dialog`, `SilicaListView`, `SilicaFlickable`, `ListItem`,
  `PullDownMenu`/`PushUpMenu`, `ContextMenu`, `RemorseItem`/`remorseAction`, `ViewPlaceholder`,
  `PageHeader`, `DialogHeader`, `SectionHeader`, `DetailItem`, `TextSwitch`, `ComboBox`,
  `TextField`/`TextArea` with `EnterKey`, `IconButton`, `CoverBackground`/`CoverActionList`.
- **Theme for everything.** Colours (`Theme.primaryColor` for interactive, `Theme.highlightColor`
  for static text/headers, `Theme.secondary*` for less important), sizes (`Theme.fontSize*`,
  `Theme.itemSize*`, `Theme.iconSize*`), spacing (`Theme.horizontalPageMargin` left/right,
  `Theme.paddingSmall/Medium/Large`). Never hard-code pixels or colours. Pressed state →
  highlight colours. Must look right in light and dark ambiences.
- **Theme icons** (`image://theme/icon-m-…`) before own icons. Own icons only where the theme has
  none (e.g. delivery/read state), drawn in the same monochrome style and tinted via Theme.
- **Navigation:** page stack + swipe back. No hamburger menus, no tab bars, no floating action
  buttons. Avoid deep stacks; `pageStack.replace()` for flows (onboarding).
- **Long lists of names** (contacts, choosing a chat or members) get a `SearchField` in the list's
  header, narrowed by `ListFilter` in the model (`currentIndex: -1`, or the first row takes the
  field's focus). The Search page finds chats and contacts by name through the core (`findChats`),
  above the messages.
- **Pulley menus:** ≤ 4 items (Jolla apps use 1–3); most-used item closest to the list. One
  exception: the chat's attachment menu (push-up) has up to six. Silica's image, video and content
  pickers are separate pages, and one picker for all would put a step before the photos.
  What gives way on the chat list stays one swipe away on the Contacts page: New distribution list
  while offline or with an app lock, and offline with an app lock also New group, for which the
  Contacts page's Refresh (no use offline) makes room.
  Pulleys are for **actions**. Details of the current object (e.g. the contact of a chat) are an
  **attached page** (`pageStack.pushAttached`), reached by swiping left — like Jolla Messages.
- **Dialogs:** accept = swipe forward / DialogHeader, cancel = swipe back. No OK/Cancel buttons.
- **Lists with a header whose height changes** get `components/StayAtTop.qml`: ListView leaves
  `contentY` alone when its header grows, and a list resting past its top has a pull-down menu
  that doesn't open. `scripts/test-pages.sh` checks where lists come to rest.
- **Destructive actions:** `remorseAction`/`RemorsePopup`, never a confirmation box.
- **Every scrollable view** gets a `VerticalScrollDecorator`. Empty views use `ViewPlaceholder`
  with text + hint.
- **Text input:** `label` and `placeholderText`, `EnterKey.iconSource`/`onClicked`.
- **Orientation:** `allowedOrientations: Orientation.All` on content pages; check landscape.
- **Display cutouts (notch):** Silica 5.1 handles them itself for standard layouts (verified in
  the SDK target's `Page.qml`/`PageHeader.qml`): in landscape every page is shrunk and shifted
  away from the cutout (`CutoutMode.AvoidLandscapeCutout` is the window default), and in upright
  portrait `PageHeader`/`DialogHeader`, pulley menus and remorse popups pad themselves by the
  public `Screen.topCutout.height`. A **custom** top layout (chat header, profile card, PIN
  entry) gets no padding in portrait: add `components/CutoutInset.qml`'s `inset` to its top
  spacing. Full-screen media pages set `cutoutMode: CutoutMode.FullScreen` like Gallery and keep
  controls centred/at the bottom. Never `Screen.safeAreaMargins` (doesn't exist on Qt 5.6) or
  model-specific pixel constants; the emulator has no cutout, so this needs a device check.
- **Cover:** summary (unread count, latest senders), one or two `CoverAction`s (e.g. new
  message). Plain `Label`s, `Theme.opacityHigh`. Model: `jolla-notes/cover/NotesCover.qml`.
- **Strings:** `qsTrId("threesea-…")` with `//%` engineering English (Jolla style, id-based).
- **Settings:** in-app settings page from the pulley (Harbour forbids `com.jolla.settings`
  plugins). A jolla-settings entry can be added later for the non-Harbour build.

## Harbour-allowed QML imports (docs.sailfishos.org → Harbour → Allowed APIs)

Allowed: `Sailfish.Silica`, `Sailfish.Pickers`, `Sailfish.Share`, `Sailfish.Secrets`,
`Sailfish.Crypto`, `Sailfish.Media`, `Sailfish.Contacts` (last ones: no compat guarantee),
`Amber.Mpris`, `Nemo.Notifications 1.0`, `Nemo.DBus 2.0`, `Nemo.Configuration`,
`Nemo.Thumbnailer`, `Nemo.KeepAlive 1.2`, `Nemo.Time`, `org.nemomobile.contacts`,
`QtMultimedia ≤ 5.6`, `QtQuick ≤ 2.6`.
**Not allowed:** `Sailfish.Gallery`, `Sailfish.Silica.private`, `com.jolla.*`,
`Nemo.FileManager` → anything we want from those must be rebuilt from Silica/QtQuick.

## Screen-by-screen patterns

Where CLAUDE.md Phase 4.5 is more specific (user decisions), it wins over this table.

Jolla's Messages, People, Email and Phone UIs are **closed source**, so for messaging layouts we
follow the community apps that deliberately mimic Jolla Messages (Whisperfish, Fernschreiber),
and the open Jolla apps (Notes, Gallery, Camera, Weather) for everything else.

| Screen | Pattern | Reference |
|---|---|---|
| Conversation list | `SilicaListView`, `PullDownMenu` (New chat, Settings, …), `ViewPlaceholder`, sections (Pinned/Today/Yesterday/Older); `ListItem` delegate = square picture, name with the unread count after it, three-line preview, clock time (details: CLAUDE.md Phase 4.5); `ContextMenu` (pin, mute for 8 hours or on again, delete with `remorseAction`); pinned rows show the day with the time, as their section names none | whisperfish `qml/pages/MainPage.qml`, `qml/delegates/SessionDelegate.qml` |
| Chat | Header: contact name, avatar to its right (Jolla Messages); contact details as attached page (swipe left). `SilicaListView` with `verticalLayoutDirection: ListView.BottomToTop`; outgoing right, incoming left; bubbles and their corners, time and delivery icons as in CLAUDE.md Phase 4.5 (white overlay 12 %/6 %, run-dependent corners) | whisperfish `qml/components/MessagesView.qml`, `qml/delegates/MessageDelegate.qml`, `qml/components/message/InfoItem.qml` |
| Input bar | `TextArea` + emoji button + send/record `IconButton`, the last item of the message list so it scrolls with the messages (Jolla Messages), not docked | whisperfish `qml/components/ChatTextInput.qml`; fernschreiber `qml/pages/ChatPage.qml` |
| Attachments | `Sailfish.Pickers` (`ImagePickerPage`, `ContentPickerPage`, `MultiContentPickerDialog`, …) pushed on the stack; theme icons `icon-m-image`, `icon-m-video`, `icon-m-mic`, `icon-m-document` | docs: sailfish-components-pickers |
| Image/video viewer | `FullscreenContentPage { allowedOrientations: Orientation.All }` with a `SlideshowView` of the chat's photos, GIFs and videos (always these three, not the gallery's filter; going round like Gallery's), back by swiping down, flick/pinch zoom that holds the slideshow still, tap for the overlay of actions and the position — re-created without `Sailfish.Gallery` | jolla-gallery `src/pages/GalleryFullscreenPage.qml`, sailfish-components-gallery `ImageViewer.qml`, `GalleryOverlay.qml` |
| Share out | `ShareAction { mimeType }` + `resources = [url]; trigger()`, kept once in `MessageOut.qml` for messages; the core makes a file's short-lived copy (ADR 0031); a link as `{type: "text/x-url", status: url}`, text as `{type: "text/plain", status}` (with `data` a share method may write it to a temporary file). Base.permission has the D-Bus access, no permission to declare | jolla-notes `pages/NotePage.qml`, `GalleryOverlay.qml` |
| Contact / group details | `PageHeader`, avatar, `DetailItem`s, `SectionHeader`s, pulley actions; contact picking via `Sailfish.Contacts` / `org.nemomobile.contacts` | whisperfish `ContactCardPage.qml`, `NewMessage.qml` |
| Cover | unread count + last sender (not the text; can be hidden), with the action to that chat next to new message while the name shows | jolla-notes `cover/NotesCover.qml` |
| Notifications | `Nemo.Notifications` with `appName`, `summary/body`, `previewSummary/previewBody`, `itemCount` grouping per chat, `remoteActions` with a `"default"` D-Bus action (+ `ExecDBus` in X-Sailjail) so tapping works when the app is closed; `replacesId` to update | commhistory-daemon `src/personalnotification.cpp`, whisperfish `qml/harbour-whisperfish-main.qml` |
| Settings | `Page` with `SectionHeader`s, `TextSwitch`, `ComboBox`, `Slider` | jolla-notes `settings/notes.qml` (look, not the plugin mechanism) |
| Voice messages | QtMultimedia `Audio`/`AudioRecorder`; playback controls via `Amber.Mpris` if useful. In the bubble: play/pause icon, Silica `Slider`, length, and the speed as a plain tappable label (1× / 1.5× / 2×, Threema's chip — text, not an own control) | docs: amber-mpris |
| Chat background | Android's wallpaper, off by default: the picked picture behind everything in the chat (`PreserveAspectCrop`) under a `Theme.highlightDimmerColor` layer, so the bubbles' own overlays keep their contrast in every Ambience. Chosen with `ImagePickerPage`, taken away from the Appearance page's pulley | — |

## Sources

- Silica reference and pitfalls: https://sailfishos.org/develop/docs/silica/ ,
  https://sailfishos.org/develop/docs/silica/sailfish-application-pitfalls.html
- UI docs and Definition of Done: https://docs.sailfishos.org/Develop/Apps/UI/
- Harbour allowed APIs/permissions: https://docs.sailfishos.org/Develop/Apps/Harbour/
- Notifications: https://sailfishos.org/develop/docs/nemo-qml-plugin-notifications/
- Pickers: https://sailfishos.org/develop/docs/sailfish-components-pickers/
- KeepAlive: https://sailfishos.org/develop/docs/nemo-keepalive/
- Secrets/Crypto: https://sailfishos.org/develop/docs/sailfish-secrets/
- Jolla open-source apps: https://github.com/sailfishos (jolla-notes, jolla-gallery,
  sailfish-components-gallery, jolla-camera, sailfish-weather, commhistory-daemon — all BSD/LGPL)
- Whisperfish (AGPL): https://gitlab.com/whisperfish/whisperfish ;
  Fernschreiber (GPL): https://github.com/Wunderfitz/harbour-fernschreiber
  — check each project's licence before reusing code, and credit what is reused.

- Mentions in bubbles: "@Name" in bold and `Theme.highlightColor` (user request, to catch the
  eye). Silica's link parser is private (not Harbour-allowed), so a hidden `LinkedLabel` finds
  the links and a `Text` with `StyledText` shows them with the mentions marked
  (`Format.markMentions`, names HTML-escaped).
