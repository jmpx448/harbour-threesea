// SPDX-License-Identifier: AGPL-3.0-only
//! A call signal from a contact: its JSON, then through the call's state machine.
#![no_main]
use threesea_core::{
    calls::{Calls, Signal},
    providers::CallSignalKind,
};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&kind, json)) = data.split_first() else {
        return;
    };
    let kind = match kind % 5 {
        0 => CallSignalKind::Offer,
        1 => CallSignalKind::Answer,
        2 => CallSignalKind::IceCandidates,
        3 => CallSignalKind::Hangup,
        _ => CallSignalKind::Ringing,
    };
    let Ok(signal) = Signal::decode(kind, json) else {
        return;
    };
    let _ = signal.encode();
    let mut calls = Calls::default();
    // In the order a call goes: an offer leaves a call for the rest to act on.
    let _ = calls.on_signal("ABCD1234", signal, 0);
    if let Ok(again) = Signal::decode(kind, json) {
        let _ = calls.on_signal("ABCD1234", again, 1);
    }
});
