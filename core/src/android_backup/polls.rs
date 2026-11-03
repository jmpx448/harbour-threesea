// SPDX-License-Identifier: AGPL-3.0-only
//! Polls in the backup: `ballot.csv`, `ballot_choice.csv`, `ballot_vote.csv`
//! (`BackupService.backupPolls`, `RestoreService.restorePollFile`), and the `BALLOT` message
//! rows whose body is `PollDataModel`: `[type, ballot id]` (1 created, 2 modified, 3 closed;
//! the id is `ballot.csv`'s local `id`). Choices and votes point at their poll with
//! `"<aid>-<creator>"`; a vote names its choice by the choice's api id alone, which this
//! reader resolves within the vote's own poll (Android's restore looks it up globally).
//!
//! Ours: a message of kind `poll` (or `pollClosed`) whose body is the UI state of
//! [`crate::polls`]. Not carried over: `last_viewed_at`, Android's display mode (a closed poll
//! with only totals and no per-voter rows becomes a summary poll), `TEMPORARY` drafts.

use std::collections::{BTreeMap, HashMap};

use serde_json::{Value, json};

use super::csv::{Table, write_row};

/// Android's `PollDataModel.Type` ids.
pub const CREATED: i64 = 1;
pub const CLOSED: i64 = 3;

/// A choice while reading: (order, api id, name, vote count).
type Choice = (i64, i64, String, i64);

/// The archive's polls, by their local `id`, as our UI state.
#[derive(Default)]
pub struct ArchivePolls {
    by_id: HashMap<String, Value>,
}

impl ArchivePolls {
    pub fn read(ballots: &Table, choices: Option<&Table>, votes: Option<&Table>) -> (Self, usize) {
        let mut invalid = 0;
        // uid "<aid>-<creator>" → (local id, poll state, choices (order, id, name, count)).
        let mut polls: HashMap<String, (String, Value, Vec<Choice>)> = HashMap::new();
        for row in &ballots.rows {
            let get = |column: &str| ballots.get(row, column).unwrap_or_default();
            let (id, aid, creator) = (get("id"), get("aid"), get("creator"));
            let Some(poll_id) = crate::app_safe::parse_safe_id(aid) else {
                invalid += 1;
                continue;
            };
            if id.is_empty() || creator.len() != 8 {
                invalid += 1;
                continue;
            }
            let state = match get("state") {
                "OPEN" => 0,
                "CLOSED" => 1,
                // Drafts never left the device; nothing references them.
                _ => continue,
            };
            let poll = json!({
                "pollId": format!("{poll_id:016x}"),
                "creator": creator,
                "description": get("name"),
                "state": state,
                "multiple": get("assessment") == "MULTIPLE_CHOICE",
                "announce": get("type") == "INTERMEDIATE",
                "summary": false,
                "choices": [],
                "votes": {},
            });
            polls.insert(
                format!("{aid}-{creator}"),
                (id.to_owned(), poll, Vec::new()),
            );
        }
        if let Some(table) = choices {
            for row in &table.rows {
                let get = |column: &str| table.get(row, column).unwrap_or_default();
                let Some((_, _, list)) = polls.get_mut(get("ballot")) else {
                    invalid += 1;
                    continue;
                };
                let Ok(choice_id) = get("aid").parse::<i64>() else {
                    invalid += 1;
                    continue;
                };
                let order = get("order").parse().unwrap_or(i64::MAX);
                let count = get("vote_count").parse().unwrap_or(0);
                list.push((order, choice_id, get("name").to_owned(), count));
            }
        }
        // Per poll: voter → chosen ids (every voter has a row per choice, 0 or 1).
        let mut chosen: HashMap<String, BTreeMap<String, Vec<i64>>> = HashMap::new();
        if let Some(table) = votes {
            for row in &table.rows {
                let get = |column: &str| table.get(row, column).unwrap_or_default();
                let uid = get("ballot_uid");
                let (Some(_), Ok(choice), voter) = (
                    polls.get(uid),
                    get("choice_uid").parse::<i64>(),
                    get("identity"),
                ) else {
                    invalid += 1;
                    continue;
                };
                if voter.len() != 8 {
                    invalid += 1;
                    continue;
                }
                let ids = chosen
                    .entry(uid.to_owned())
                    .or_default()
                    .entry(voter.to_owned())
                    .or_default();
                if get("choice") == "1" && !ids.contains(&choice) {
                    ids.push(choice);
                }
            }
        }
        let mut by_id = HashMap::new();
        for (uid, (id, mut poll, mut list)) in polls {
            list.sort_by_key(|(order, choice, _, _)| (*order, *choice));
            let voters = chosen.remove(&uid).unwrap_or_default();
            let known: Vec<i64> = list.iter().map(|(_, id, _, _)| *id).collect();
            let summary = poll["state"] == 1 && voters.is_empty() && list.iter().any(|c| c.3 > 0);
            poll["choices"] = list
                .iter()
                .map(|(_, choice, name, count)| {
                    let mut value = json!({ "id": choice, "name": name });
                    if summary {
                        value["total"] = json!(count);
                    }
                    value
                })
                .collect();
            poll["summary"] = json!(summary);
            poll["votes"] = json!(
                voters
                    .into_iter()
                    .map(|(voter, ids)| {
                        // In the choices' order: the same on every import of the same poll.
                        let ids: Vec<i64> =
                            known.iter().copied().filter(|i| ids.contains(i)).collect();
                        (voter, ids)
                    })
                    .collect::<BTreeMap<_, _>>()
            );
            by_id.insert(id, poll);
        }
        (Self { by_id }, invalid)
    }

