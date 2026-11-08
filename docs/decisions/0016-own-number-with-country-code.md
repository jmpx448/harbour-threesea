# 0016 — The user's own number needs its country code

Status: accepted (2026-11-04); the address book's country added the same day.

## Context

Android completes a national number with the SIM's country, then the locale's, and formats it with
libphonenumber (`LocaleServiceImpl`). We have neither: `normalize_phone` knows a short table of calling codes
and a simplified national prefix, and its country comes from the UI locale alone. A phone set to
German with an Austrian number turned `0664 1234567` into `+49 664 1234567`. For the link request that
means a real SMS sent to a number the user didn't enter.

## Decision

`linkMobile` takes only numbers that start with `+` or `00` (`international_phone`). The link dialog
asks the core what it makes of the text (`normalizeMobile`, no network), shows "The code goes to +…",
can't be accepted without one, and sends exactly the number it showed.

Address book entries without a country code (user decision 2026-11-04) take their country from a
setting, Settings → Privacy, "Numbers without a country code": a country from the list, or Automatic,
the default. Automatic is the calling code of the user's linked number, else the locale's country.
Not the SIM's country (it may need a permission the Jolla Store doesn't allow), and no libphonenumber
crate (a new dependency for what a short table does). The choice stays on this device: it describes
this address book, and the linked number that decides by default goes with the ID anyway.

## Consequences

- Stricter than Android: someone typing a national number is told to add the country code.
- Own number and address book differ: the address book can't ask, and its entries are often national.
- The national rules live in one table (`COUNTRIES` in `address_book.rs`): calling code and national
  prefix per country. Italy has none, so its landlines keep their `0`; North America's is `1`, so
  `1 415 …` and `415 …` come out the same; Hungary's is `06`; Russia's `8` goes only from all 11 digits,
  since real numbers start with it too; Liechtenstein has its own code, `423`. The table also has the
  prefixes for calling abroad besides `00` where they differ (`011` in North America, `010` in Japan,
  `810` in Russia, `0011` and its kin in Australia, `0[0-3]x` in Singapore). The own number takes only
  `+` and `00`. A country not listed isn't guessed at and can't be chosen: its numbers need their `+`
  to be found, and a linked number from there leaves Automatic to the locale. With no country at all,
  a number that doesn't say its own isn't hashed, as it could equal someone's number elsewhere.
- Known gaps: carrier codes in prefixes (Brazil's `015 11 9…`, Korea's and Israel's prefixes for calling
  abroad) aren't understood, so those numbers aren't found unless written with `+`.
- Numbers that say their country are cleaned up too: `(+49) 171 …` counts as `+49 171 …`, a national
  prefix after the calling code goes (`+49 0171 …`, `+49 (0171) …`, `+36 06 …`), and marks that only
  set the text's direction (U+202A, U+202C and their kind) are ignored, as are extensions and tones
  (`;12`, `ext. 12`); digits of other scripts count. The prefix goes only if it is a `0` (or `06`) and
  the country is known to have it. `(0)` is dropped except for Italy, where it is part of the number
  (`+39 (0)6 …`). This also holds for the own number: the dialog shows the result before anything is
  sent.
