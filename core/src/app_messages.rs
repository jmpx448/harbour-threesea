// SPDX-License-Identifier: AGPL-3.0-only
//! Chat commands and the connection lifecycle.

use std::rc::Rc;

use libthreema::common::{ChatServerGroup, DeviceCookie, FeatureMask, MessageId};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App},
    connection::{self, ConnectionSetup, FromConnection, ToConnection},
    contacts,
    messages::{EDIT_DELETE_MAX_AGE_MS, Message, OutgoingState, now_ms},
    reactions,
    store::keys,
};

/// Longest text message we accept (Threema's limit is about 7000 bytes of text).
const MAX_TEXT_BYTES: usize = 6000;

/// A conversation: with a contact or a group.
pub(crate) enum Conversation {
    Contact(libthreema::model::contact::Contact),
    Group(crate::groups::Group),
    /// A distribution list: its own chat; messages go to each member as 1:1 messages.
    List(crate::distribution_lists::DistributionList),
}

impl Conversation {
    /// A notes group, where everything goes to our own devices only.
    fn is_notes(&self) -> bool {
        matches!(self, Self::Group(group) if group.is_notes())
    }

    /// Whether the receiver(s) support a feature: the contact, or any group member. A notes group
    /// has no one who could lack it (Android's `GroupMessageReceiver`).
    pub(crate) fn supports(
        &self,
        store: &crate::store::Store,
        feature: u64,
    ) -> Result<bool, ApiError> {
        Ok(match self {
            Self::Contact(contact) => contact.feature_mask.0 & feature != 0,
            Self::Group(group) if group.is_notes() => true,
            Self::Group(group) => {
                let mut any = false;
                for member in &group.members {
                    any |= store
                        .contact(member)?
                        .is_some_and(|c| c.feature_mask.0 & feature != 0);
                }
                any
            }
            // Lists only carry plain messages (texts, files, locations), like Android.
            Self::List(_) => false,
        })
    }

    /// Groups that were left can't be written to.
    /// We can write here: still in the group, the contact's ID not revoked and the contact not
    /// blocked by us (`blocked`: the block list; Android asks to unblock first).
    pub(crate) fn check_writable(&self, blocked: &[String]) -> Result<(), ApiError> {
        match self {
            Self::Group(group) if group.left => Err(ApiError::GroupLeft),
            Self::Contact(contact)
                if contact.activity_state
                    == libthreema::protobuf::d2d_sync::contact::ActivityState::Invalid =>
            {
                Err(ApiError::ContactRevoked)
            }
            Self::Contact(contact) if blocked.contains(&contact.identity.to_string()) => {
                Err(ApiError::ContactBlocked)
            }
            _ => Ok(()),
        }
    }
}

/// How long removing the ID waits for the connection to send a call's end.
const CONNECTION_FLUSH_LIMIT: std::time::Duration = std::time::Duration::from_secs(3);

impl App {
    /// The conversation `identity` (a Threema ID or a group key) refers to, with its key.
    pub(crate) fn conversation(&self, identity: &str) -> Result<(String, Conversation), ApiError> {
        if crate::groups::parse_key(identity).is_some() {
            let group = self.store.group(identity)?.ok_or(ApiError::GroupNotFound)?;
            return Ok((identity.to_owned(), Conversation::Group(group)));
        }
        if crate::distribution_lists::is_key(identity) {
            let list = self
                .store
                .distribution_list(identity)?
                .ok_or(ApiError::GroupNotFound)?;
            return Ok((identity.to_owned(), Conversation::List(list)));
        }
        let id = contacts::parse_identity(identity)
            .ok_or(ApiError::IdentityFormat)?
            .to_string();
        let contact = self.store.contact(&id)?.ok_or(ApiError::ContactNotFound)?;
        Ok((id, Conversation::Contact(contact)))
    }

    /// `chatMedia`: the gallery with each message's kind, the active filter and how many of
    /// each kind there are (the UI hides filters for kinds the chat doesn't have).
    pub(crate) fn chat_media(
        &self,
        identity: &str,
        kinds: Option<Vec<String>>,
    ) -> Result<Value, ApiError> {
        use crate::messages::{MEDIA_KINDS, media_kind};
        let key = self.readable_conversation(identity)?.0;
        let saved = self.store.setting(keys::MEDIA_KINDS)?;
        let wanted: Vec<String> = kinds.unwrap_or_else(|| match saved {
            Some(saved) => saved
                .split(',')
                .filter(|k| !k.is_empty())
                .map(str::to_owned)
                .collect(),
            None => MEDIA_KINDS.iter().map(|k| (*k).to_owned()).collect(),
        });
        let all = self.store.conversation_media(&key, &MEDIA_KINDS)?;
        let mut counts = serde_json::Map::new();
        for kind in MEDIA_KINDS {
            let n = all.iter().filter(|m| media_kind(&m.body) == kind).count();
            counts.insert(kind.to_owned(), json!(n));
        }
        let shown: Vec<_> = all
            .into_iter()
            .filter(|m| wanted.iter().any(|w| w == media_kind(&m.body)))
            .collect();
        let mut messages = self.store.messages_json(&shown);
        for (json, message) in messages.iter_mut().zip(&shown) {
            json["mediaKind"] = json!(media_kind(&message.body));
        }
        Ok(json!({ "messages": messages, "kinds": wanted, "counts": counts }))
    }

    /// Reading a private chat needs the app unlocked (ADR 0009).
    pub(crate) fn check_readable(&self, conversation: &str) -> Result<(), ApiError> {
        if !self.unlocked && self.store.is_private(conversation)? {
            return Err(ApiError::Locked);
        }
        Ok(())
    }

    /// The conversation, if its content may be read or changed now: not a hidden private chat
    /// (ADR 0009; every command that shows or changes messages goes through this or
    /// `check_readable`, deep audit D02).
    pub(crate) fn readable_conversation(
        &self,
        identity: &str,
    ) -> Result<(String, Conversation), ApiError> {
        let (key, conversation) = self.conversation(identity)?;
        self.check_readable(&key)?;
        Ok((key, conversation))
    }

    /// A chat that may be read now, also one without a conversation to write to (`None`): earlier
    /// versions removed a deleted contact's row and left its chat behind (ADR 0022).
    pub(crate) fn readable_chat(
        &self,
        identity: &str,
    ) -> Result<(String, Option<Conversation>), ApiError> {
        let (key, conversation) = match self.conversation(identity) {
            Ok((key, conversation)) => (key, Some(conversation)),
            Err(ApiError::ContactNotFound) => {
                let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
                (id.to_string(), None)
            }
            Err(error) => return Err(error),
        };
        self.check_readable(&key)?;
        Ok((key, conversation))
    }

    /// The media readers for the UI (other threads) follow the lock: while private chats are
    /// hidden, their messages' media are not handed out.
    pub(crate) fn update_media_gate(&self) {
        let hidden = if self.unlocked {
            None
        } else {
            Some(self.store.private_message_ids().unwrap_or_default())
        };
        self.media.set_hidden(hidden);
    }

    /// A new or changed message of a hidden private chat: its media stay hidden too. A chat
    /// marked private (or not) takes its older messages along.
    pub(crate) fn gate_media(&self, event: &crate::providers::CoreEvent) {
        use crate::providers::CoreEvent;
        if matches!(event, CoreEvent::PrivateChanged { .. }) {
            self.update_media_gate();
            return;
        }
        if self.unlocked {
            return;
        }
        let messages: &[crate::messages::Message] = match event {
            CoreEvent::MessageAdded { message, .. } => std::slice::from_ref(message),
            CoreEvent::MessagesChanged(messages) => messages,
            _ => return,
        };
        for message in messages {
            if self.store.is_private(&message.conversation).unwrap_or(true) {
                self.media.hide(message.id);
            }
        }
    }

    /// Whether a result list may show this conversation's messages.
    fn listable(&self, conversation: &str) -> bool {
        self.check_readable(conversation).is_ok()
    }