    /// A `BALLOT` message: its kind here and the poll state, or `None` (unknown poll, a
    /// "modified" line we do not show).
    pub fn message(&self, body: &str) -> Option<(&'static str, &Value)> {
        let parts: Value = serde_json::from_str(body).ok()?;
        let kind = match parts.get(0)?.as_i64()? {
            CREATED => "poll",
            CLOSED => crate::polls::KIND_CLOSED,
            _ => return None,
        };
        let id = match parts.get(1)? {
            Value::Number(number) => number.to_string(),
            Value::String(text) => text.clone(),
            _ => return None,
        };
        self.by_id.get(&id).map(|poll| (kind, poll))
    }
}

/// The poll a state belongs to: `(creator, poll id)`.
pub fn key(poll: &Value) -> Option<(String, u64)> {
    Some((
        poll["creator"].as_str()?.to_owned(),
        u64::from_str_radix(poll["pollId"].as_str()?, 16).ok()?,
    ))
}

/// Collects the three ballot files while the export walks the chats.
pub struct PollWriter {
    polls: String,
    choices: String,
    votes: String,
    /// (conversation, creator, poll id) → local id, so the closing line points at its poll.
    ids: HashMap<(String, String, u64), i64>,
    next_choice: i64,
    next_vote: i64,
    pub count: usize,
}

impl Default for PollWriter {
    fn default() -> Self {
        Self {
            polls: write_row(&[
                "id",
                "aid",
                "creator",
                "ref",
                "ref_id",
                "name",
                "state",
                "assessment",
                "type",
                "choice_type",
                "last_viewed_at",
                "created_at",
                "modified_at",
            ]),
            choices: write_row(&[
                "id",
                "ballot",
                "aid",
                "type",
                "name",
                "vote_count",
                "order",
                "created_at",
                "modified_at",
            ]),
            votes: write_row(&[
                "id",
                "ballot_uid",
                "choice_uid",
                "identity",
                "choice",
                "created_at",
                "modified_at",
            ]),
            ids: HashMap::new(),
            next_choice: 1,
            next_vote: 1,
            count: 0,
        }
    }
}

impl PollWriter {
    /// The `BALLOT` body for a `poll`/`pollClosed` message, writing the poll on first sight.
    /// `reference`: `("IdentityBallotModel", identity)` or `("GroupBallotModel", group uid)`.
    pub fn message_body(
        &mut self,
        conversation: &str,
        reference: (&str, &str),
        kind: &str,
        poll: &Value,
        created_at: i64,
        modified_at: i64,
    ) -> Option<String> {
        let (creator, poll_id) = key(poll)?;
        let map_key = (conversation.to_owned(), creator.clone(), poll_id);
        let id = match self.ids.get(&map_key) {
            Some(id) => *id,
            None => {
                let id = i64::try_from(self.ids.len()).unwrap_or(i64::MAX - 1) + 1;
                self.ids.insert(map_key, id);
                self.write_poll(
                    id,
                    reference,
                    &creator,
                    poll_id,
                    poll,
                    created_at,
                    modified_at,
                );
                id
            }
        };
        let kind = if kind == crate::polls::KIND_CLOSED {
            CLOSED
        } else {
            CREATED
        };
        Some(json!([kind, id]).to_string())
    }

