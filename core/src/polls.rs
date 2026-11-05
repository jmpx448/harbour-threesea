// SPDX-License-Identifier: AGPL-3.0-only
//! Polls (csp.struct.yml `poll-setup`, `poll-vote`): a poll is a message of kind `poll` whose
//! `body` is its state for the UI:
//!
//! `{pollId (hex), creator, description, state (0 open, 1 closed), multiple, announce (votes go
//! to everyone), summary (only totals), choices: [{id, name, total?}], votes: {identity: [choice
//! ids]}}`
//!
//! Votes are kept per voter (our own under our identity); a closed poll's results replace them.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};

use crate::{
    messages::{Message, now_ms},
    store::{Store, StoreError},
};

/// Schema (store migration 10): which message holds a poll.
pub(crate) const MIGRATION: &str = "
    CREATE TABLE polls (
        conversation  TEXT NOT NULL,
        creator       TEXT NOT NULL,
        poll_id       INTEGER NOT NULL,
        message       INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
        PRIMARY KEY (conversation, creator, poll_id)
    ) WITHOUT ROWID;";

/// Outbox kinds: our vote (`{creator, pollId, choices: [[id, 0|1]]}`) and closing our poll
/// (`{pollId, poll}`: the closing `poll-setup` JSON; only for entries queued before closing became
/// a `pollClosed` message).
pub const OUTBOX_POLL_VOTE: &str = "pollVote";
pub const OUTBOX_POLL_CLOSE: &str = "pollClose";
/// The message kind of a poll's closing (Android's `POLL_CLOSED`): its body is the closed poll
/// with the results. Ours is sent as a poll setup like the poll itself.
pub const KIND_CLOSED: &str = "pollClosed";

/// Protocol limits.
const TEXT_LENGTH_MAX: usize = 256;
const JSON_LENGTH_MAX: usize = 6000;

