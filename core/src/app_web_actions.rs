// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web: what the browser does (send text, typing, delete, empty a chat, star, the
//! thumbs reactions). Each goes through the app's own command, like a tap in our UI, so the
//! same rules, sending and multi-device sync apply.

use rmpv::Value;
use serde_json::json;

use crate::{
    app::App,
    app_web_data::receiver_type,
    providers::CoreEvent,
    web::{
        arp::{self, Incoming, map},
        saltyrtc::field,
    },
};

/// An app remote protocol error code (`schema/v2.yaml`).
type Code = &'static str;

impl App {
    /// Run one of our commands as the UI would; its result, or the error kind.
    pub(crate) async fn web_command(
        &mut self,
        command: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let mut command = command;
        command["id"] = json!(0);
        let reply: serde_json::Value =
            serde_json::from_str(&self.handle(&command.to_string()).await).unwrap_or_default();
        if reply["ok"] == true {
            Ok(reply["result"].clone())
        } else {
            Err(reply["error"]["kind"]
                .as_str()
                .unwrap_or("internal")
                .to_owned())
        }
    }

    /// The chat (`args.id`, not private) and, for `args.messageId`, the message's hex ID and
    /// whether it is ours.
    fn web_target(
        &self,
        incoming: &Incoming<'_>,
    ) -> Result<(String, Option<(String, bool)>), Code> {
        let key = self
            .web_receiver_key(incoming.arg("id"))
            .map_err(|_| "invalidIdentity")?;
        let message = match incoming.arg("messageId") {
            None => None,
            Some(id) => {
                let message = id
                    .parse::<i64>()
                    .ok()
                    .and_then(|id| self.store.message(id).ok().flatten())
                    .filter(|message| message.conversation == key)
                    .ok_or("invalidMessage")?;
                Some((format!("{:016x}", message.message_id), message.outgoing))
            }
        };
        Ok((key, message))
    }

    /// Tell our UI (and through the event, the browsers) that a message changed.
    fn web_changed_message(&self, local_id: i64) {
        if let Ok(Some(message)) = self.store.message(local_id) {
            let _ = self
                .media_events
                .send(CoreEvent::MessagesChanged(vec![message]));
        }
    }

    /// Tell our UI the chat list changed (pinned, emptied, a message deleted).
    fn web_changed_chats(&self) {
        let _ = self
            .media_events
            .send(CoreEvent::Call(json!({ "event": "chatsChanged" })));
    }

    /// A chat opened in the browser loses its "Mark as unread". The browser hears of it with the
    /// next refresh, our UI through the event. A private or unknown chat stays as it is.
    fn web_chat_opened(&self, incoming: &Incoming<'_>) {
        let Ok(key) = self.web_receiver_key(incoming.arg("id")) else {
            return;
        };
        if self
            .store
            .marked_unread_chats()
            .is_ok_and(|marked| marked.contains(&key))
            && self.store.set_marked_unread(&key, false).is_ok()
        {
            self.web_changed_chats();
        }
    }

    /// The browser's actions; `None` if `incoming` isn't one. `Ok`: the reply (its `ack` added
    /// by the caller).
    pub(crate) async fn web_action(
        &mut self,
        incoming: &Incoming<'_>,
    ) -> Option<Result<Value, Code>> {
        let confirm = || arp::message("update", "confirm", None, None);
        Some(match (incoming.kind, incoming.sub_type) {
            ("create", "textMessage") => self.web_send_text(incoming).await,
            ("update", "typing") => {
                let typing = incoming
                    .data
                    .and_then(|d| field(d, "isTyping"))
                    .and_then(Value::as_bool);
                match (self.web_target(incoming), typing) {
                    (Ok((key, _)), Some(typing)) => self
                        .web_command(json!({"cmd": "setTyping", "identity": key, "typing": typing}))
                        .await
                        .map(|_| confirm())
                        .map_err(|_| "badRequest"),
                    _ => Err("badRequest"),
                }
            }
            // The chat is open in the browser: it counts as seen, like Android.
            ("update", "activeConversation") => {
                self.web_chat_opened(incoming);
                Ok(confirm())
            }
            // The browser hears of the removal from the next refresh, like every other.
            ("delete", "message") => match self.web_target(incoming) {
                Ok((key, Some((message_id, outgoing)))) => self
                    .web_command(json!({"cmd": "deleteMessage", "identity": key,
                        "messageId": message_id, "outgoing": outgoing, "everyone": false}))
                    .await
                    .map(|_| {
                        self.web_changed_chats();
                        confirm()
                    })
                    .map_err(|_| "badRequest"),
                Ok(_) => Err("badRequest"),
                Err(code) => Err(code),
            },
            ("delete", "cleanReceiverConversation") => match self.web_target(incoming) {
                Ok((key, _)) => self
                    .web_command(json!({"cmd": "emptyConversation", "identity": key}))
                    .await
                    .map(|_| {
                        self.web_changed_chats();
                        confirm()
                    })
                    .map_err(|_| "badRequest"),
                Err(code) => Err(code),
            },
            // Starring a chat is our pinning.
            ("update", "conversation") => {
                let starred = incoming
                    .data
                    .and_then(|d| field(d, "isStarred"))
                    .and_then(Value::as_bool);
                match (self.web_target(incoming), starred) {
                    (Ok((key, _)), Some(starred)) => self
                        .web_command(
                            json!({"cmd": "setPinned", "identity": key, "pinned": starred}),
                        )
                        .await
                        .map(|_| {
                            self.web_changed_chats();
                            confirm()
                        })
                        .map_err(|_| "badRequest"),
                    (Err(code), _) => Err(code),
                    _ => Ok(confirm()),
                }
            }
            // Agree / disagree: thumbs up / down reactions, like Android.
            ("request", "ack") => {
                // Threema Web puts it into the args (the spec: data); either.
                let agree = incoming
                    .args
                    .and_then(|a| field(a, "acknowledged"))
                    .or_else(|| incoming.data.and_then(|d| field(d, "acknowledged")))
                    .and_then(Value::as_bool);
                match (self.web_target(incoming), agree) {
                    (Ok((key, Some((message_id, _)))), Some(agree)) => self
                        .web_command(
                            json!({"cmd": "react", "identity": key, "messageId": message_id,
                            "emoji": if agree { "👍" } else { "👎" }, "apply": true}),
                        )
                        .await
                        .map(|_| {
                            if let Some(id) =
                                incoming.arg("messageId").and_then(|id| id.parse().ok())
                            {
                                self.web_changed_message(id);
                            }
                            confirm()
                        })
                        .map_err(|_| "badRequest"),
                    (Err(code), _) => Err(code),
                    _ => Err("badRequest"),
                }
            }
            _ => return None,
        })
    }