    #[allow(clippy::too_many_arguments)]
    fn write_poll(
        &mut self,
        id: i64,
        reference: (&str, &str),
        creator: &str,
        poll_id: u64,
        poll: &Value,
        created_at: i64,
        modified_at: i64,
    ) {
        let aid = crate::app_safe::safe_id(poll_id);
        let uid = format!("{aid}-{creator}");
        let (created, modified) = (created_at.to_string(), modified_at.to_string());
        let closed = poll["state"] == 1;
        self.polls.push_str(&write_row(&[
            &id.to_string(),
            &aid,
            creator,
            reference.0,
            reference.1,
            poll["description"].as_str().unwrap_or_default(),
            if closed { "CLOSED" } else { "OPEN" },
            if poll["multiple"] == true {
                "MULTIPLE_CHOICE"
            } else {
                "SINGLE_CHOICE"
            },
            if poll["announce"] == true {
                "INTERMEDIATE"
            } else {
                "RESULT_ON_CLOSE"
            },
            "TEXT",
            "",
            &created,
            &modified,
        ]));
        let choices = poll["choices"].as_array().cloned().unwrap_or_default();
        let votes = poll["votes"].as_object().cloned().unwrap_or_default();
        let chose = |voter: &Value, choice: &Value| {
            voter
                .as_array()
                .is_some_and(|ids| ids.iter().any(|id| id == choice))
        };
        for (order, choice) in choices.iter().enumerate() {
            let count = match choice["total"].as_i64() {
                Some(total) => total,
                None => i64::try_from(votes.values().filter(|v| chose(v, &choice["id"])).count())
                    .unwrap_or_default(),
            };
            self.choices.push_str(&write_row(&[
                &self.next_choice.to_string(),
                &uid,
                &choice["id"].to_string(),
                "Text",
                choice["name"].as_str().unwrap_or_default(),
                &count.to_string(),
                &order.to_string(),
                &created,
                &modified,
            ]));
            self.next_choice += 1;
        }
        for (voter, ids) in &votes {
            for choice in &choices {
                self.votes.push_str(&write_row(&[
                    &self.next_vote.to_string(),
                    &uid,
                    &choice["id"].to_string(),
                    voter,
                    if chose(ids, &choice["id"]) { "1" } else { "0" },
                    &created,
                    &modified,
                ]));
                self.next_vote += 1;
            }
        }
        self.count += 1;
    }