#[derive(Debug, thiserror::Error)]
pub enum PollError {
    #[error("text too long")]
    TooLong,
    #[error("invalid poll: {0}")]
    Invalid(&'static str),
}

/// The poll state of a stored message body.
fn state(message: &Message) -> Value {
    serde_json::from_str(&message.body).unwrap_or(Value::Null)
}

/// A new poll's UI state; checks the protocol's limits.
pub fn new_poll(
    poll_id: u64,
    creator: &str,
    description: &str,
    choices: &[String],
    multiple: bool,
    announce: bool,
) -> Result<Value, PollError> {
    let description = description.trim();
    let choices: Vec<&str> = choices
        .iter()
        .map(|c| c.trim())
        .filter(|c| !c.is_empty())
        .collect();
    if description.is_empty() {
        return Err(PollError::Invalid("no question"));
    }
    if choices.len() < 2 {
        return Err(PollError::Invalid("fewer than two choices"));
    }
    if description.len() > TEXT_LENGTH_MAX || choices.iter().any(|c| c.len() > TEXT_LENGTH_MAX) {
        return Err(PollError::TooLong);
    }
    let poll = json!({
        "pollId": format!("{poll_id:016x}"),
        "creator": creator,
        "description": description,
        "state": 0,
        "multiple": multiple,
        "announce": announce,
        "summary": false,
        "choices": choices.iter().enumerate().map(|(i, c)| json!({"id": i, "name": c})).collect::<Vec<_>>(),
        "votes": {},
    });
    if setup_json(&poll).len() > JSON_LENGTH_MAX {
        return Err(PollError::TooLong);
    }
    Ok(poll)
}

/// The `poll-setup` JSON for a poll state (open, or closed with the results).
pub fn setup_json(poll: &Value) -> String {
    let closed = poll["state"] == 1;
    let choices = poll["choices"].as_array().cloned().unwrap_or_default();
    let votes = poll["votes"].as_object().cloned().unwrap_or_default();
    let participants: Vec<&String> = votes.keys().collect();
    let mut json = json!({
        "d": poll["description"],
        "s": i32::from(closed),
        "a": i32::from(poll["multiple"] == true),
        "t": i32::from(poll["announce"] == true),
        "u": 0,
        "o": 0,
        "c": choices.iter().enumerate().map(|(index, choice)| {
            let mut c = json!({"i": choice["id"], "n": choice["name"], "o": index});
            if closed {
                c["r"] = json!(participants.iter().map(|voter| {
                    let chosen = votes[voter.as_str()].as_array().is_some_and(|ids| ids.contains(&choice["id"]));
                    i32::from(chosen)
                }).collect::<Vec<_>>());
            }
            c
        }).collect::<Vec<_>>(),
    });
    if closed {
        json["p"] = json!(participants);
    }
    json.to_string()
}

/// Whether closing this poll, results included, fits a message (poll ID and, in a group, its
/// identity in front). There is no shorter form: a poll can't turn to summary mode when it closes,
/// other clients discard that.
pub fn closing_fits(poll: &Value) -> bool {
    const IN_FRONT: usize = 8 + 16;
    let mut closed = poll.clone();
    closed["state"] = json!(1);
    setup_json(&closed).len() + IN_FRONT <= crate::messages::MESSAGE_BODY_MAX
}

/// Our UI state from a received `poll-setup` JSON.
fn from_setup(poll_id: u64, creator: &str, setup: &Value) -> Option<Value> {
    let choices = setup["c"].as_array()?;
    let summary = setup["u"] == 1;
    let mut votes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    if setup["s"] == 1 && !summary {
        let participants: Vec<&str> = setup["p"]
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .collect();
        for choice in choices {
            for (index, vote) in choice["r"].as_array().into_iter().flatten().enumerate() {
                if vote.as_i64().unwrap_or(0) != 0
                    && let Some(voter) = participants.get(index)
                {
                    votes
                        .entry((*voter).to_owned())
                        .or_default()
                        .push(choice["i"].clone());
                }
            }
        }
    }
    Some(json!({
        "pollId": format!("{poll_id:016x}"),
        "creator": creator,
        "description": setup["d"].as_str().unwrap_or_default(),
        "state": i32::from(setup["s"] == 1),
        "multiple": setup["a"] == 1,
        "announce": setup["t"] == 1,
        "summary": summary,
        "choices": choices.iter().map(|c| {
            let mut choice = json!({"id": c["i"], "name": c["n"].as_str().unwrap_or_default()});
            if summary && setup["s"] == 1 {
                choice["total"] = c["t"].clone();
            }
            choice
        }).collect::<Vec<_>>(),
        "votes": votes,
    }))
}

/// The selected choice ids of a `poll-vote` JSON (`[[id, 0|1], …]`).
fn selected(choices: &Value) -> Vec<Value> {
    choices
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pair| pair[1].as_i64().unwrap_or(0) != 0)
        .map(|pair| pair[0].clone())
        .collect()
}

/// A vote as `poll-vote` JSON: every choice with 0 or 1.
pub fn vote_json(poll: &Value, chosen: &[i64]) -> String {
    json!(
        poll["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                let id = c["id"].as_i64().unwrap_or(-1);
                json!([id, i32::from(chosen.contains(&id))])
            })
            .collect::<Vec<_>>()
    )
    .to_string()
}

impl Store {
    pub fn set_message_body(&self, id: i64, body: &Value) -> Result<Option<Message>, StoreError> {
        self.conn.execute(
            "UPDATE messages SET body = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, body.to_string(), now_ms()],
        )?;
        self.message(id)
    }

    pub fn poll_message(
        &self,
        conversation: &str,
        creator: &str,
        poll_id: u64,
    ) -> Result<Option<Message>, StoreError> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT message FROM polls WHERE conversation = ?1 AND creator = ?2 AND poll_id = ?3",
                params![conversation, creator, poll_id.cast_signed()],
                |row| row.get(0),
            )
            .optional()?;
        // Older versions kept the entry of a poll someone deleted for everyone: a placeholder
        // that takes no votes and can't be closed.
        Ok(match id {
            Some(id) => self.message(id)?.filter(|message| message.kind == "poll"),
            None => None,
        })
    }

    pub fn register_poll(
        &self,
        conversation: &str,
        creator: &str,
        poll_id: u64,
        message: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO polls (conversation, creator, poll_id, message) VALUES (?1, ?2, ?3, ?4)",
            params![conversation, creator, poll_id.cast_signed(), message],
        )?;
        Ok(())
    }
}