    pub(crate) fn lock_status(&self) -> Result<Value, ApiError> {
        let lock = self.store.lock()?;
        Ok(json!({
            "lock": lock.map(|lock| json!({
                "keyboard": lock.keyboard,
                "appLock": lock.app_lock,
                "graceSecs": lock.grace_secs,
            })),
            "unlocked": self.unlocked,
            "graceTimes": crate::applock::GRACE_TIMES,
        }))
    }

    /// Store the lock (`None`: remove it, only without private chats).
    pub(crate) fn set_lock(
        &mut self,
        options: Option<crate::applock::LockOptions<'_>>,
    ) -> Result<Value, ApiError> {
        use crate::applock::{Lock, MIN_PIN_LEN};
        let Some(options) = options else {
            if !self.store.private_chats()?.is_empty() {
                return Err(ApiError::PrivateChatsExist);
            }
            self.store.set_lock(None)?;
            // No lock, nothing open: marks arriving later wait for a new PIN.
            self.unlocked = false;
            self.update_media_gate();
            return self.lock_status();
        };
        let current = self.store.lock()?;
        let lock = match (options.new_pin, options.pin, current) {
            (true, Some(pin), _) if pin.chars().count() < MIN_PIN_LEN => {
                return Err(ApiError::PinTooShort);
            }
            (true, Some(pin), _) => {
                Lock::with_pin(pin, options.keyboard, options.app_lock, options.grace_secs)
            }
            // Same PIN, other options.
            (false, _, Some(lock)) => Lock {
                app_lock: options.app_lock,
                grace_secs: options.grace_secs,
                ..lock
            },
            _ => return Err(ApiError::PinTooShort),
        };
        self.store.set_lock(Some(&lock))?;
        self.lock_status()
    }

    /// The PIN opens the lock; without a lock there is nothing to open (for `unlock` that is an
    /// error, `lockNeeded`). Like Android (PinLockViewModel):
    /// after more than three wrong PINs in a row no PIN is checked for a while, 30 s at first and
    /// longer each time up to an hour, until the right one comes. Kept in the store, so
    /// restarting doesn't help (security audit 2026-10-27).
    pub(crate) fn check_lock(&self, pin: Option<&str>) -> Result<(), ApiError> {
        const ATTEMPTS: i64 = 3;
        const PAUSES_SECS: [i64; 7] = [30, 60, 120, 300, 900, 1800, 3600];
        let Some(lock) = self.store.lock()? else {
            return Ok(());
        };
        let number = |key| -> Result<i64, ApiError> {
            Ok(self
                .store
                .setting(key)?
                .and_then(|value| value.parse().ok())
                .unwrap_or(0))
        };
        let step = number(keys::PIN_LOCKOUT_STEP)?.clamp(0, PAUSES_SECS.len() as i64 - 1);
        let now = crate::messages::now_ms();
        // A clock set back can't make a pause longer than it is.
        let until = number(keys::PIN_LOCKOUT_UNTIL)?.min(now + PAUSES_SECS[step as usize] * 1000);
        if until > now {
            return Err(ApiError::PinLockout((until - now + 999) / 1000));
        }
        if pin.is_some_and(|pin| lock.check_pin(pin)) {
            for key in [
                keys::PIN_FAILED_ATTEMPTS,
                keys::PIN_LOCKOUT_UNTIL,
                keys::PIN_LOCKOUT_STEP,
            ] {
                self.store.delete_setting(key)?;
            }
            return Ok(());
        }
        tracing::info!("lock check failed");
        let attempts = number(keys::PIN_FAILED_ATTEMPTS)? + 1;
        if attempts <= ATTEMPTS {
            self.store
                .set_setting(keys::PIN_FAILED_ATTEMPTS, &attempts.to_string())?;
            return Err(ApiError::WrongPin);
        }
        // This step's pause; the next one is longer.
        let pause = PAUSES_SECS[step as usize];
        self.store.delete_setting(keys::PIN_FAILED_ATTEMPTS)?;
        self.store
            .set_setting(keys::PIN_LOCKOUT_UNTIL, &(now + pause * 1000).to_string())?;
        self.store
            .set_setting(keys::PIN_LOCKOUT_STEP, &(step + 1).to_string())?;
        Err(ApiError::PinLockout(pause))
    }

    /// What takes the ID or the chats off this device, or gives someone else a hold on the ID,
    /// needs the PIN whenever a lock exists, whether or not the chats are open (ADR 0009). The
    /// core checks it so that no UI path can skip the prompt. Scheduled backups don't come
    /// through here: the user set them up with the PIN.
    pub(crate) fn check_pin(&self, pin: Option<Zeroizing<String>>) -> Result<(), ApiError> {
        self.check_lock(pin.as_deref().map(String::as_str))
    }

    /// Store a private mark and let everyone who follows them know at once: the media readers
    /// here, and through the event the UI's notifications and the browsers.
    pub(crate) fn mark_private(&self, key: &str, private: bool) -> Result<(), ApiError> {
        self.store.set_private(key, private)?;
        self.update_media_gate();
        let _ = self
            .media_events
            .send(crate::providers::CoreEvent::PrivateChanged {
                conversation: key.to_owned(),
                private,
            });
        Ok(())
    }

    pub(crate) fn set_private(&mut self, identity: &str, private: bool) -> Result<Value, ApiError> {
        let (key, _) = self.conversation(identity)?;
        if self.store.lock()?.is_none() {
            return Err(ApiError::LockNeeded);
        }
        if !private {
            self.check_readable(&key)?;
        }
        self.mark_private(&key, private)?;
        Ok(json!({ "identity": key, "private": private }))
    }

    /// A new outgoing message in a group goes to every member, and it brings its chat back from
    /// the archive.
    pub(crate) fn prepare_sending(
        &self,
        conversation: &Conversation,
        message: &Message,
    ) -> Result<(), ApiError> {
        self.back_from_archive(&message.conversation)?;
        match conversation {
            Conversation::Group(group) => {
                self.store
                    .set_pending_receivers(&group.key, message.message_id, &group.members)?;
            }
            // The list's copy is only a record; texts and locations go out to every member
            // (that is a contact and not blocked) as their own 1:1 message. Files are sent by
            // send_file, which uploads them per member.
            Conversation::List(list) => {
                self.store.raise_outgoing_state(
                    &list.key,
                    &[message.message_id],
                    OutgoingState::Sent,
                )?;
                if message.kind != "file" {
                    for member in self.list_receivers(list)? {
                        self.back_from_archive(&member)?;
                        self.store.insert_outgoing(
                            &member,
                            MessageId::random().0,
                            &message.kind,
                            &message.body,
                        )?;
                    }
                }
            }
            Conversation::Contact(contact) => self.make_direct(contact)?,
        }
        Ok(())
    }

    /// Writing in an archived chat brings it back (Android's `ContactMessageReceiver`,
    /// `GroupMessageReceiver`, `DistributionListMessageReceiver`). The chat list hears of it at once,
    /// the other devices when the command is done (`handle`).
    fn back_from_archive(&self, key: &str) -> Result<(), ApiError> {
        if self.store.unarchive(key)? {
            self.unarchived.borrow_mut().push(key.to_owned());
            let _ = self.media_events.send(crate::providers::CoreEvent::Call(
                json!({ "event": "chatsChanged" }),
            ));
        }
        Ok(())
    }

    /// Writing to someone makes them a direct contact (Android's `ContactMessageReceiver`): with
    /// "block unknown senders" on, it is also what lets the message out and their answer in. The
    /// contact list hears of it at once, the other devices when the command is done (`handle`).
    fn make_direct(&self, contact: &libthreema::model::contact::Contact) -> Result<(), ApiError> {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        if contact.acquaintance_level == AcquaintanceLevel::Direct {
            return Ok(());
        }
        let mut contact = contact.clone();
        contact.acquaintance_level = AcquaintanceLevel::Direct;
        self.store.upsert_contact(&contact)?;
        self.made_direct
            .borrow_mut()
            .push(contact.identity.to_string());
        let _ = self
            .media_events
            .send(crate::providers::CoreEvent::ContactsChanged);
        Ok(())
    }

