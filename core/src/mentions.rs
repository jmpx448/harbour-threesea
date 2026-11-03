// SPDX-License-Identifier: AGPL-3.0-only
//! Mentions in group texts (and captions): `@[<ID>]`, or `@[@@@@@@@@]` for everyone (Android
//! `ConversationTextAnalyzer`, `ContactService.ALL_USERS_PLACEHOLDER_ID`). The text keeps them
//! raw; the UI gets the mentioned contacts' names next to it and shows "@Name" (our own ID and
//! "everyone" are translated in the UI).

use serde_json::{Map, Value};

use crate::{contacts, messages::Message, store::Store};

pub const EVERYONE: &str = "@@@@@@@@";

/// The identities mentioned in `text`, in order, without duplicates.
pub fn identities(text: &str) -> Vec<&str> {
    let mut found: Vec<&str> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("@[") {
        let candidate = &rest[start + 2..];
        match candidate.get(..9) {
            Some(tag)
                if tag.ends_with(']')
                    && tag[..8].bytes().all(|b| {
                        b.is_ascii_digit() || b.is_ascii_uppercase() || b == b'*' || b == b'@'
                    }) =>
            {
                if !found.contains(&&tag[..8]) {
                    found.push(&tag[..8]);
                }
                rest = &candidate[9..];
            }
            _ => rest = candidate,
        }
    }
    found
}

impl Store {
    /// A message's JSON for the UI, with `mentions`: `{ID: display name}` of the mentioned
    /// contacts, our own ID with `""` (the UI says "Me"); `@@@@@@@@` and unknown IDs are left
    /// out (the UI shows "All" / the ID). `mentionsMe`: we or everyone are mentioned.
    pub fn message_json(&self, message: &Message) -> Value {
        let mut json = message.to_json();
        let text = json["text"]
            .as_str()
            .or_else(|| json["body"]["caption"].as_str())
            .unwrap_or_default()
            .to_owned();
        let mentioned = identities(&text);
        if mentioned.is_empty() {
            return json;
        }
        let own = self.identity().ok().flatten().map(|own| own.threema_id);
        let mut mentions = Map::new();
        let mut mentions_me = false;
        for identity in mentioned {
            if identity == EVERYONE {
                mentions_me = true;
            } else if own.as_deref() == Some(identity) {
                mentions_me = true;
                mentions.insert(identity.to_owned(), "".into());
            } else if let Ok(Some(contact)) = self.contact(identity) {
                mentions.insert(identity.to_owned(), contacts::display_name(&contact).into());
            }
        }
        json["mentions"] = Value::Object(mentions);
        json["mentionsMe"] = mentions_me.into();
        json
    }

    pub fn messages_json(&self, messages: &[Message]) -> Vec<Value> {
        messages.iter().map(|m| self.message_json(m)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mentions() {
        assert_eq!(
            identities("Hi @[ABCD1234] and @[@@@@@@@@], @[ABCD1234] again"),
            vec!["ABCD1234", EVERYONE]
        );
        assert_eq!(identities("@[*THREEMA]"), vec!["*THREEMA"]);
        assert!(identities("@[short] @[lowercas] @[ABCD1234 x @[").is_empty());
        assert_eq!(identities("@[@[ABCD1234]"), vec!["ABCD1234"]);
        assert_eq!(identities("ü@[ABCD1234]ü"), vec!["ABCD1234"]);
    }
}