/// _Common Poll Setup Receive Steps_: a new poll (returns it as added) or the closing of a known
/// open one (returns the poll as changed and the results message as added).
#[expect(clippy::too_many_arguments, reason = "the receive context")]
pub fn receive_setup(
    store: &Store,
    conversation: &str,
    sender: &str,
    group_sender: bool,
    message_id: u64,
    created_at: i64,
    poll_id: u64,
    setup: &str,
) -> Result<(Option<Message>, Option<Message>), StoreError> {
    let Ok(setup) = serde_json::from_str::<Value>(setup) else {
        tracing::warn!("invalid poll JSON, discarded");
        return Ok((None, None));
    };
    let closing = setup["s"] == 1;
    let Some(poll) = from_setup(poll_id, sender, &setup) else {
        return Ok((None, None));
    };
    match store.poll_message(conversation, sender, poll_id)? {
        None if closing => Ok((None, None)),
        None => {
            let body = poll.to_string();
            let stored = if group_sender {
                store.insert_incoming_group(
                    conversation,
                    sender,
                    message_id,
                    "poll",
                    &body,
                    created_at,
                )?
            } else {
                store.insert_incoming(conversation, message_id, "poll", &body, created_at)?
            };
            if let Some(stored) = &stored {
                store.register_poll(conversation, sender, poll_id, stored.id)?;
            }
            Ok((stored, None))
        }
        Some(existing) => {
            let mut current = state(&existing);
            if current["state"] == 1 || !closing {
                return Ok((None, None));
            }
            close_with_results(&mut current, &poll);
            let changed = store.set_message_body(existing.id, &current)?;
            // And a message of its own in the chat, like Android's "Poll closed": the results.
            let body = current.to_string();
            let added = if group_sender {
                store.insert_incoming_group(
                    conversation,
                    sender,
                    message_id,
                    KIND_CLOSED,
                    &body,
                    created_at,
                )?
            } else {
                store.insert_incoming(conversation, message_id, KIND_CLOSED, &body, created_at)?
            };
            Ok((added, changed))
        }
    }
}

/// A poll setup another device of ours sent (multi-device, reflected): a new poll of ours,
/// stored as sent and registered so votes find it, or the closing of one (the poll as
/// changed, the results message as added) — [`receive_setup`]'s steps for our own polls.
pub fn reflect_setup(
    store: &Store,
    conversation: &str,
    user: &str,
    message_id: u64,
    created_at: i64,
    poll_id: u64,
    setup: &str,
) -> Result<(Option<Message>, Option<Message>), StoreError> {
    let Ok(setup) = serde_json::from_str::<Value>(setup) else {
        tracing::warn!("invalid reflected poll JSON, discarded");
        return Ok((None, None));
    };
    let closing = setup["s"] == 1;
    let Some(poll) = from_setup(poll_id, user, &setup) else {
        return Ok((None, None));
    };
    match store.poll_message(conversation, user, poll_id)? {
        None if closing => Ok((None, None)),
        None => {
            let stored = store.insert_reflected_outgoing(
                conversation,
                message_id,
                "poll",
                &poll.to_string(),
                created_at,
            )?;
            if let Some(stored) = &stored {
                store.register_poll(conversation, user, poll_id, stored.id)?;
            }
            Ok((stored, None))
        }
        Some(existing) => {
            let mut current = state(&existing);
            if current["state"] == 1 || !closing {
                return Ok((None, None));
            }
            close_with_results(&mut current, &poll);
            let changed = store.set_message_body(existing.id, &current)?;
            let added = store.insert_reflected_outgoing(
                conversation,
                message_id,
                KIND_CLOSED,
                &current.to_string(),
                created_at,
            )?;
            Ok((added, changed))
        }
    }
}