    /// The members of a list a message of ours goes to right now, each as a 1:1 message. They are
    /// made direct contacts for it, like anyone we write to; revoked and blocked ones are left out,
    /// so that no failed bubble turns up in their chats while the list's copy counts as sent.
    pub(crate) fn list_receivers(
        &self,
        list: &crate::distribution_lists::DistributionList,
    ) -> Result<Vec<String>, ApiError> {
        use libthreema::protobuf::d2d_sync::contact::ActivityState;
        let mut receivers = Vec::new();
        for member in &list.members {
            let Some(contact) = self.store.contact(member)? else {
                continue;
            };
            if contact.activity_state == ActivityState::Invalid
                || self.store.is_explicitly_blocked(member)?
            {
                continue;
            }
            self.make_direct(&contact)?;
            if crate::providers::may_receive(&self.store, &self.config, member, false)? {
                receivers.push(member.clone());
            }
        }
        Ok(receivers)
    }

    /// Queue an outbox message (reaction, edit, deletion) for the conversation.
    pub(crate) fn queue(
        &self,
        key: &str,
        conversation: &Conversation,
        kind: &str,
        body: &Value,
    ) -> Result<(), ApiError> {
        match conversation {
            Conversation::Contact(_) => {
                self.store
                    .outbox_add(key, MessageId::random().0, kind, body)?;
            }
            // No receivers, but our other devices hear of it when it is "sent" (`send_to_group`).
            Conversation::Group(group) if group.is_notes() => {
                self.store
                    .outbox_add(key, MessageId::random().0, kind, body)?;
            }
            Conversation::Group(group) => {
                self.store
                    .group_outbox_add(key, &group.members, kind, body)?;
            }
            // Reactions, edits and deletions for everyone don't exist for lists.
            Conversation::List(_) => return Err(ApiError::NotSupportedByContact),
        }
        Ok(())
    }

    pub(crate) fn device_cookie(&self) -> Result<DeviceCookie, ApiError> {
        let hex = match self.store.setting(keys::DEVICE_COOKIE)? {
            Some(hex) => hex,
            None => {
                let hex = data_encoding::HEXLOWER.encode(&rand::random::<[u8; 16]>());
                self.store.set_setting(keys::DEVICE_COOKIE, &hex)?;
                self.store.set_setting(keys::DEVICE_COOKIE_FRESH, "1")?;
                hex
            }
        };
        let bytes: [u8; 16] = data_encoding::HEXLOWER
            .decode(hex.as_bytes())
            .ok()
            .and_then(|b| b.try_into().ok())
            .ok_or_else(|| ApiError::Internal("stored device cookie invalid".into()))?;
        Ok(DeviceCookie(bytes))
    }

    /// Start the chat server connection if there is an identity and it is not running yet.
    /// Returns the receiving end for connection events when newly started.
    pub fn ensure_connection(&mut self) -> Option<mpsc::UnboundedReceiver<FromConnection>> {
        if self.connection.as_ref().is_some_and(|tx| !tx.is_closed()) {
            return None;
        }
        // A store that refused to empty still holds the ID, which is on its way out.
        if self.identity_removed {
            return None;
        }
        if !self.license_valid().ok()? {
            return None;
        }
        let identity = self.store.identity().ok().flatten()?;
        let setup = ConnectionSetup {
            store: Rc::clone(&self.store),
            media: Arc::clone(&self.media),
            https: self.https.clone(),
            config: Arc::clone(&self.config),
            identity: identity.threema_id.parse().ok()?,
            client_key: Zeroizing::new(*identity.client_key),
            server_group: ChatServerGroup::try_from(identity.server_group.as_str()).ok()?,
            device_cookie: self.device_cookie().ok()?,
            client_info: self.version.client_info(),
            fs_mask_checks: connection::FsMaskChecks::default(),
            storage_failing: connection::StorageFailing::default(),
            failing_items: connection::FailingItems::default(),
            carried_events: core::cell::RefCell::default(),
            wake_hold: connection::WakeHold::default(),
        };
        let (to_tx, to_rx) = mpsc::unbounded_channel();
        let (from_tx, from_rx) = mpsc::unbounded_channel();
        self.connection_task = Some(tokio::task::spawn_local(connection::run(
            setup, to_rx, from_tx,
        )));
        self.connection = Some(to_tx);
        Some(from_rx)
    }

    /// Stop the connection, wherever it is: after this it sends nothing more, also if it was in the
    /// middle of logging in. The run loop drops what it had said already
    /// (`runtime::forget_stopped_connection`).
    pub(crate) fn stop_connection(&mut self) {
        self.connection = None;
        self.connection_state = "disconnected";
        if let Some(task) = &self.connection_task {
            task.abort();
        }
    }

    /// Wait until a stopped connection's task is really gone, with what it was still writing to the
    /// store rolled back or done: what follows needs the store to itself (a wipe).
    pub(crate) async fn connection_gone(&mut self) {
        if let Some(task) = self.connection_task.take() {
            let _ = task.await;
        }
    }

    /// Wait until the connection has handled what it was told so far, within a short limit.
    pub(crate) async fn connection_flushed(&self) {
        if self.connection.is_none() {
            return;
        }
        let (reply, answer) = tokio::sync::oneshot::channel();
        self.tell_connection(ToConnection::Flush(reply));
        // Offline the answer is dropped at once; connecting or busy it may not come in time.
        let _ = tokio::time::timeout(CONNECTION_FLUSH_LIMIT, answer).await;
    }

    pub(crate) fn tell_connection(&self, message: ToConnection) {
        if let Some(tx) = &self.connection {
            let _ = tx.send(message);
        }
    }

    /// Ask the connection to send what is pending (e.g. a file whose upload just finished).
    pub(crate) fn send_pending_messages(&self) {
        self.tell_connection(ToConnection::SendPending);
    }

    /// Wake the connection, without the user asking (timer, network changed, back in the app).
    /// `waiting`: `wakeSettled` will follow.
    pub(crate) fn wake_connection(&mut self) -> Value {
        // The word that is held back is for a wake-up gone by.
        self.wake_settled_held = false;
        // The hourly tick goes by tokio's clock, which stands still while the device sleeps: with
        // short wake-ups an hour of it can take days.
        self.maintenance_overdue = self
            .maintenance_ran_at
            .elapsed()
            .is_ok_and(|since| since >= crate::app::MAINTENANCE_INTERVAL);
        let waiting = self
            .connection
            .as_ref()
            .is_some_and(|tx| tx.send(ToConnection::Wake).is_ok());
        // The loop's timer stands still while the phone sleeps, a pause goes by the clock.
        self.retry_failed_transfers();
        json!({ "waiting": waiting })
    }

    /// The user asked to connect now: that also ends a wait a server asked for.
    pub(crate) fn connect_now(&mut self) -> Value {
        self.tell_connection(ToConnection::ConnectNow);
        self.retry_failed_transfers();
        json!({})
    }

    pub(crate) fn connection_json(&self) -> Value {
        json!({ "state": self.connection_state })
    }

