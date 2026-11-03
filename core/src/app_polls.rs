// SPDX-License-Identifier: AGPL-3.0-only
//! Poll commands: create, vote, close. Votes go to everyone if the poll announces them, else
//! only to its creator (who sends the results when closing).

use libthreema::common::MessageId;
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    app_messages::Conversation,
    connection::ToConnection,
    polls::{self, OUTBOX_POLL_VOTE, PollError},
    providers::CoreEvent,
};

impl From<PollError> for ApiError {
    fn from(error: PollError) -> Self {
        match error {
            PollError::TooLong => Self::MessageTooLong,
            PollError::Invalid(reason) => Self::Internal(reason.into()),
        }
    }
}

impl App {
    fn own_identity(&self) -> Result<String, ApiError> {
        Ok(self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id)
    }

    /// The poll message `id` (local) in a conversation, with its state.
    fn poll(&self, id: i64) -> Result<(String, Conversation, i64, Value), ApiError> {
        let message = self
            .store
            .message(id)?
            .filter(|m| m.kind == "poll")
            .ok_or_else(|| ApiError::Internal("no such poll".into()))?;
        let (key, conversation) = self.readable_conversation(&message.conversation)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let state: Value = serde_json::from_str(&message.body).unwrap_or_default();
        Ok((key, conversation, message.id, state))
    }

    /// `createPoll`.
    pub(crate) fn create_poll(
        &mut self,
        identity: &str,
        description: &str,
        choices: &[String],
        multiple: bool,
        announce: bool,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        if !conversation.supports(&self.store, libthreema::common::FeatureMask::POLL_SUPPORT)? {
            return Err(ApiError::NotSupportedByContact);
        }
        let user = self.own_identity()?;
        let poll_id: u64 = rand::random();
        let poll = polls::new_poll(poll_id, &user, description, choices, multiple, announce)?;
        let message =
            self.store
                .insert_outgoing(&key, MessageId::random().0, "poll", &poll.to_string())?;
        self.store.register_poll(&key, &user, poll_id, message.id)?;
        self.prepare_sending(&conversation, &message)?;
        self.tell_connection(ToConnection::SendPending);
        Ok(self.store.message_json(&message))
    }

    /// `vote`: our choices (choice ids; one for single choice polls).
    pub(crate) fn vote(&mut self, id: i64, chosen: &[i64]) -> Result<Value, ApiError> {
        let (key, conversation, local, mut state) = self.poll(id)?;
        if state["state"] == 1 {
            return Err(ApiError::Internal("poll is closed".into()));
        }
        if state["multiple"] != true && chosen.len() > 1 {
            return Err(ApiError::Internal("single choice poll".into()));
        }
        let user = self.own_identity()?;
        let creator = state["creator"].as_str().unwrap_or_default().to_owned();
        state["votes"][&user] = json!(chosen);
        let body = json!({
            "creator": creator,
            "pollId": state["pollId"],
            "choices": polls::vote_json(&state, chosen),
        });
        match &conversation {
            Conversation::Contact(_) => {
                // 1:1: the other one either is the creator or gets the vote announced.
                if creator != user || state["announce"] == true {
                    self.store
                        .outbox_add(&key, MessageId::random().0, OUTBOX_POLL_VOTE, &body)?;
                }
            }
            Conversation::List(_) => return Err(ApiError::NotSupportedByContact),
            Conversation::Group(group) => {
                let receivers: Vec<String> = if state["announce"] == true {
                    group.members.clone()
                } else if creator != user {
                    vec![creator.clone()]
                } else {
                    Vec::new()
                };
                self.store
                    .group_outbox_add(&key, &receivers, OUTBOX_POLL_VOTE, &body)?;
            }
        }
        let changed = self.store.set_message_body(local, &state)?;
        self.tell_connection(ToConnection::SendPending);
        Ok(changed.map_or(Value::Null, |m| self.store.message_json(&m)))
    }

    /// `closePoll` (creator): send the results to everyone, as a message of its own like
    /// Android's "Poll closed" (sent as the closing poll setup). Returns that message; the poll
    /// itself changes too (a `messagesChanged` event).
    pub(crate) fn close_poll(&mut self, id: i64) -> Result<Value, ApiError> {
        let (key, conversation, local, mut state) = self.poll(id)?;
        if state["creator"].as_str() != Some(self.own_identity()?.as_str()) {
            return Err(ApiError::Internal("not our poll".into()));
        }
        if state["state"] == 1 {
            return Ok(Value::Null);
        }
        state["state"] = json!(1);
        let closed = self.store.insert_outgoing(
            &key,
            MessageId::random().0,
            polls::KIND_CLOSED,
            &state.to_string(),
        )?;
        self.prepare_sending(&conversation, &closed)?;
        if let Some(changed) = self.store.set_message_body(local, &state)? {
            let _ = self
                .media_events
                .send(CoreEvent::MessagesChanged(vec![changed]));
        }
        self.tell_connection(ToConnection::SendPending);
        Ok(self.store.message_json(&closed))
    }
}