/// Close a poll with the results of its closing setup (`closing`, as [`from_setup`] reads it): who
/// voted for what and the totals, by choice ID. Nothing else may change (the spec's "ignore any
/// other fields"; Android takes the names too): the creator can't reword the choices under votes
/// already cast. Votes for choices the poll doesn't have count for nothing.
fn close_with_results(current: &mut Value, closing: &Value) {
    let known: Vec<Value> = current["choices"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|choice| choice["id"].clone())
        .collect();
    let mut votes = serde_json::Map::new();
    for (voter, chosen) in closing["votes"].as_object().into_iter().flatten() {
        let chosen: Vec<Value> = chosen
            .as_array()
            .into_iter()
            .flatten()
            .filter(|choice| known.contains(choice))
            .cloned()
            .collect();
        if !chosen.is_empty() {
            votes.insert(voter.clone(), Value::Array(chosen));
        }
    }
    for choice in current["choices"].as_array_mut().into_iter().flatten() {
        let total = closing["choices"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|given| given["id"] == choice["id"])
            .map(|given| given["total"].clone());
        match (choice.as_object_mut(), total) {
            (Some(choice), Some(total)) if !total.is_null() => {
                choice.insert("total".to_owned(), total);
            }
            (Some(choice), _) => {
                choice.remove("total");
            }
            (None, _) => {}
        }
    }
    current["state"] = json!(1);
    current["votes"] = Value::Object(votes);
    current["summary"] = closing["summary"].clone();
}

/// _Common Poll Vote Receive Steps_.
pub fn receive_vote(
    store: &Store,
    conversation: &str,
    voter: &str,
    creator: &str,
    poll_id: u64,
    choices: &str,
) -> Result<Option<Message>, StoreError> {
    let Some(existing) = store.poll_message(conversation, creator, poll_id)? else {
        return Ok(None);
    };
    let mut current = state(&existing);
    if current["state"] == 1 {
        return Ok(None);
    }
    let Ok(choices) = serde_json::from_str::<Value>(choices) else {
        return Ok(None);
    };
    let Some(chosen) = valid_vote(&current, &choices) else {
        return Ok(None);
    };
    current["votes"][voter] = json!(chosen);
    store.set_message_body(existing.id, &current)
}