    /// The chat list, the archive or (`private`, unlocked only) the private chats. Private chats
    /// are only ever in their own list.
    pub(crate) fn conversations_json(
        &self,
        archived: bool,
        private: bool,
    ) -> Result<Value, ApiError> {
        if private && !self.unlocked {
            return Err(ApiError::Locked);
        }
        // Everything the rows need, in one query each instead of one per chat: with a few
        // hundred chats the per-chat lookups were the whole cost of this list.
        let mut list = Vec::new();
        let archive = self.store.archived_chats()?;
        let hidden = self.store.private_chats()?;
        let marked_unread = self.store.marked_unread_chats()?;
        let muted = self.store.muted_chats()?;
        let picture_versions = self.store.shown_contact_picture_versions()?;
        let contacts: std::collections::HashMap<String, _> = self
            .store
            .contacts()?
            .into_iter()
            .map(|contact| (contact.identity.to_string(), contact))
            .collect();
        let groups: std::collections::HashMap<String, _> = self
            .store
            .groups()?
            .into_iter()
            .map(|group| (group.key.clone(), group))
            .collect();
        let lists: std::collections::HashMap<String, _> = self
            .store
            .distribution_lists()?
            .into_iter()
            .map(|list| (list.key.clone(), list))
            .collect();
        let display_name = |identity: &str| {
            contacts
                .get(identity)
                .map_or_else(|| identity.to_owned(), contacts::display_name)
        };
        let (mut archived_count, mut private_count) = (0, 0);
        // The cover's unread total and who it names, counted over the chat list alone (not the
        // archive, not the private chats) whichever list was asked for, so opening the archive
        // doesn't skew them. `conversations()` is newest first, so the first unread chat is the
        // one to name: who is still waiting, not whoever wrote last.
        let mut unread_total = 0_i64;
        let mut unread_chat: Option<String> = None;
        for (last, unread) in self.store.conversations()? {
            let is_private = hidden.contains(&last.conversation);
            private_count += usize::from(is_private);
            if !is_private && unread > 0 && !archive.contains(&last.conversation) {
                unread_total += unread;
                if unread_chat.is_none() {
                    unread_chat = Some(last.conversation.clone());
                }
            }
            if is_private != private {
                continue;
            }
            if !private && archive.contains(&last.conversation) != archived {
                archived_count += usize::from(!archived);
                continue;
            }
            if let Some(group) = groups.get(&last.conversation) {
                list.push(json!({
                    "identity": last.conversation,
                    "displayName": crate::app_groups::title(&group.name, &group.members, &display_name),
                    "group": true,
                    "left": group.left,
                    "pictureVersion": group.picture_version,
                    "verification": 0,
                    "unread": unread,
                    "markedUnread": marked_unread.contains(&last.conversation),
                    "muted": muted.contains(&last.conversation),
                    "lastMessage": self.store.message_json(&last),
                    "senderName": last.sender.as_deref().map(&display_name),
                }));
                continue;
            }
            if let Some(dl) = lists.get(&last.conversation) {
                list.push(json!({
                    "identity": last.conversation,
                    "displayName": crate::app_groups::title(&dl.name, &dl.members, &display_name),
                    "distributionList": true,
                    "pictureVersion": 0,
                    "verification": 0,
                    "unread": 0,
                    "muted": false,
                    "lastMessage": self.store.message_json(&last),
                }));
                continue;
            }
            let contact = contacts.get(&last.conversation);
            list.push(json!({
                "identity": last.conversation,
                "displayName": contact.map_or_else(|| last.conversation.clone(), contacts::display_name),
                "verification": contact.map_or(0, |c| c.verification_level as i32),
                "unread": unread,
                "markedUnread": marked_unread.contains(&last.conversation),
                "pictureVersion": picture_versions.get(&last.conversation).copied().unwrap_or(0),
                "muted": muted.contains(&last.conversation),
                "lastMessage": self.store.message_json(&last),
            }));
        }
        // Pinned chats first, in the user's order (`pinned`: their place), then by last message.
        let pinned = self.store.pinned_chats()?;
        let drafts = self.store.settings_with_prefix("draft:")?;
        for entry in &mut list {
            if let Some(id) = entry["identity"].as_str()
                && let Some(draft) = drafts.get(id)
            {
                entry["draft"] = draft.clone().into();
                let (mentions, _) = self
                    .store
                    .mention_names(&crate::mentions::identities(draft));
                if !mentions.is_empty() {
                    entry["draftMentions"] = Value::Object(mentions);
                }
            }
            let place = entry["identity"]
                .as_str()
                .and_then(|id| pinned.iter().position(|p| p == id));
            entry["pinned"] = place.map_or(Value::Null, |place| json!(place));
        }
        list.sort_by_key(|entry| entry["pinned"].as_u64().unwrap_or(u64::MAX));
        let unread_from = unread_chat.as_deref().map(|key| {
            if let Some(group) = groups.get(key) {
                crate::app_groups::title(&group.name, &group.members, &display_name)
            } else if let Some(dl) = lists.get(key) {
                crate::app_groups::title(&dl.name, &dl.members, &display_name)
            } else {
                display_name(key)
            }
        });
        Ok(json!({
            "conversations": list,
            "archivedCount": archived_count,
            "privateCount": private_count,
            "unreadTotal": unread_total,
            "unreadFrom": unread_from,
        }))
    }

    pub(crate) fn messages_json(
        &self,
        identity: &str,
        limit: Option<u32>,
        around: Option<i64>,
        before: Option<i64>,
        after: Option<i64>,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_chat(identity)?;
        // Scrolling: just the page on that side of the message, nothing else.
        if before.is_some() || after.is_some() {
            let messages = match (before, after) {
                (Some(before), _) => {
                    self.store
                        .conversation_messages_before(&key, before, limit.unwrap_or(100))?
                }
                (_, Some(after)) => {
                    self.store
                        .conversation_messages_newer(&key, after, limit.unwrap_or(100))?
                }
                _ => Vec::new(),
            };
            return Ok(json!({
                "identity": key,
                "messages": self.store.messages_json(&messages),
                "private": self.store.is_private(&key)?,
            }));
        }
        // Opening a 1:1 chat (the initial load, not the follow-ups) may ask the contact for a
        // missing picture: bounded by the request backoff, never for blocked contacts.
        if around.is_none()
            && matches!(conversation, Some(Conversation::Contact(_)))
            && !self.app_settings()?.blocked_contacts.contains(&key)
            && self.store.request_picture(&key)?
        {
            self.tell_connection(ToConnection::SendPending);
        }
        // The first unread and enough context around it, so the chat can open there and mark
        // where the new messages start; bounded for a chat with thousands unread.
        let (unread, first_unread) = self.store.unread_position(&key)?;
        // A jump to a message stays a window whatever is unread: the reader asked for that place.
        let (messages, reaches_newest) = match around {
            Some(around) => {
                let side = (limit.unwrap_or(100) / 2).max(1);
                self.store
                    .conversation_messages_around(&key, around, side)?
            }
            None => {
                let limit = limit
                    .unwrap_or(200)
                    .max(u32::try_from(unread + 25).unwrap_or(u32::MAX).min(1000));
                (self.store.conversation_messages(&key, limit)?, true)
            }
        };
        Ok(json!({
            "identity": key,
            "messages": self.store.messages_json(&messages),
            "private": self.store.is_private(&key)?,
            "firstUnread": first_unread,
            "reachesNewest": reaches_newest,
        }))
    }

    /// Message JSON for result lists: with the chat's name, whether it is a group, the sender.
    fn result_json(&self, message: &Message) -> Value {
        let mut json = self.store.message_json(message);
        json["displayName"] = match self.store.group(&message.conversation) {
            Ok(Some(group)) => self.group_title(&group).into(),
            _ => match self.store.distribution_list(&message.conversation) {
                Ok(Some(list)) => self.list_title(&list).into(),
                _ => self.display_name_of(&message.conversation).into(),
            },
        };
        json["group"] = crate::groups::parse_key(&message.conversation)
            .is_some()
            .into();
        if let Some(sender) = &message.sender {
            json["senderName"] = self.display_name_of(sender).into();
        }
        json
    }

    pub(crate) fn starred_messages_json(&self, identity: Option<&str>) -> Result<Value, ApiError> {
        let conversation = identity
            .map(|id| self.conversation(id))
            .transpose()?
            .map(|c| c.0);
        let results: Vec<Value> = self
            .store
            .starred_messages(conversation.as_deref())?
            .iter()
            .filter(|message| self.listable(&message.conversation))
            .map(|message| {
                let mut json = self.result_json(message);
                json["snippet"] = json["text"].clone();
                json
            })
            .collect();
        Ok(json!({ "results": results }))
    }

