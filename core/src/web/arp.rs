// SPDX-License-Identifier: AGPL-3.0-only
//! The connection layer of Threema's app remote protocol (ARP, `app-remote-protocol`
//! `schema/v2.yaml`) over the relayed-data task: msgpack wire messages
//! (`type`/`subType`/`args`/`data`, `id` asks for an acknowledgement, `ack` gives it), chunked
//! (`chunks`), `update/connectionInfo` first in both directions, chunk counting for
//! `connectionAck`.

use rmpv::Value;

use super::{chunks, saltyrtc::field};

/// A wire message.
#[must_use]
pub fn message(kind: &str, sub_type: &str, args: Option<Value>, data: Option<Value>) -> Value {
    let mut fields = vec![
        (Value::from("type"), Value::from(kind)),
        (Value::from("subType"), Value::from(sub_type)),
    ];
    if let Some(args) = args {
        fields.push(("args".into(), args));
    }
    if let Some(data) = data {
        fields.push(("data".into(), data));
    }
    Value::Map(fields)
}

/// `message` confirming the request `id` (`error`: its error code).
#[must_use]
pub fn with_ack(mut message: Value, id: &str, error: Option<&str>) -> Value {
    let mut ack = vec![
        (Value::from("id"), Value::from(id)),
        ("success".into(), Value::from(error.is_none())),
    ];
    if let Some(error) = error {
        ack.push(("error".into(), error.into()));
    }
    if let Value::Map(fields) = &mut message {
        fields.push(("ack".into(), Value::Map(ack)));
    }
    message
}

/// A map from string keys.
#[must_use]
pub fn map(fields: Vec<(&str, Value)>) -> Value {
    Value::Map(
        fields
            .into_iter()
            .map(|(key, value)| (key.into(), value))
            .collect(),
    )
}

/// The parts of a received wire message.
pub struct Incoming<'a> {
    pub kind: &'a str,
    pub sub_type: &'a str,
    pub args: Option<&'a Value>,
    pub data: Option<&'a Value>,
    /// The sender wants an acknowledgement for this ID.
    pub id: Option<&'a str>,
}

impl<'a> Incoming<'a> {
    #[must_use]
    pub fn parse(message: &'a Value) -> Option<Self> {
        Some(Self {
            kind: field(message, "type")?.as_str()?,
            sub_type: field(message, "subType")?.as_str()?,
            args: field(message, "args"),
            data: field(message, "data"),
            id: field(message, "id").and_then(Value::as_str),
        })
    }

    /// A string argument.
    #[must_use]
    pub fn arg(&self, name: &str) -> Option<&'a str> {
        field(self.args?, name)?.as_str()
    }
}

/// What the connection layer wants done.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Send this chunk as a 'data' payload.
    Chunk(Vec<u8>),
    /// A message for the app (everything after the handshake that isn't connection layer).
    Message(Value),
    /// The browser asked to stop (`delete`: and to forget the session).
    Disconnect { delete: bool },
}

/// The ARP connection state of one SaltyRTC connection.
pub struct Connection {
    /// Our outgoing message number (the chunks' message ID).
    message_id: u32,
    unchunker: chunks::Unchunker,
    /// Chunks received (what `connectionAck` reports).
    incoming_chunks: u32,
    /// The browser's `connectionInfo` arrived; until then, ours is the only message out.
    ready: bool,
    queued: Vec<Value>,
    /// A chunk arrived since our last `connectionAck`.
    unacknowledged: bool,
}

impl Connection {
    /// The connection is up (SaltyRTC handshake done): our `connectionInfo`, with the connection
    /// ID both sides derive. We don't resume yet, so no `resume`.
    pub fn start(connection_id: [u8; 16]) -> (Self, Vec<Action>) {
        let mut connection = Self {
            message_id: 0,
            unchunker: chunks::Unchunker::default(),
            incoming_chunks: 0,
            ready: false,
            queued: Vec::new(),
            unacknowledged: false,
        };
        let info = message(
            "update",
            "connectionInfo",
            None,
            Some(map(vec![("id", Value::Binary(connection_id.to_vec()))])),
        );
        let actions = connection.encode(&info);
        (connection, actions)
    }

    /// Send `message` (held back until the browser's `connectionInfo` arrived).
    pub fn send(&mut self, message: Value) -> Vec<Action> {
        if !self.ready {
            self.queued.push(message);
            return Vec::new();
        }
        self.encode(&message)
    }