    /// `ballot.csv`, `ballot_choice.csv`, `ballot_vote.csv`.
    pub fn finish(self) -> [(&'static str, String); 3] {
        [
            ("ballot.csv", self.polls),
            ("ballot_choice.csv", self.choices),
            ("ballot_vote.csv", self.votes),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll(state: i32) -> Value {
        json!({
            "pollId": "1122334455667788",
            "creator": "B0B0B0B0",
            "description": "Lunch?",
            "state": state,
            "multiple": true,
            "announce": false,
            "summary": false,
            "choices": [{"id": 0, "name": "Pizza"}, {"id": 1, "name": "Sushi"}],
            "votes": {"B0B0B0B0": [0], "ABCD1234": [0, 1], "ANNANNAN": []},
        })
    }

    fn round_trip(writer: PollWriter) -> ArchivePolls {
        let [(_, polls), (_, choices), (_, votes)] = writer.finish();
        let (read, invalid) = ArchivePolls::read(
            &Table::parse(&polls),
            Some(&Table::parse(&choices)),
            Some(&Table::parse(&votes)),
        );
        assert_eq!(invalid, 0);
        read
    }

    #[test]
    fn round_trips_open_and_closed() {
        let mut writer = PollWriter::default();
        let open = poll(0);
        let created = writer
            .message_body(
                "B0B0B0B0",
                ("IdentityBallotModel", "B0B0B0B0"),
                "poll",
                &open,
                5,
                6,
            )
            .unwrap();
        assert_eq!(created, "[1,1]");
        // The closing line points at the same poll; a second poll gets its own id.
        let closed = writer
            .message_body(
                "B0B0B0B0",
                ("IdentityBallotModel", "B0B0B0B0"),
                crate::polls::KIND_CLOSED,
                &poll(1),
                7,
                7,
            )
            .unwrap();
        assert_eq!(closed, "[3,1]");
        let mut other = poll(1);
        other["pollId"] = json!("0000000000000009");
        assert_eq!(
            writer
                .message_body("g", ("GroupBallotModel", "g"), "poll", &other, 1, 1)
                .unwrap(),
            "[1,2]"
        );
        assert_eq!(writer.count, 2);
        let polls = round_trip(writer);
        let (kind, read) = polls.message(&created).unwrap();
        assert_eq!(kind, "poll");
        assert_eq!(*read, open, "everything survives");
        assert_eq!(polls.message(&closed).unwrap().0, crate::polls::KIND_CLOSED);
        assert_eq!(polls.message("[3,2]").unwrap().1["state"], 1);
        assert!(
            polls.message("[2,1]").is_none(),
            "modified lines are not shown"
        );
        assert!(polls.message("[1,99]").is_none());
    }

    #[test]
    fn android_shapes() {
        // As Android writes them: the aid is the poll id's wire-order hex; choice ids repeat
        // across polls (votes resolve within their poll); a closed summary poll has totals.
        let polls = write_row(&[
            "id",
            "aid",
            "creator",
            "ref",
            "ref_id",
            "name",
            "state",
            "assessment",
            "type",
            "choice_type",
            "last_viewed_at",
            "created_at",
            "modified_at",
        ]) + &write_row(&[
            "4",
            "8877665544332211",
            "B0B0B0B0",
            "IdentityBallotModel",
            "B0B0B0B0",
            "Q1",
            "OPEN",
            "SINGLE_CHOICE",
            "INTERMEDIATE",
            "TEXT",
            "",
            "1",
            "2",
        ]) + &write_row(&[
            "5",
            "0900000000000000",
            "ANNANNAN",
            "GroupBallotModel",
            "g-17",
            "Q2",
            "CLOSED",
            "SINGLE_CHOICE",
            "RESULT_ON_CLOSE",
            "TEXT",
            "",
            "1",
            "2",
        ]) + &write_row(&[
            "6",
            "0a00000000000000",
            "ANNANNAN",
            "GroupBallotModel",
            "g-17",
            "draft",
            "TEMPORARY",
            "SINGLE_CHOICE",
            "RESULT_ON_CLOSE",
            "TEXT",
            "",
            "1",
            "2",
        ]);
        let choices =
            write_row(&[
                "id",
                "ballot",
                "aid",
                "type",
                "name",
                "vote_count",
                "order",
                "created_at",
                "modified_at",
            ]) + &write_row(&[
                "1",
                "8877665544332211-B0B0B0B0",
                "1",
                "Text",
                "B",
                "0",
                "1",
                "",
                "",
            ]) + &write_row(&[
                "2",
                "8877665544332211-B0B0B0B0",
                "0",
                "Text",
                "A",
                "0",
                "0",
                "",
                "",
            ]) + &write_row(&[
                "3",
                "0900000000000000-ANNANNAN",
                "0",
                "Text",
                "X",
                "4",
                "0",
                "",
                "",
            ]) + &write_row(&["4", "missing-ANNANNAN", "0", "Text", "Y", "0", "0", "", ""]);
        let votes = write_row(&[
            "id",
            "ballot_uid",
            "choice_uid",
            "identity",
            "choice",
            "created_at",
            "modified_at",
        ]) + &write_row(&[
            "1",
            "8877665544332211-B0B0B0B0",
            "0",
            "ANNANNAN",
            "0",
            "",
            "",
        ]) + &write_row(&[
            "2",
            "8877665544332211-B0B0B0B0",
            "1",
            "ANNANNAN",
            "1",
            "",
            "",
        ]);
        let (read, invalid) = ArchivePolls::read(
            &Table::parse(&polls),
            Some(&Table::parse(&choices)),
            Some(&Table::parse(&votes)),
        );
        assert_eq!(invalid, 1, "the choice of an unknown poll");
        let (_, first) = read.message("[1,4]").unwrap();
        assert_eq!(first["pollId"], "1122334455667788");
        assert_eq!(key(first), Some(("B0B0B0B0".into(), 0x1122_3344_5566_7788)));
        assert_eq!(first["announce"], true);
        assert_eq!(first["choices"][0]["name"], "A", "by order");
        assert_eq!(first["votes"], json!({"ANNANNAN": [1]}));
        let (_, second) = read.message("[3,5]").unwrap();
        assert_eq!(second["summary"], true);
        assert_eq!(second["choices"][0]["total"], 4);
        assert!(read.message("[1,6]").is_none(), "drafts are skipped");
    }
}