    pub(crate) fn search_messages(
        &self,
        query: &str,
        identity: Option<&str>,
    ) -> Result<Value, ApiError> {
        let query = query.trim();
        if query.chars().count() < 2 {
            return Ok(json!({ "results": [] }));
        }
        let conversation = identity
            .map(|id| self.conversation(id))
            .transpose()?
            .map(|c| c.0);
        let names = |id: &str| {
            self.store
                .contact(id)
                .ok()
                .flatten()
                .map(|c| contacts::display_name(&c))
        };
        let results: Vec<Value> = self
            .store
            .search_messages(query, conversation.as_deref(), 100, names)?
            .iter()
            .filter(|(message, _)| self.listable(&message.conversation))
            .map(|(message, hit)| {
                let mut json = self.result_json(message);
                json["snippet"] = hit.as_str().into();
                json
            })
            .collect();
        Ok(json!({ "results": results }))
    }

    /// A message of the chat (either direction) by message id; null if not (or no longer) here.
    pub(crate) fn quoted_message(
        &self,
        identity: &str,
        message_id: &str,
    ) -> Result<Value, ApiError> {
        let (key, _) = self.readable_conversation(identity)?;
        self.check_readable(&key)?;
        let id = u64::from_str_radix(message_id, 16)
            .map_err(|_| ApiError::Internal("invalid message id".into()))?;
        // Quotes we wrote before 2026-10-27 named the id byte-reversed: those still resolve.
        for id in [id, id.swap_bytes()] {
            for outgoing in [false, true] {
                if let Some(message) = self.store.message_by_id(&key, outgoing, id)? {
                    return Ok(self.store.message_json(&message));
                }
            }
        }
        Ok(Value::Null)
    }