    /// `create/textMessage`: send (with a quote of one of the chat's messages); the reply names
    /// the new message.
    async fn web_send_text(&mut self, incoming: &Incoming<'_>) -> Result<Value, Code> {
        let (key, _) = self.web_target(incoming)?;
        let data = incoming.data.ok_or("badRequest")?;
        let text = field(data, "text")
            .and_then(Value::as_str)
            .ok_or("badRequest")?;
        let quote = field(data, "quote")
            .and_then(|quote| field(quote, "messageId"))
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<i64>().ok())
            .and_then(|id| self.store.message(id).ok().flatten())
            .filter(|message| message.conversation == key)
            .map(|message| format!("{:016x}", message.message_id));
        let result = self
            .web_command(json!({"cmd": "sendText", "identity": key, "text": text, "quote": quote}))
            .await
            .map_err(|kind| {
                if kind == "messageTooLong" {
                    "valueTooLong"
                } else {
                    "internalError"
                }
            })?;
        // Our UI shows it like one sent on another device.
        if let Some(Ok(Some(message))) = result["id"].as_i64().map(|id| self.store.message(id)) {
            let _ = self.media_events.send(CoreEvent::MessageAdded { message });
        }
        let id = result["id"]
            .as_i64()
            .map(|id| id.to_string())
            .unwrap_or_default();
        Ok(arp::message(
            "create",
            "textMessage",
            Some(map(vec![
                ("type", receiver_type(&key).into()),
                ("id", key.as_str().into()),
            ])),
            Some(map(vec![("messageId", id.into())])),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

    #[tokio::test(flavor = "current_thread")]
    async fn browser_sends_a_quoting_text() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5.1".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: "AAAAAAAA".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let quoted = app
            .store
            .insert_incoming("BBBBBBBB", 0xabc, "text", "question?", 1)
            .unwrap()
            .unwrap();

        let message = arp::message(
            "create",
            "textMessage",
            Some(map(vec![
                ("type", "contact".into()),
                ("id", "BBBBBBBB".into()),
            ])),
            Some(map(vec![
                ("text", "answer".into()),
                (
                    "quote",
                    map(vec![
                        ("identity", "BBBBBBBB".into()),
                        ("text", "question?".into()),
                        ("messageId", quoted.id.to_string().into()),
                    ]),
                ),
            ])),
        );
        let incoming = Incoming::parse(&message).unwrap();
        let reply = app.web_action(&incoming).await.unwrap().unwrap();
        let new_id: i64 = field(field(&reply, "data").unwrap(), "messageId")
            .and_then(Value::as_str)
            .unwrap()
            .parse()
            .unwrap();
        let sent = app.store.message(new_id).unwrap().unwrap();
        assert!(sent.outgoing);
        assert_eq!(sent.to_json()["text"], "answer");
        assert_eq!(sent.to_json()["quoteId"], format!("{:016x}", 0xabc));

        // A message of another chat can't be the target.
        let message = arp::message(
            "delete",
            "message",
            Some(map(vec![
                ("type", "contact".into()),
                ("id", "CCCCCCCC".into()),
                ("messageId", quoted.id.to_string().into()),
            ])),
            None,
        );
        let incoming = Incoming::parse(&message).unwrap();
        assert!(app.web_action(&incoming).await.unwrap().is_err());
    }
}
