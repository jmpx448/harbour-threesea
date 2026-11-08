# Security

Threesea handles private keys, messages and a paid license, so security reports are very
welcome. Please read the [disclaimer](README.md#disclaimer--please-read) first: this is an
experimental, AI-assisted project without warranty.

## Reporting a vulnerability

Report it **privately** through GitHub: the repository's **Security** tab → **Report a
vulnerability**. Please don't open a public issue for it.

Helpful: the version and flavour (Settings → About), the device and Sailfish OS version, what an
attacker needs (a contact, a paired browser, access to the phone…) and what they gain, and how
to reproduce it. **Never include** your Threema ID's backup, passwords, license keys, private
keys or other people's messages or IDs.

This is a spare-time project: replies may take a few days. Fixes go into a new release (the
[CHANGELOG](CHANGELOG.md) names them once they are out).

## Scope

- In scope: this repository (the Rust core, the Qt bridge, the QML app, the build and release
  scripts), including our changes to the vendored `third_party/libthreema`.
- Out of scope here: Threema's own apps, servers and protocols, and the unmodified upstream
  code. Report those to [Threema](https://threema.ch/en/contact) instead. Threesea is not made by
  or affiliated with Threema AG, so please don't send Threesea issues to them.

## Reviews so far

These are AI-run reviews, not a professional audit or certification.

The audits of 2026-10-27 say at their top what has been fixed since.

- [docs/security-audit.md](docs/security-audit.md)
- [docs/security-audit-2026-10-27.md](docs/security-audit-2026-10-27.md)
- [docs/rules-audit-2026-10-27.md](docs/rules-audit-2026-10-27.md): the code against the project's
  own rules
- [docs/deep-audit-2026-10-27.md](docs/deep-audit-2026-10-27.md): its header lists what is still
  open, such as the device check of the notification actions
- [docs/audit-2026-10-30.md](docs/audit-2026-10-30.md)
- A review of 2026-11-04, not published as a document: its fixes are in the next release, listed in
  the [CHANGELOG](CHANGELOG.md), and what still needs a phone is in
  [docs/voice-video-device-checklist.md](docs/voice-video-device-checklist.md).

## What a data backup's password protects

Android's data backup format, which Threesea reads and writes, authenticates each entry's content
but not the entry names or which entries there are. Someone who can write to a backup file without
knowing its password can't read or change the contents, but can swap or remove whole entries: put
one chat's messages under another contact's name, or take out the list of used nonces. Threesea
refuses duplicate and overlapping entries, says on the result page when that list is missing and
points out some crude copies and swaps, but it can't notice every such change. Keep backups where
others can't write to them. Details: [docs/android-backup-design.md](docs/android-backup-design.md).

## Verifying a release

Each release lists `SHA256SUMS`, and every RPM has a build provenance attestation:
`gh attestation verify <file>.rpm --repo penttimus/harbour-threesea` shows that it was built by
this repository's release workflow from the tagged source.