    pub(crate) fn send_text(
        &mut self,
        identity: &str,
        text: &str,
        quote: Option<&str>,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        let text = text.trim();
        if text.is_empty() {
            return Err(ApiError::Internal("empty message".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(ApiError::MessageTooLong);
        }
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let quote = quote
            .map(|id| u64::from_str_radix(id, 16))
            .transpose()
            .map_err(|_| ApiError::Internal("invalid quote".into()))?;
        let body = crate::messages::with_quote(quote, text);
        let message = self
            .store
            .insert_outgoing(&key, MessageId::random().0, "text", &body)?;
        self.prepare_sending(&conversation, &message)?;
        self.forget_sent_draft(&key, text);
        // Its state may have changed (a list's copy counts as sent).
        let message = self.store.message(message.id)?.unwrap_or(message);
        self.tell_connection(ToConnection::SendPending);
        Ok(self.store.message_json(&message))
    }

    /// `setDraft`: what the chat's input holds, its mentions as `@[ID]`, with the message it
    /// quotes. A draft without text is none, and its quote goes with it.
    pub(crate) fn set_draft(
        &mut self,
        identity: &str,
        text: &str,
        quote: Option<&str>,
    ) -> Result<Value, ApiError> {
        let key = self.readable_conversation(identity)?.0;
        if text.trim().is_empty() {
            self.remove_draft(&key)?;
            return Ok(json!({}));
        }
        let quote = quote
            .map(|id| u64::from_str_radix(id, 16).map(|_| id))
            .transpose()
            .map_err(|_| ApiError::Internal("invalid quote".into()))?;
        self.store.set_setting(&format!("draft:{key}"), text)?;
        let quote_key = format!("draftQuote:{key}");
        match quote {
            Some(id) => self.store.set_setting(&quote_key, id)?,
            None => self.store.delete_setting(&quote_key)?,
        }
        Ok(json!({}))
    }

    /// `draft`: `{text, mentions, quote}`, the quoted message as JSON (none if it is gone).
    pub(crate) fn draft(&self, identity: &str) -> Result<Value, ApiError> {
        let key = self.conversation(identity)?.0;
        self.check_readable(&key)?;
        let Some(text) = self.store.setting(&format!("draft:{key}"))? else {
            return Ok(json!({ "text": null }));
        };
        let (mentions, _) = self
            .store
            .mention_names(&crate::mentions::identities(&text));
        let quote = match self.store.setting(&format!("draftQuote:{key}"))? {
            Some(id) => self.quoted_message(identity, &id)?,
            None => Value::Null,
        };
        Ok(json!({ "text": text, "mentions": mentions, "quote": quote }))
    }

    pub(crate) fn remove_draft(&self, key: &str) -> Result<(), ApiError> {
        self.store.delete_setting(&format!("draft:{key}"))?;
        self.store.delete_setting(&format!("draftQuote:{key}"))?;
        Ok(())
    }

    /// A text or caption the core has accepted: if it is what the draft held, the draft has gone
    /// out. The UI writes the draft before it sends, so whatever becomes of the send, and of
    /// the app, the words are in one of the two; anything else in the draft is newer, and stays.
    pub(crate) fn forget_sent_draft(&self, key: &str, sent: &str) {
        let draft = self.store.setting(&format!("draft:{key}"));
        if matches!(&draft, Ok(Some(draft)) if draft.trim() == sent.trim())
            && let Err(error) = self.remove_draft(key)
        {
            tracing::warn!(%error, "removing a sent draft failed");
        }
    }

    /// `sendLocation`: a location message (coordinates from the device's positioning, or a
    /// picked point; the name and address are optional).
    pub(crate) fn send_location(
        &mut self,
        identity: &str,
        location: &Value,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        let body = location.to_string();
        if crate::messages::location_message(&body).is_none() {
            return Err(ApiError::Internal("invalid location".into()));
        }
        let message = self
            .store
            .insert_outgoing(&key, MessageId::random().0, "location", &body)?;
        self.prepare_sending(&conversation, &message)?;
        // Its state may have changed (a list's copy counts as sent).
        let message = self.store.message(message.id)?.unwrap_or(message);
        self.tell_connection(ToConnection::SendPending);
        Ok(self.store.message_json(&message))
    }

    /// Mark a conversation read; sends read receipts if enabled (globally or per contact).
    pub(crate) fn mark_read(&mut self, identity: &str) -> Result<Value, ApiError> {
        let (id, conversation) = self.readable_chat(identity)?;
        let read = self.store.mark_conversation_read(&id)?;
        // Also when read elsewhere than in the app's chat (Threema Web, a notification's action):
        // the UI takes its notification away and recounts.
        if !read.is_empty() {
            let _ = self.media_events.send(crate::providers::CoreEvent::Call(
                json!({ "event": "conversationRead", "identity": id }),
            ));
        }
        // Groups have no read receipts (lists have nothing incoming); no contact, no way to send.
        if !matches!(conversation, Some(Conversation::Contact(_))) {
            return Ok(json!({ "read": read.len() }));
        }
        let send = match self
            .store
            .contact(&id)?
            .and_then(|c| c.read_receipt_policy_override)
        {
            Some(libthreema::protobuf::d2d_sync::ReadReceiptPolicy::SendReadReceipt) => true,
            Some(libthreema::protobuf::d2d_sync::ReadReceiptPolicy::DontSendReadReceipt) => false,
            None => self.app_settings()?.read_receipts,
        };
        if send && !read.is_empty() {
            self.tell_connection(ToConnection::ReadReceipts {
                identity: id,
                message_ids: read.clone(),
            });
        }
        Ok(json!({ "read": read.len() }))
    }

    /// Tell the contact we are (not) typing, if enabled globally or for the contact.
    pub(crate) fn set_typing(&mut self, identity: &str, typing: bool) -> Result<Value, ApiError> {
        // No typing indicators in groups.
        if crate::groups::parse_key(identity).is_some() {
            return Ok(json!({ "sent": false }));
        }
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let id = id.to_string();
        let Some(contact) = self.store.contact(&id)? else {
            return Err(ApiError::ContactNotFound);
        };
        let send = match contact.typing_indicator_policy_override {
            Some(libthreema::protobuf::d2d_sync::TypingIndicatorPolicy::SendTypingIndicator) => {
                true
            }
            Some(
                libthreema::protobuf::d2d_sync::TypingIndicatorPolicy::DontSendTypingIndicator,
            ) => false,
            None => self.app_settings()?.send_typing,
        };
        if send {
            self.tell_connection(ToConnection::Typing {
                identity: id,
                typing,
                at: tokio::time::Instant::now(),
            });
        }
        Ok(json!({ "sent": send }))
    }

    /// The 1:1 reaction submit steps: a `Reaction` if the contact has `REACTION_SUPPORT` (we
    /// always do), else the legacy thumbs up/down receipt; applied locally right away.
    pub(crate) fn react(
        &mut self,
        identity: &str,
        message_id: &str,
        emoji: &str,
        apply: bool,
    ) -> Result<Value, ApiError> {
        let (id, conversation) = self.readable_conversation(identity)?;
        conversation.check_writable(&self.app_settings()?.blocked_contacts)?;
        if emoji.len() > 64 || !reactions::is_emoji(emoji) {
            return Err(ApiError::Internal("invalid emoji".into()));
        }
        let referred = u64::from_str_radix(message_id, 16)
            .map_err(|_| ApiError::Internal("invalid message id".into()))?;
        let local = self
            .store
            .reactable_message(&id, referred)?
            .ok_or_else(|| ApiError::Internal("no such message".into()))?;
        let supported = conversation.supports(
            &self.store,
            libthreema::common::FeatureMask::REACTION_SUPPORT,
        )?;
        let body = json!({ "messageId": message_id, "emoji": emoji, "apply": apply });
        let now = crate::messages::now_ms();
        let changed = if supported {
            self.queue(&id, &conversation, reactions::OUTBOX_REACTION, &body)?;
            self.store
                .set_reaction(local, reactions::OWN_SENDER, emoji, apply, now)?
        } else if apply && reactions::legacy_reaction(emoji).is_some() {
            self.queue(&id, &conversation, reactions::OUTBOX_LEGACY_REACTION, &body)?;
            self.store
                .replace_reactions(local, reactions::OWN_SENDER, emoji, now)?
        } else {
            return Err(ApiError::ReactionNotSupported);
        };
        self.tell_connection(ToConnection::SendPending);
        Ok(changed.map_or(Value::Null, |message| self.store.message_json(&message)))
    }

    /// Our own message `message_id` (hex) in a conversation with a contact.
    fn own_message(
        &self,
        identity: &str,
        message_id: &str,
    ) -> Result<(String, Conversation, Message), ApiError> {
        let (id, contact) = self.readable_conversation(identity)?;
        let message_id = u64::from_str_radix(message_id, 16)
            .map_err(|_| ApiError::Internal("invalid message id".into()))?;
        let message = self
            .store
            .message_by_id(&id, true, message_id)?
            .ok_or_else(|| ApiError::Internal("no such message".into()))?;
        Ok((id, contact, message))
    }

    /// The edit submit steps. A message whose send hasn't begun is just changed. One whose send has
    /// may be with anyone already, and a resend under its ID is dropped by those who have it, so it
    /// is edited like a sent one: a resend carries the new text to those who don't have it, the edit
    /// message to those who do. Notes have no age limit, here and in `delete_message` (Android's
    /// `canBeEdited`, `canDeleteRemotely`).
    pub(crate) fn edit_message(
        &mut self,
        identity: &str,
        message_id: &str,
        text: &str,
    ) -> Result<Value, ApiError> {
        let (id, contact, message) = self.own_message(identity, message_id)?;
        let text = text.trim();
        if message.kind != "text" || text.is_empty() {
            return Err(ApiError::Internal("not an editable message".into()));
        }
        if text.len() > MAX_TEXT_BYTES {
            return Err(ApiError::MessageTooLong);
        }
        if text == crate::messages::split_quote(&message.body).1 {
            return Ok(self.store.message_json(&message));
        }
        let changed = if message.state == OutgoingState::Pending as i64 && !message.written_out {
            self.store.edit_message(message.id, text, None)?
        } else {
            if message.state == OutgoingState::Failed as i64 {
                return Err(ApiError::Internal("message was not sent".into()));
            }
            contact.check_writable(&self.app_settings()?.blocked_contacts)?;
            if !contact.supports(&self.store, FeatureMask::EDIT_MESSAGE_SUPPORT)? {
                return Err(ApiError::NotSupportedByContact);
            }
            if !contact.is_notes() && now_ms() - message.created_at > EDIT_DELETE_MAX_AGE_MS {
                return Err(ApiError::MessageTooOld);
            }
            self.queue(
                &id,
                &contact,
                reactions::OUTBOX_EDIT,
                &json!({
                    "messageId": format!("{:016x}", message.message_id),
                    // The bare text: receivers keep the original's quote, and Android would show
                    // a quote marker in here as text.
                    "text": text,
                }),
            )?;
            self.store.edit_message(message.id, text, Some(now_ms()))?
        };
        self.tell_connection(ToConnection::SendPending);
        Ok(changed.map_or(Value::Null, |message| self.store.message_json(&message)))
    }

    /// Delete a message on this device, or (ours, the delete submit steps) for everyone: replaced
    /// by a placeholder; a message nobody can have yet is just removed.
    pub(crate) fn delete_message(
        &mut self,
        identity: &str,
        message_id: &str,
        outgoing: bool,
        everyone: bool,
    ) -> Result<Value, ApiError> {
        if !everyone {
            let id = self.readable_conversation(identity)?.0;
            let message_id = u64::from_str_radix(message_id, 16)
                .map_err(|_| ApiError::Internal("invalid message id".into()))?;
            if let Some(message) = self.store.message_by_id(&id, outgoing, message_id)? {
                self.store.remove_message(message.id)?;
                self.forget_media(message.id);
            }
            return Ok(json!({ "removed": true }));
        }
        let (id, contact, message) = self.own_message(identity, message_id)?;
        if !crate::messages::delete_applies_to(&message.kind) {
            return Err(ApiError::Internal("not a deletable message".into()));
        }
        if (message.state == OutgoingState::Pending as i64 && !message.written_out)
            || message.state == OutgoingState::Failed as i64
        {
            self.store.remove_message(message.id)?;
            self.forget_media(message.id);
            return Ok(json!({ "removed": true }));
        }
        contact.check_writable(&self.app_settings()?.blocked_contacts)?;
        if !contact.supports(&self.store, FeatureMask::DELETE_MESSAGE_SUPPORT)? {
            return Err(ApiError::NotSupportedByContact);
        }
        if !contact.is_notes() && now_ms() - message.created_at > EDIT_DELETE_MAX_AGE_MS {
            return Err(ApiError::MessageTooOld);
        }
        self.queue(
            &id,
            &contact,
            reactions::OUTBOX_DELETE,
            &json!({ "messageId": format!("{:016x}", message.message_id) }),
        )?;
        let changed = self.store.mark_deleted(message.id, now_ms())?;
        self.forget_media(message.id);
        self.tell_connection(ToConnection::SendPending);
        Ok(changed.map_or(Value::Null, |message| self.store.message_json(&message)))
    }

    pub(crate) fn delete_conversation(&mut self, identity: &str) -> Result<Value, ApiError> {
        let (id, conversation) = self.readable_chat(identity)?;
        match conversation {
            // A group's conversation goes with the group, which must have been left.
            Some(Conversation::Group(group)) if group.left => {
                self.reflect_group_deleted(&group)?;
                self.store.delete_group(&id)?;
                self.media.remove_picture(&id);
            }
            // A list's chat goes with the list.
            Some(Conversation::List(_)) => self.store.delete_distribution_list(&id)?,
            Some(Conversation::Group(_) | Conversation::Contact(_)) | None => {
                self.store.delete_conversation(&id)?;
            }
        }
        self.chat_removed(&id);
        Ok(json!({}))
    }

    /// A chat's rows are gone: its media go too. The UI takes its notification, and with it the
    /// notification's action token, away; a page still open on it no longer keeps a draft. Of a
    /// hidden private chat (deleted on another device) the UI hears nothing.
    pub(crate) fn chat_removed(&self, key: &str) {
        let store = Rc::clone(&self.store);
        self.media
            .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
        if self.check_readable(key).is_err() {
            return;
        }
        for event in ["conversationRead", "chatDeleted"] {
            let _ = self.media_events.send(crate::providers::CoreEvent::Call(
                json!({ "event": event, "identity": key }),
            ));
        }
    }
}

use std::sync::Arc;

/// How many recently used emoji the picker keeps (Android keeps a row or two).
const RECENT_EMOJI: usize = 32;

impl App {
    fn recent_emoji(&self) -> Result<Vec<String>, ApiError> {
        Ok(self
            .store
            .setting(keys::RECENT_EMOJI)?
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default())
    }

    /// `recentEmoji`: `{emoji: [text]}`, newest first.
    pub(crate) fn recent_emoji_json(&self) -> Result<Value, ApiError> {
        Ok(json!({ "emoji": self.recent_emoji()? }))
    }

    /// `useEmoji {emoji}`: put it first in the recently used.
    pub(crate) fn use_emoji(&mut self, emoji: &str) -> Result<Value, ApiError> {
        if emoji.is_empty() || emoji.len() > 64 {
            return Err(ApiError::Internal("not an emoji".into()));
        }
        let mut recent = self.recent_emoji()?;
        recent.retain(|e| e != emoji);
        recent.insert(0, emoji.to_owned());
        recent.truncate(RECENT_EMOJI);
        self.store.set_setting(
            keys::RECENT_EMOJI,
            &serde_json::to_string(&recent).map_err(|e| ApiError::Internal(e.to_string()))?,
        )?;
        Ok(json!({ "emoji": recent }))
    }
}

/// Live end-to-end test against the real chat server: restores a test ID from an ID backup
/// (env `THREESEA_TEST_BACKUP` / `THREESEA_TEST_BACKUP_PASSWORD`, never committed), sends a text
/// to ECHOECHO and waits for the echo. Run with
/// `cargo test --release -p threesea-core echo_roundtrip -- --ignored --nocapture`.
/// Don't run it while another device is connected with the same ID: the server allows only one.
#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::{app::StartConfig, providers::CoreEvent};

    #[test]
    #[ignore = "talks to the real Threema directory with a test ID"]
    fn address_book_match_roundtrip() {
        let (Ok(backup), Ok(password)) = (
            std::env::var("THREESEA_TEST_BACKUP"),
            std::env::var("THREESEA_TEST_BACKUP_PASSWORD"),
        ) else {
            eprintln!("THREESEA_TEST_BACKUP(_PASSWORD) not set, skipping");
            return;
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async move {
            let dir = tempfile::tempdir().unwrap();
            let mut app = App::new(
                &StartConfig {
                    data_dir: dir.path().to_owned(),
                    app_version: crate::VERSION.into(),
                    language: "en".into(),
                    country: "CH".into(),
                    model: "host-test".into(),
                    os_version: "test".into(),
                    pictures_dir: None,
                    calls: false,
                    log_level: None,
                    update_check: false,
                },
                &[9; 32],
            )
            .unwrap();
            app.store.set_setting(keys::LICENSE_KEY, "XXXXX-XXXXX").unwrap();
            let reply: Value = serde_json::from_str(
                &app.handle(
                    &json!({"id": 1, "cmd": "restoreIdentityBackup", "backup": backup, "password": password})
                        .to_string(),
                )
                .await,
            )
            .unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            let reply: Value = serde_json::from_str(
                &app.handle(
                    &json!({"id": 2, "cmd": "matchAddressBook", "entries": [
                        {"firstName": "Nobody", "emails": ["threesea-nobody@example.invalid"],
                         "phones": ["+352 621 000 000"]}
                    ]})
                    .to_string(),
                )
                .await,
            )
            .unwrap();
            // Token obtained and the match answered (nobody is linked to these).
            eprintln!("match reply: {reply}");
            assert_eq!(reply["ok"], true, "{reply}");
            assert_eq!(reply["result"]["matched"], 0);
        });
    }

    #[test]
    #[ignore = "connects to the real Threema chat server with a test ID"]
    fn echo_roundtrip() {
        let (Ok(backup), Ok(password)) = (
            std::env::var("THREESEA_TEST_BACKUP"),
            std::env::var("THREESEA_TEST_BACKUP_PASSWORD"),
        ) else {
            eprintln!("THREESEA_TEST_BACKUP(_PASSWORD) not set, skipping");
            return;
        };
        crate::runtime::init_logging(None);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&runtime, async move {
            let dir = tempfile::tempdir().unwrap();
            let mut app = App::new(
                &StartConfig {
                    data_dir: dir.path().to_owned(),
                    app_version: crate::VERSION.into(),
                    language: "en".into(),
                    country: "US".into(),
                    model: "host-test".into(),
                    os_version: "test".into(),
                    pictures_dir: None,
                    calls: false,
                    log_level: None,
                    update_check: false,
                },
                &[9; 32],
            )
            .unwrap();
            // The license is only checked locally before a restore.
            app.store
                .set_setting(keys::LICENSE_KEY, "XXXXX-XXXXX")
                .unwrap();
            let reply: Value = serde_json::from_str(
                &app.handle(
                    &json!({"id": 1, "cmd": "restoreIdentityBackup",
                    "backup": backup, "password": password})
                    .to_string(),
                )
                .await,
            )
            .unwrap();
            assert_eq!(reply["ok"], true, "{reply}");
            let reply: Value = serde_json::from_str(
                &app.handle(r#"{"id":2,"cmd":"addContact","identity":"ECHOECHO"}"#)
                    .await,
            )
            .unwrap();
            assert_eq!(reply["ok"], true, "{reply}");

            let mut events = app.ensure_connection().expect("connection starts");
            let text = format!("Threesea test {}", MessageId::random().0);
            let reply: Value = serde_json::from_str(
                &app.handle(
                    &json!({"id": 3, "cmd": "sendText", "identity": "ECHOECHO", "text": text})
                        .to_string(),
                )
                .await,
            )
            .unwrap();
            assert_eq!(reply["ok"], true, "{reply}");

            let mut sent = false;
            let mut echoed = false;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(45);
            while !(sent && echoed) {
                let event = tokio::time::timeout_at(deadline, events.recv())
                    .await
                    .expect("timed out waiting for the echo")
                    .expect("connection ended");
                match event {
                    FromConnection::State(state) => eprintln!("connection: {}", state.as_str()),
                    FromConnection::Event(event) => match *event {
                        CoreEvent::MessagesChanged(messages) => {
                            for m in messages {
                                eprintln!("outgoing {:016x} -> state {}", m.message_id, m.state);
                                sent |= m.state >= 1;
                            }
                        }
                        CoreEvent::MessageAdded { message, .. } => {
                            eprintln!("incoming from {}: {}", message.conversation, message.body);
                            echoed |=
                                message.conversation == "ECHOECHO" && message.body.contains(&text);
                        }
                        other => eprintln!("event: {other:?}"),
                    },
                    FromConnection::ServerMessage { message, .. } => eprintln!("server: {message}"),
                    FromConnection::WakeSettled => {}
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::StartConfig, contacts::test_contact, store::StoredIdentity};

    const PEER: &str = "ABCD1234";

    fn app(dir: &std::path::Path) -> App {
        let app = App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
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
                threema_id: "ECHOECHO".into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        app
    }

    /// A contact with `features`, and an unlocked app to talk to them.
    fn app_with_contact(dir: &std::path::Path, features: u64) -> App {
        let app = app(dir);
        let mut contact = test_contact(PEER);
        contact.feature_mask = FeatureMask(features);
        app.store.upsert_contact(&contact).unwrap();
        app
    }

    fn notes_group(app: &App) -> String {
        let group = crate::groups::Group {
            key: "g:ECHOECHO:0000000000000001".into(),
            identity: libthreema::common::GroupIdentity {
                creator_identity: "ECHOECHO".parse().unwrap(),
                group_id: 1,
            },
            name: "Notes".into(),
            members: Vec::new(),
            left: false,
            created_at: 0,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        group.key
    }

    /// As old as `age_ms`.
    fn make_old(app: &App, id: i64, age_ms: i64) {
        app.store
            .conn
            .execute(
                "UPDATE messages SET created_at = ?2 WHERE id = ?1",
                rusqlite::params![id, now_ms() - age_ms],
            )
            .unwrap();
    }

    fn sent_message(app: &App, conversation: &str, message_id: u64, body: &str) -> Message {
        let stored = app
            .store
            .insert_outgoing(conversation, message_id, "text", body)
            .unwrap();
        app.store
            .raise_outgoing_state(conversation, &[message_id], OutgoingState::Sent)
            .unwrap();
        stored
    }

    /// The edit goes out as the bare text, as Android sends it and shows it; the quote stays with
    /// the message on either side.
    #[test]
    fn an_edit_of_a_quoting_message_sends_no_quote_marker() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), FeatureMask::EDIT_MESSAGE_SUPPORT);
        let quoting = crate::messages::with_quote(Some(5), "Yes");
        let stored = sent_message(&app, PEER, 7, &quoting);
        app.edit_message(PEER, &format!("{:x}", 7), "No").unwrap();
        let outbox = app.store.outbox().unwrap();
        assert_eq!(outbox[0].kind, reactions::OUTBOX_EDIT);
        assert_eq!(outbox[0].body["text"], "No");
        let edited = app.store.message(stored.id).unwrap().unwrap();
        assert_eq!(edited.body, crate::messages::with_quote(Some(5), "No"));
    }

    /// In a notes group nobody can lack a feature and nothing gets too old: a heart, an edit of a
    /// note from long ago and its deletion all work, and each goes into the outbox for the linked
    /// devices although there are no receivers.
    #[test]
    fn a_notes_group_supports_everything_at_any_age() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let key = notes_group(&app);
        let note = sent_message(&app, &key, 7, "old note");
        make_old(&app, note.id, 2 * EDIT_DELETE_MAX_AGE_MS);
        let id = format!("{:x}", 7);

        app.react(&key, &id, "\u{2764}\u{fe0f}", true).unwrap();
        app.edit_message(&key, &id, "new note").unwrap();
        let edited = app.store.message(note.id).unwrap().unwrap();
        assert_eq!(edited.body, "new note");
        assert!(edited.edited_at.is_some());

        app.delete_message(&key, &id, true, true).unwrap();
        assert_eq!(app.store.message(note.id).unwrap().unwrap().kind, "deleted");

        let outbox = app.store.outbox().unwrap();
        let kinds: Vec<&str> = outbox.iter().map(|entry| entry.kind.as_str()).collect();
        assert_eq!(
            kinds,
            [
                reactions::OUTBOX_REACTION,
                reactions::OUTBOX_EDIT,
                reactions::OUTBOX_DELETE
            ]
        );
        for entry in &outbox {
            assert!(
                app.store
                    .pending_receivers(&key, entry.message_id)
                    .unwrap()
                    .is_empty()
            );
        }
    }

    /// Only the notes group goes without the limit: an ordinary group's old message stays too old,
    /// and a group we left supports nothing.
    #[test]
    fn other_groups_keep_their_limits() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), FeatureMask::EDIT_MESSAGE_SUPPORT);
        let key = notes_group(&app);
        let mut group = app.store.group(&key).unwrap().unwrap();
        group.members = vec![PEER.to_owned()];
        app.store.upsert_group(&group).unwrap();
        let old = sent_message(&app, &key, 7, "old");
        make_old(&app, old.id, 2 * EDIT_DELETE_MAX_AGE_MS);
        assert!(matches!(
            app.edit_message(&key, &format!("{:x}", 7), "new"),
            Err(ApiError::MessageTooOld)
        ));