    fn encode(&mut self, message: &Value) -> Vec<Action> {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, message).expect("writing to a Vec does not fail");
        let id = self.message_id;
        self.message_id = self.message_id.wrapping_add(1);
        chunks::chunk(id, &bytes)
            .into_iter()
            .map(Action::Chunk)
            .collect()
    }

    /// A 'data' payload from the browser (one chunk).
    pub fn receive(&mut self, payload: &Value) -> Vec<Action> {
        let Some(chunk) = payload.as_slice() else {
            tracing::warn!("Threema Web: non-binary payload dropped");
            return Vec::new();
        };
        self.incoming_chunks = self.incoming_chunks.wrapping_add(1);
        self.unacknowledged = true;
        let Some(bytes) = self.unchunker.add(chunk) else {
            return Vec::new();
        };
        // Within the budget before it is decoded (deep audit D03).
        if !super::budget::within(&bytes, super::budget::MAX_VALUES, super::budget::MAX_DEPTH) {
            tracing::warn!("Threema Web: oversized message dropped");
            return Vec::new();
        }
        let Ok(message) = rmpv::decode::read_value(&mut bytes.as_slice()) else {
            tracing::warn!("Threema Web: undecodable message dropped");
            return Vec::new();
        };
        let Some(incoming) = Incoming::parse(&message) else {
            tracing::warn!("Threema Web: message without type dropped");
            return Vec::new();
        };
        match (incoming.kind, incoming.sub_type) {
            ("update", "connectionInfo") => {
                // Resumption comes later: a new connection is a fresh start for both sides.
                self.ready = true;
                let queued = std::mem::take(&mut self.queued);
                queued
                    .into_iter()
                    .flat_map(|message| self.encode(&message))
                    .collect()
            }
            ("request", "connectionAck") => self.acknowledge(),
            // We keep no chunk cache yet, so there is nothing to prune.
            ("update", "connectionAck") => Vec::new(),
            ("update", "connectionDisconnect") => {
                let reason = incoming
                    .data
                    .and_then(|data| field(data, "reason"))
                    .and_then(Value::as_str);
                vec![Action::Disconnect {
                    delete: reason == Some("delete"),
                }]
            }
            _ if !self.ready => {
                tracing::warn!("Threema Web: message before connectionInfo dropped");
                Vec::new()
            }
            _ => vec![Action::Message(message.clone())],
        }
    }

    /// `update/connectionAck` with the chunks received so far (asked for, or 10 s after a chunk).
    pub fn acknowledge(&mut self) -> Vec<Action> {
        self.unacknowledged = false;
        let ack = message(
            "update",
            "connectionAck",
            None,
            Some(map(vec![("sequenceNumber", self.incoming_chunks.into())])),
        );
        self.send(ack)
    }

    /// A chunk arrived since the last `connectionAck`.
    #[must_use]
    pub fn needs_acknowledgement(&self) -> bool {
        self.unacknowledged
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(actions: &[Action]) -> Vec<Value> {
        let mut unchunker = chunks::Unchunker::default();
        actions
            .iter()
            .filter_map(|action| match action {
                Action::Chunk(chunk) => unchunker.add(chunk),
                _ => None,
            })
            .map(|bytes| rmpv::decode::read_value(&mut bytes.as_slice()).unwrap())
            .collect()
    }

    fn chunk_of(message: &Value, id: u32) -> Value {
        let mut bytes = Vec::new();
        rmpv::encode::write_value(&mut bytes, message).unwrap();
        Value::Binary(chunks::chunk(id, &bytes).remove(0))
    }

    #[test]
    fn connection_info_first_then_messages() {
        let (mut connection, actions) = Connection::start([9; 16]);
        let sent = decode(&actions);
        let info = Incoming::parse(&sent[0]).unwrap();
        assert_eq!((info.kind, info.sub_type), ("update", "connectionInfo"));
        assert_eq!(
            field(info.data.unwrap(), "id"),
            Some(&Value::Binary(vec![9; 16]))
        );
        // Held back until the browser's connectionInfo.
        assert!(
            connection
                .send(message("response", "profile", None, None))
                .is_empty()
        );
        let request = message("request", "clientInfo", None, None);
        assert!(connection.receive(&chunk_of(&request, 0)).is_empty());
        let browser_info = message(
            "update",
            "connectionInfo",
            None,
            Some(map(vec![("id", Value::Binary(vec![9; 16]))])),
        );
        let sent = decode(&connection.receive(&chunk_of(&browser_info, 1)));
        assert_eq!(Incoming::parse(&sent[0]).unwrap().sub_type, "profile");
        // Now requests reach the app.
        let actions = connection.receive(&chunk_of(&request, 2));
        assert!(matches!(&actions[..], [Action::Message(_)]));
        // Three chunks received.
        let ack = decode(&connection.acknowledge());
        assert_eq!(
            field(field(&ack[0], "data").unwrap(), "sequenceNumber"),
            Some(&Value::from(3))
        );
        let disconnect = message(
            "update",
            "connectionDisconnect",
            None,
            Some(map(vec![("reason", "delete".into())])),
        );
        assert_eq!(
            connection.receive(&chunk_of(&disconnect, 3)),
            vec![Action::Disconnect { delete: true }]
        );
    }

    #[test]
    fn acks_carry_the_request_id() {
        let reply = with_ack(message("response", "clientInfo", None, None), "abc", None);
        let ack = field(&reply, "ack").unwrap();
        assert_eq!(field(ack, "id"), Some(&Value::from("abc")));
        assert_eq!(field(ack, "success"), Some(&Value::from(true)));
    }
}
