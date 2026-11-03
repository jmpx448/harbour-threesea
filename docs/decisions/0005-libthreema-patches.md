# 0005 — Small patch series on top of pristine libthreema

**Superseded by 0006** (vendored copy; the patches are now commits). Kept for history.

**Context.** Shop/Libre licenses must be sent (`licenseKey`, `deviceId`) when creating an
identity (Android: `UserServiceImpl.createIdentityRequestDataJSON`). libthreema's
`CreateIdentityTask` cannot do that, and the pieces needed to do it ourselves (directory
challenge key derivation, request builders) are `pub(crate)`. Re-implementing them would
duplicate key handling outside libthreema (§7 rule 1).

**Decision.** Keep the submodule at the pristine upstream tag and carry minimal
patches in `patches/libthreema/NNNN-*.patch` (paths relative to `domain/libthreema`), applied
idempotently by `scripts/apply-patches.sh` (also run by `scripts/build-core.sh`). Every patch
comes with tests and keeps all upstream targets (cli/uniffi/wasm) compiling. `.gitmodules` sets
`ignore = dirty` so the applied patches don't show up as submodule changes.

Patches (applied as a series; the script resets `domain/libthreema` first):
- `0001-identity-create-serial-license`: `CreateIdentityContext::serial_license:
  Option<SerialLicense { license_key, device_id }>`, flattened into the `identity/create` body.
- `0002-public-directory-challenge-solver`: `ClientKey::solve_directory_challenge()` so `core`
  can talk to directory endpoints libthreema has no task for yet (`identity/fetch_priv`, link
  e-mail/phone, revocation key …) without touching the client key outside libthreema.
- `0003-public-outgoing-contact-message-encoder`: `encode_outgoing_contact_message()` wraps
  libthreema's existing (unused, TODO LIB-51) `encode_and_encrypt_message` into a CSP
  `MessageWithMetadataBox`. Stop-gap until libthreema's outgoing message task exists; caller
  picks/records the nonce, sends, awaits the ack.

**Consequences.** On every upstream bump, re-run the script; fix conflicts by regenerating the
patch (`git -C third_party/threema-android diff --relative=domain/libthreema`). Do **not** open pull requests upstream (user decision 2026-10-21: Threema's plans for
libthreema are unknown, and so is their stance on AI-written commits). Drop a patch once upstream
ships an equivalent.