        group.members = Vec::new();
        group.left = true;
        app.store.upsert_group(&group).unwrap();
        assert!(!group.is_notes());
        assert!(matches!(
            Conversation::Group(group).supports(&app.store, FeatureMask::EDIT_MESSAGE_SUPPORT),
            Ok(false)
        ));
    }

    /// Before its send has begun a message is simply another message; nobody hears of it.
    #[test]
    fn a_message_not_written_out_is_just_changed_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), 0);
        let id = format!("{:x}", 7);
        let stored = app.store.insert_outgoing(PEER, 7, "text", "helo").unwrap();
        app.edit_message(PEER, &id, "hello").unwrap();
        let edited = app.store.message(stored.id).unwrap().unwrap();
        assert_eq!((edited.body.as_str(), edited.edited_at), ("hello", None));
        assert!(app.store.outbox().unwrap().is_empty());
        app.delete_message(PEER, &id, true, true).unwrap();
        assert!(app.store.message(stored.id).unwrap().is_none());
        assert!(app.store.outbox().unwrap().is_empty());
    }

    /// Once its send has begun someone may have it: a resend under the same ID would reach only
    /// those who don't, so everyone who can take an edit gets one. It stays in the queue with the
    /// new text for the others.
    #[test]
    fn a_message_written_out_is_edited_for_those_who_have_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), FeatureMask::EDIT_MESSAGE_SUPPORT);
        let key = notes_group(&app);
        let mut group = app.store.group(&key).unwrap().unwrap();
        group.members = vec![PEER.to_owned()];
        app.store.upsert_group(&group).unwrap();
        let id = format!("{:x}", 7);

        for conversation in [PEER, key.as_str()] {
            let stored = app
                .store
                .insert_outgoing(conversation, 7, "text", "helo")
                .unwrap();
            app.store.set_written_out(stored.id).unwrap();
            app.edit_message(conversation, &id, "hello").unwrap();
            let edited = app.store.message(stored.id).unwrap().unwrap();
            assert_eq!(edited.state, OutgoingState::Pending as i64);
            assert_eq!(edited.body, "hello");
            assert!(edited.edited_at.is_some());
            assert!(
                app.store
                    .pending_outgoing()
                    .unwrap()
                    .iter()
                    .any(|m| m.id == stored.id)
            );

            let outbox = app.store.outbox().unwrap();
            let edit = outbox
                .iter()
                .find(|e| e.conversation == conversation)
                .unwrap();
            assert_eq!(edit.kind, reactions::OUTBOX_EDIT);
            assert_eq!(edit.body["text"], "hello");
        }
        let group_edit = app.store.outbox().unwrap();
        let group_edit = group_edit.iter().find(|e| e.conversation == key).unwrap();
        assert_eq!(
            app.store
                .pending_receivers(&key, group_edit.message_id)
                .unwrap(),
            vec![PEER.to_owned()]
        );
    }

    /// A contact who can't take an edit keeps what they may have: changing only our copy is what
    /// this is there to prevent.
    #[test]
    fn a_message_written_out_stays_for_a_contact_without_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), 0);
        let stored = app.store.insert_outgoing(PEER, 7, "text", "helo").unwrap();
        app.store.set_written_out(stored.id).unwrap();
        assert!(matches!(
            app.edit_message(PEER, &format!("{:x}", 7), "hello"),
            Err(ApiError::NotSupportedByContact)
        ));
        assert_eq!(app.store.message(stored.id).unwrap().unwrap().body, "helo");
    }

    /// Deleting for everyone goes the same way, and what is left has nothing more to send.
    #[test]
    fn a_message_written_out_is_deleted_for_everyone() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with_contact(dir.path(), FeatureMask::DELETE_MESSAGE_SUPPORT);
        let stored = app.store.insert_outgoing(PEER, 7, "text", "helo").unwrap();
        app.store.set_written_out(stored.id).unwrap();
        app.delete_message(PEER, &format!("{:x}", 7), true, true)
            .unwrap();
        let deleted = app.store.message(stored.id).unwrap().unwrap();
        assert_eq!(deleted.kind, "deleted");
        assert_eq!(deleted.state, OutgoingState::Sent as i64);
        assert_eq!(
            app.store.outbox().unwrap()[0].kind,
            reactions::OUTBOX_DELETE
        );
        assert!(app.store.pending_outgoing().unwrap().is_empty());
    }
}