/// What a received vote chooses, of the choices the poll has. Several in a single choice poll can
/// only come from a broken or dishonest client: the vote is ignored, an earlier one stands.
fn valid_vote(poll: &Value, choices: &Value) -> Option<Vec<Value>> {
    let known = poll["choices"].as_array().cloned().unwrap_or_default();
    let chosen: Vec<Value> = selected(choices)
        .into_iter()
        .filter(|choice| known.iter().any(|k| k["id"] == *choice))
        .collect();
    (poll["multiple"] == true || chosen.len() <= 1).then_some(chosen)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A poll with voters voting on every choice, its results as long as they get.
    fn poll_with_voters(voters: usize, choices: usize) -> Value {
        let voted: Vec<usize> = (0..choices).collect();
        let votes: serde_json::Map<String, Value> = (0..voters)
            .map(|voter| (format!("V{voter:07}"), json!(voted)))
            .collect();
        json!({
            "pollId": "0000000000000001",
            "description": "Lunch?",
            "state": 0,
            "multiple": true,
            "choices": voted.iter().map(|&id| json!({"id": id, "name": "A"})).collect::<Vec<_>>(),
            "votes": votes,
        })
    }

    /// A closing goes out with everyone's votes, or not at all: summary mode isn't open to a poll
    /// that was announced as a list.
    #[test]
    fn closing_a_poll_fits_a_message_or_says_so() {
        assert!(closing_fits(&poll_with_voters(20, 4)));
        let large = poll_with_voters(255, 10);
        assert!(!closing_fits(&large));
        assert_eq!(large["state"], 0);
    }

    /// A poll deleted before this version's rule, by a client that shouldn't have: votes and a
    /// close leave its placeholder alone, and no "Poll closed" without a question appears.
    #[test]
    fn a_deleted_poll_takes_no_votes_and_cant_be_closed() {
        for keep_entry in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
            let setup = json!({"s": 0, "a": 0, "t": 0, "d": "Who?", "c": [{"i": 0, "n": "a"}]});
            let (poll, _) = receive_setup(
                &store,
                "ABCD1234",
                "ABCD1234",
                false,
                1,
                1,
                9,
                &setup.to_string(),
            )
            .unwrap();
            let poll = poll.unwrap();
            store.mark_deleted(poll.id, 5).unwrap();
            if keep_entry {
                store
                    .register_poll("ABCD1234", "ABCD1234", 9, poll.id)
                    .unwrap();
            }

            let vote = receive_vote(&store, "ABCD1234", "B0B0B0B0", "ABCD1234", 9, "[[0, 1]]");
            assert!(vote.unwrap().is_none());
            let closing = json!({"s": 1, "a": 0, "t": 0, "d": "Who?", "p": [], "c": [{"i": 0, "n": "a", "r": []}]});
            let (added, changed) = receive_setup(
                &store,
                "ABCD1234",
                "ABCD1234",
                false,
                2,
                2,
                9,
                &closing.to_string(),
            )
            .unwrap();
            assert!(added.is_none() && changed.is_none());
            let stored = store.message(poll.id).unwrap().unwrap();
            assert_eq!(
                (stored.kind.as_str(), stored.body.as_str()),
                ("deleted", "")
            );
        }
    }

    /// Closing takes the results and nothing else: choice texts stay what people voted for, in a
    /// poll closed by a member and in one closed on another device of ours, and the results
    /// message says the same.
    #[test]
    fn closing_a_poll_keeps_its_choice_texts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let setup = json!({"s": 0, "a": 0, "t": 0, "d": "Who pays?",
                           "c": [{"i": 0, "n": "Alice"}, {"i": 1, "n": "Bob"}]});
        let closing = json!({"s": 1, "a": 0, "t": 0, "d": "Who is nice?", "p": ["B0B0B0B0", "C0C0C0C0"],
                             "c": [{"i": 0, "n": "Bob", "r": [1, 0]}, {"i": 1, "n": "Alice", "r": [0, 1]},
                                   {"i": 7, "n": "Carol", "r": [1, 1]}]});
        let (setup, closing) = (setup.to_string(), closing.to_string());
        let check = |poll: &Message, closed: &Message, changed: &Message| {
            for body in [&closed.body, &changed.body] {
                let body: Value = serde_json::from_str(body).unwrap();
                assert_eq!(body["description"], "Who pays?");
                assert_eq!(body["state"], 1);
                assert_eq!(
                    body["choices"],
                    json!([{"id": 0, "name": "Alice"}, {"id": 1, "name": "Bob"}])
                );
                // The results by choice ID, and the choice that isn't one is gone.
                assert_eq!(body["votes"], json!({"B0B0B0B0": [0], "C0C0C0C0": [1]}));
            }
            assert_eq!(changed.id, poll.id);
        };

        let (poll, _) =
            receive_setup(&store, "ABCD1234", "ABCD1234", false, 1, 1, 9, &setup).unwrap();
        let (closed, changed) =
            receive_setup(&store, "ABCD1234", "ABCD1234", false, 2, 2, 9, &closing).unwrap();
        check(&poll.unwrap(), &closed.unwrap(), &changed.unwrap());

        let (poll, _) = reflect_setup(&store, "ABCD1234", "ECHOECHO", 3, 1, 10, &setup).unwrap();
        let (closed, changed) =
            reflect_setup(&store, "ABCD1234", "ECHOECHO", 4, 2, 10, &closing).unwrap();
        check(&poll.unwrap(), &closed.unwrap(), &changed.unwrap());
    }

    /// A summary poll's totals come with the close, by choice ID.
    #[test]
    fn closing_a_summary_poll_takes_the_totals_by_id() {
        let mut current = json!({"state": 0, "summary": false, "votes": {"B0B0B0B0": [0]},
                                 "choices": [{"id": 0, "name": "a"}, {"id": 1, "name": "b"}]});
        let closing = json!({"summary": true, "votes": {},
                             "choices": [{"id": 1, "name": "x", "total": 3}, {"id": 0, "name": "y", "total": 1}]});
        close_with_results(&mut current, &closing);
        assert_eq!(
            current,
            json!({"state": 1, "summary": true, "votes": {},
                   "choices": [{"id": 0, "name": "a", "total": 1}, {"id": 1, "name": "b", "total": 3}]})
        );
    }

    #[test]
    fn a_received_vote_stays_within_the_poll() {
        let poll = |multiple| json!({"multiple": multiple, "choices": [{"id": 0, "name": "a"}, {"id": 1, "name": "b"}]});
        let vote = |pairs: Value| valid_vote(&poll(false), &pairs);
        // One choice, or none (taking a vote back), in a single choice poll.
        assert_eq!(vote(json!([[0, 1], [1, 0]])), Some(vec![json!(0)]));
        assert_eq!(vote(json!([[0, 0], [1, 0]])), Some(vec![]));
        // Two are refused as a whole, and only the poll's own choices count.
        assert_eq!(vote(json!([[0, 1], [1, 1]])), None);
        assert_eq!(vote(json!([[1, 1], [7, 1]])), Some(vec![json!(1)]));
        // A poll that allows several takes them.
        assert_eq!(
            valid_vote(&poll(true), &json!([[0, 1], [1, 1]])),
            Some(vec![json!(0), json!(1)])
        );
    }

    #[test]
    fn create_vote_close() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        assert!(new_poll(1, "ME", "?", &["only".into()], false, true).is_err());
        assert!(
            new_poll(
                1,
                "ME",
                &"x".repeat(257),
                &["a".into(), "b".into()],
                false,
                true
            )
            .is_err()
        );
        let poll = new_poll(
            9,
            "00000BOB",
            "Lunch?",
            &["Pizza".into(), " Sushi ".into(), "".into()],
            false,
            true,
        )
        .unwrap();
        assert_eq!(poll["choices"].as_array().unwrap().len(), 2);

        // Received from Bob.
        let (added, _) = receive_setup(
            &store,
            "00000BOB",
            "00000BOB",
            false,
            5,
            1,
            9,
            &setup_json(&poll),
        )
        .unwrap();
        let added = added.unwrap();
        assert_eq!(added.kind, "poll");
        // Votes: Bob and us.
        receive_vote(
            &store,
            "00000BOB",
            "00000BOB",
            "00000BOB",
            9,
            "[[0,1],[1,0]]",
        )
        .unwrap();
        let changed = receive_vote(
            &store,
            "00000BOB",
            "ME000001",
            "00000BOB",
            9,
            &vote_json(&poll, &[1]),
        )
        .unwrap()
        .unwrap();
        let state = state(&changed);
        assert_eq!(state["votes"]["00000BOB"], json!([0]));
        assert_eq!(state["votes"]["ME000001"], json!([1]));

        // Bob closes: results replace the votes; afterwards votes are ignored.
        let mut closed = state.clone();
        closed["state"] = json!(1);
        let json = setup_json(&closed);
        let parsed: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["p"], json!(["00000BOB", "ME000001"]));
        assert_eq!(parsed["c"][0]["r"], json!([1, 0]));
        let (results, changed) =
            receive_setup(&store, "00000BOB", "00000BOB", false, 6, 2, 9, &json).unwrap();
        let after = super::state(&changed.unwrap());
        assert_eq!(after["state"], 1);
        assert_eq!(after["votes"], state["votes"]);
        // The closing is a message of its own, with the results.
        let results = results.unwrap();
        assert_eq!(results.kind, KIND_CLOSED);
        assert_eq!(super::state(&results)["votes"], state["votes"]);
        assert!(
            receive_vote(&store, "00000BOB", "00000BOB", "00000BOB", 9, "[[1,1]]")
                .unwrap()
                .is_none()
        );
        // A closing setup for an unknown poll is discarded.
        assert_eq!(
            receive_setup(&store, "00000BOB", "00000BOB", false, 7, 3, 10, &json)
                .unwrap()
                .0
                .map(|m| m.id),
            None
        );
    }
}
