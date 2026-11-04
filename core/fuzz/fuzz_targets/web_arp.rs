// SPDX-License-Identifier: AGPL-3.0-only
//! A Threema Web message from the browser: the connection layer's handling of whole messages
//! (one chunk each, so the fuzzer gets past reassembly) and the parts of a wire message.
#![no_main]
use rmpv::Value;
use threesea_core::web::arp::{Connection, Incoming};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let (mut connection, _) = Connection::start([0; 16]);
    // Chunk header: end flag, message id, chunk number.
    let chunk = [&[1, 0, 0, 0, 0, 0, 0, 0, 0], data].concat();
    let _ = connection.receive(&Value::Binary(chunk));
    let _ = connection.receive(&Value::Binary(data.to_vec()));
    let _ = connection.acknowledge();
    if let Ok(message) = rmpv::decode::read_value(&mut &*data)
        && let Some(incoming) = Incoming::parse(&message)
    {
        let _ = incoming.arg("id");
    }
});
