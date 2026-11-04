# 0024 — Failed transfers try again by themselves, a limited number of times

Status: accepted (2026-11-05).

## Context

Android has a connect and a read timeout on blobs and no limit on the whole transfer. It doesn't
try a failed download again by itself (`IncomingContactFileMessageTask`: the user can do that). We
had one 100-second limit on the whole transfer, and sent failed uploads again at every reconnect,
with no end. A phone that sleeps and moves (CLAUDE.md §8) breaks off many transfers, and tapping
each message in a chat is no answer.

## Decision

- A transfer ends after 30 s without a connection or 100 s without anything moving, and has no limit
  as a whole. Once the connection took all of an upload's body the answer has 300 s.
- A failed upload or download tries again by itself, while connected, after 1 min, 5 min, 30 min,
  2 h and 8 h, then waits for a tap. A tap always tries and starts them over. The count is in the
  message's body, so it survives a restart and goes with the message. An upload cut short by the
  end of the process counts as a failed try.
- Tries are only for what may work later: network errors, 408, 429 and 5xx. Other answers of the
  server, decryption errors, files too large and a full disk wait for a tap.
- A failed file part comes again only as it would by itself on the current network (Settings →
  Media). What failed before tries were counted, and a restored backup's media, wait for a tap.
- A file part uploaded stays recorded if the thumbnail fails, for a day, while it would go the same
  way (server, scope, `persist`).

**Deviation from Android:** downloads trying again by themselves.

## Consequences

The worst an upload that keeps breaking off near its end can cost is six times its size, between
two taps. A body sent in pieces can't be sent twice, so an upload isn't sent again on a 307 or 308
(reqwest hands that back, and it waits for a tap as any other answer does), and an HTTP/2 refused
stream, which reqwest would send again by itself, costs a try and a pause. Very slow links can still
fail: below about 80 kbit/s the system's send buffer outlasts the limits (`docs/protocol-notes.md`).
