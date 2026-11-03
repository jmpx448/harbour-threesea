# 0008 — The UI renders text from others as plain text and never goes online

## Context
Qt's `Text` defaults to `AutoText`: a string that looks like markup is rendered as rich text.
Names, nicknames, messages, captions, group titles and poll choices come from other people, so
`<b>…</b>` changed how they looked, and `<img src="https://…">` would make the QML engine fetch
the image: a tracking pixel that reveals the reader's IP address to a third party
(CLAUDE.md §7.6).

## Decision
- `ApplicationWindow._defaultLabelFormat: Text.PlainText` (Silica's switch, used by Jolla's
  apps): every Silica `Label`, including those inside `PageHeader`, is plain text.
- Message text uses Silica's `LinkedLabel` (escapes the text, then marks up links, e-mail
  addresses and phone numbers itself).
- The QML engine gets a network access manager that only allows `file:`, `qrc:` and `data:`
  (`bridge/offlinenetwork.h`). All network traffic is the core's.

## Consequences
- A label that really wants markup must set `textFormat` explicitly.
- Remote images can never be shown by QML; media always comes through the core and the
  `image://threesea` provider.
- Notifications are rendered by lipstick, outside our engine; we pass plain strings.
