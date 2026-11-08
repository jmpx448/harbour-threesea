// SPDX-License-Identifier: AGPL-3.0-only
//! A group call participant's handshake messages. The hello is sealed with the group's key, so
//! the input goes in both ways: as it is, and sealed to get to the protobuf behind.
#![no_main]
use crypto_secretbox::{KeyInit as _, XSalsa20Poly1305, aead::Aead as _};
use libthreema::common::keys::ClientKey;
use threesea_core::group_call_handshake::{Known, Local, MediaKey, Peer};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let client_key = ClientKey::from([7; 32]);
    let media_keys = [MediaKey::random(0)];
    let gchk = [9; 32];
    let local = Local {
        identity: "ABCD1234",
        nickname: "Test",
        client_key: &client_key,
        gckh: &[8; 32],
        gchk: &gchk,
        media_keys: &media_keys,
    };
    let lookup = |_: &str| {
        Some(Known {
            public_key: ClientKey::from([6; 32]).public_key(),
            member: true,
        })
    };
    let nonce = [1; 24];
    let Ok(sealed) = XSalsa20Poly1305::new((&gchk).into()).encrypt((&nonce).into(), data) else {
        return;
    };
    let hello = [nonce.as_slice(), &sealed].concat();
    let mut newcomer = Peer::newcomer(1);
    let _ = newcomer.receive(&hello, &local, &lookup);
    let _ = newcomer.receive(data, &local, &lookup);
    let (mut existing, _) = Peer::joined(2, &local);
    let _ = existing.receive(data, &local, &lookup);
});
