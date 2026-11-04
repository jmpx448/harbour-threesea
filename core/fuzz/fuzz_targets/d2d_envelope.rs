// SPDX-License-Identifier: AGPL-3.0-only
//! A reflected message from one of the user's other devices: the decoded envelope applied to a
//! real store, without the decryption in front that a fuzzer can't get past.
#![no_main]
use std::{cell::RefCell, rc::Rc};

use libthreema::protobuf::d2d;
use prost::Message as _;
use threesea_core::{reflected, store::Store};

thread_local! {
    // A fresh store now and then: rows pile up otherwise and slow everything down.
    static STORE: RefCell<(Rc<Store>, u32)> = RefCell::new((open(), 0));
}

fn open() -> Rc<Store> {
    Rc::new(Store::open(std::path::Path::new(":memory:"), &[1; 32]).expect("in-memory store"))
}

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(envelope) = d2d::Envelope::decode(data) else {
        return;
    };
    let _ = reflected::content_name(envelope.content.as_ref());
    STORE.with_borrow_mut(|(store, runs)| {
        *runs += 1;
        if *runs > 1000 {
            *store = open();
            *runs = 0;
        }
        let events = Rc::new(RefCell::new(Vec::new()));
        let user = "ABCD1234".parse().expect("a valid identity");
        let _ = reflected::apply(store, user, envelope, &events);
    });
});
