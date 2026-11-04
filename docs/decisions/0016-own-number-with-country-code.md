# 0016 — The user's own number needs its country code

Status: accepted (2026-11-04).

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

## Consequences

- Stricter than Android: someone typing a national number is told to add the country code.
- Own number and address book now differ. Address book entries without `+` are still completed from
  the locale, so people can be missed without an error. Which country to use there isn't decided yet:
  the calling code of the user's linked number, a setting, the SIM's country (may need a permission
  the Jolla Store doesn't allow) or a libphonenumber crate (a new dependency).
- Known gaps in the national rules, also still open: Italian landlines keep their `0` internationally
  and US/Canadian numbers written with their leading `1` get a second one.
