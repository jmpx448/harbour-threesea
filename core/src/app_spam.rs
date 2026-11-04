// SPDX-License-Identifier: AGPL-3.0-only
//! Reporting spam: a chat that may be spam offers a report to Threema's directory, and to block and
//! delete along with it (`docs/protocol-notes.md`). The rules follow iOS 7.5.2's written steps
//! (`SpamDetector`, `GroupSpamDetector`) with Android's further exclusions
//! (`ComposeMessageFragment.isPossibleSpamContact`): named, blocked and predefined contacts.

use libthreema::{
    common::keys::ClientKey,
    model::contact::Contact,
    protobuf::d2d_sync::contact::{AcquaintanceLevel, VerificationLevel},
};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    app_messages::Conversation,
    app_multidevice::SyncChange,
    directory,
};

/// A chat's decision: `dismissed` or `reported`, after which it never offers a report again.
const SETTING_PREFIX: &str = "spamReporting:";
/// The first message came this long after the contact was created at most: an old contact that
/// writes for the first time is somebody the user knows.
const FIRST_MESSAGE_WITHIN_MS: i64 = 7 * 24 * 60 * 60 * 1000;

impl App {
    /// A contact the user has no relation to: unverified, unnamed, not blocked, not predefined.
    /// IDs of gateways (`*…`) can't be reported: the directory takes alphanumeric ones only.
    fn unknown_contact(&self, contact: &Contact) -> Result<bool, ApiError> {
        let id = contact.identity.to_string();
        let named = |name: &Option<String>| name.as_deref().is_some_and(|n| !n.is_empty());
        Ok(id.chars().all(char::is_alphanumeric)
            && contact.verification_level == VerificationLevel::Unverified
            && !named(&contact.first_name)
            && !named(&contact.last_name)
            && !self
                .config
                .predefined_contacts
                .contains_key(&contact.identity)
            && !self.app_settings()?.blocked_contacts.contains(&id))
    }

    /// Only messages from them so far, the first soon after the contact came to be.
    fn messages_suggest_spam(&self, key: &str, contact: &Contact) -> Result<bool, ApiError> {
        if self.store.has_outgoing_content(key)? {
            return Ok(false);
        }
        let created = i64::try_from(contact.created_at).unwrap_or(i64::MAX);
        Ok(self
            .store
            .earliest_incoming_content(key)?
            .is_some_and(|first| first.saturating_sub(created) <= FIRST_MESSAGE_WITHIN_MS))
    }

    /// Who to report for this chat, if it may be spam: the contact, or the group's creator.
    fn spam_suspect(
        &self,
        key: &str,
        conversation: &Conversation,
    ) -> Result<Option<Contact>, ApiError> {
        if self
            .store
            .setting(&format!("{SETTING_PREFIX}{key}"))?
            .is_some()
        {
            return Ok(None);
        }
        let suspect = match conversation {
            Conversation::Contact(contact) => (contact.acquaintance_level
                != AcquaintanceLevel::GroupOrDeleted)
                .then(|| contact.clone()),
            Conversation::Group(group) => {
                let creator = group.identity.creator_identity.to_string();
                if group.left || creator == self.user()? {
                    None
                } else {
                    self.store.contact(&creator)?
                }
            }
            Conversation::List(_) => None,
        };
        let Some(suspect) = suspect else {
            return Ok(None);
        };
        Ok(
            (self.unknown_contact(&suspect)? && self.messages_suggest_spam(key, &suspect)?)
                .then_some(suspect),
        )
    }

    /// `spamPotential`: whether this chat offers a report.
    pub(crate) fn spam_potential(&self, identity: &str) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        Ok(json!({ "potential": self.spam_suspect(&key, &conversation)?.is_some() }))
    }

    /// `dismissSpam`: not spam, don't offer it again.
    pub(crate) fn dismiss_spam(&self, identity: &str) -> Result<Value, ApiError> {
        let (key, _) = self.readable_conversation(identity)?;
        self.store
            .set_setting(&format!("{SETTING_PREFIX}{key}"), "dismissed")?;
        Ok(json!({}))
    }

    /// `reportSpam`: report to Threema's directory. Blocking and deleting follow only once the
    /// report is through, so that a failed one can be tried again from the same chat (like Android).
    pub(crate) async fn report_spam(
        &mut self,
        identity: &str,
        block: bool,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        let suspect = self
            .spam_suspect(&key, &conversation)?
            .ok_or_else(|| ApiError::Internal("nothing to report in this chat".into()))?;
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let client_key = ClientKey::from(*own.client_key);
        let suspect_id = suspect.identity.to_string();
        let nickname = suspect.nickname.as_deref();
        match &conversation {
            Conversation::Group(group) => {
                // `members` is everyone but us.
                directory::report_group(
                    &self.https,
                    &client_key,
                    &own.threema_id,
                    &suspect_id,
                    nickname,
                    group.identity.group_id,
                    group.members.len() + 1,
                )
                .await?;
            }
            _ => {
                directory::report_identity(
                    &self.https,
                    &client_key,
                    &own.threema_id,
                    &suspect_id,
                    nickname,
                )
                .await?;
            }
        }
        self.after_report(&key, &conversation, &suspect_id, block)
            .await
    }

    /// The report is through: remember it, and block and delete if that was asked for.
    async fn after_report(
        &mut self,
        key: &str,
        conversation: &Conversation,
        suspect: &str,
        block: bool,
    ) -> Result<Value, ApiError> {
        self.store
            .set_setting(&format!("{SETTING_PREFIX}{key}"), "reported")?;
        tracing::info!(block, "spam reported");
        if block {
            self.set_blocked(suspect, true)?;
            self.set_sync_excluded(suspect, true)?;
            self.reflect_change(SyncChange::Settings, &json!({})).await;
            if matches!(conversation, Conversation::Group(_)) {
                let left = self.leave_group(key)?;
                let change = SyncChange::Group {
                    previous_members: None,
                    key: Some(key.to_owned()),
                    created: false,
                };
                self.reflect_change(change, &left).await;
            }
            self.delete_conversation(key)?;
        }
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use libthreema::common::GroupIdentity;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{
        app::StartConfig,
        contacts::test_contact,
        groups::Group,
        store::{StoredIdentity, keys},
    };

    const USER: &str = "ABCD1234";
    const SUSPECT: &str = "EFGH5678";
    const CREATED: i64 = 1_000_000;
    const DAY: i64 = 24 * 60 * 60 * 1000;

    fn app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        let app = App::new(
            &StartConfig {
                data_dir: dir.path().to_owned(),
                app_version: "0.1.0".into(),
                language: "en".into(),
                country: "US".into(),
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
            .set_setting(keys::LICENSE_KEY, "ABCDE-12345")
            .unwrap();
        app.store
            .set_identity(&StoredIdentity {
                threema_id: USER.into(),
                client_key: Zeroizing::new([7; 32]),
                server_group: "30".into(),
            })
            .unwrap();
        (dir, app)
    }

    /// The contact as a stranger who wrote one message right after appearing, `change` applied.
    fn suspect(app: &App, change: impl FnOnce(&mut libthreema::model::contact::ContactInit)) {
        let mut init = test_contact(SUSPECT);
        init.created_at = CREATED as u64;
        init.verification_level = VerificationLevel::Unverified;
        init.acquaintance_level = AcquaintanceLevel::Direct;
        change(&mut init);
        app.store.upsert_contact(&init).unwrap();
    }

    fn potential(app: &App, identity: &str) -> bool {
        app.spam_potential(identity).unwrap()["potential"] == true
    }

    fn message(app: &App, conversation: &str, at: i64) {
        app.store
            .insert_incoming(conversation, at as u64, "text", "buy now", at)
            .unwrap();
    }

    #[test]
    fn a_strangers_first_messages_may_be_spam() {
        let (_dir, app) = app();
        suspect(&app, |_| {});
        assert!(!potential(&app, SUSPECT), "nothing written yet");
        message(&app, SUSPECT, CREATED + DAY);
        assert!(potential(&app, SUSPECT));
    }

    #[test]
    fn a_status_line_is_not_a_message() {
        let (_dir, app) = app();
        suspect(&app, |_| {});
        app.store.insert_status(SUSPECT, "group|created").unwrap();
        assert!(!potential(&app, SUSPECT));
    }

    #[test]
    fn who_is_not_a_stranger() {
        let cases: [fn(&mut libthreema::model::contact::ContactInit); 3] = [
            |c| c.verification_level = VerificationLevel::ServerVerified,
            |c| c.first_name = Some("Ann".into()),
            |c| c.acquaintance_level = AcquaintanceLevel::GroupOrDeleted,
        ];
        for change in cases {
            let (_dir, app) = app();
            suspect(&app, change);
            message(&app, SUSPECT, CREATED + DAY);
            assert!(!potential(&app, SUSPECT));
        }
        // Blocked.
        let (_dir, mut app) = app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + DAY);
        app.set_blocked(SUSPECT, true).unwrap();
        assert!(!potential(&app, SUSPECT));
    }

    #[test]
    fn a_gateway_cannot_be_reported() {
        let (_dir, app) = app();
        let mut init = test_contact("*GATEWAY");
        init.created_at = CREATED as u64;
        init.verification_level = VerificationLevel::Unverified;
        init.acquaintance_level = AcquaintanceLevel::Direct;
        app.store.upsert_contact(&init).unwrap();
        message(&app, "*GATEWAY", CREATED + DAY);
        assert!(!potential(&app, "*GATEWAY"));
    }

    #[test]
    fn a_reply_or_an_old_contact_rules_it_out() {
        let (_dir, app) = app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + DAY);
        app.store
            .insert_outgoing(SUSPECT, 1, "text", "who are you?")
            .unwrap();
        assert!(!potential(&app, SUSPECT));

        let (_dir, app) = self::app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + 7 * DAY + 1);
        assert!(!potential(&app, SUSPECT));
    }

    #[test]
    fn a_dismissed_chat_stays_quiet() {
        let (_dir, app) = app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + DAY);
        app.dismiss_spam(SUSPECT).unwrap();
        assert!(!potential(&app, SUSPECT));
    }

    fn group(creator: &str, left: bool) -> Group {
        let group_id = 0x1234_u64;
        Group {
            key: format!("g:{creator}:{group_id:016x}"),
            identity: GroupIdentity {
                creator_identity: creator.parse().unwrap(),
                group_id,
            },
            name: "Offers".into(),
            members: vec![SUSPECT.into()],
            left,
            created_at: CREATED,
            picture: None,
            picture_version: 0,
        }
    }

    #[test]
    fn a_strangers_group_may_be_spam() {
        let (_dir, app) = app();
        suspect(&app, |c| {
            // A group's creator is usually only known from the group.
            c.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        });
        let theirs = group(SUSPECT, false);
        app.store.upsert_group(&theirs).unwrap();
        message(&app, &theirs.key, CREATED + DAY);
        assert!(potential(&app, &theirs.key));

        // Not one we left.
        let left = Group {
            left: true,
            ..theirs
        };
        app.store.upsert_group(&left).unwrap();
        assert!(!potential(&app, &left.key));

        // Not our own.
        let ours = group(USER, false);
        app.store.upsert_group(&ours).unwrap();
        message(&app, &ours.key, CREATED + DAY);
        assert!(!potential(&app, &ours.key));
    }

    // These never call `report_spam` with a suspect: a test must not send a report to Threema's
    // real directory. Its network step is the requests' tests in `directory.rs` and the phase 1
    // live test; what follows it is tested here.

    #[tokio::test(flavor = "current_thread")]
    async fn after_a_report_the_chat_stays_unless_asked_otherwise() {
        let (_dir, mut app) = app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + DAY);
        let (key, conversation) = app.readable_conversation(SUSPECT).unwrap();
        app.after_report(&key, &conversation, SUSPECT, false)
            .await
            .unwrap();
        assert!(!potential(&app, SUSPECT), "reported once");
        assert!(
            app.store
                .earliest_incoming_content(SUSPECT)
                .unwrap()
                .is_some()
        );
        assert!(!blocked(&app, SUSPECT));
    }

    fn blocked(app: &App, id: &str) -> bool {
        app.app_settings()
            .unwrap()
            .blocked_contacts
            .iter()
            .any(|other| other == id)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_and_delete_a_contacts_chat() {
        let (_dir, mut app) = app();
        suspect(&app, |_| {});
        message(&app, SUSPECT, CREATED + DAY);
        let (key, conversation) = app.readable_conversation(SUSPECT).unwrap();
        app.after_report(&key, &conversation, SUSPECT, true)
            .await
            .unwrap();
        assert!(blocked(&app, SUSPECT));
        assert_eq!(app.store.earliest_incoming_content(SUSPECT).unwrap(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn block_and_delete_a_group_blocks_its_creator_and_removes_it() {
        tokio::task::LocalSet::new()
            .run_until(async {
                let (_dir, mut app) = app();
                suspect(&app, |_| {});
                let theirs = group(SUSPECT, false);
                app.store.upsert_group(&theirs).unwrap();
                message(&app, &theirs.key, CREATED + DAY);
                let (key, conversation) = app.readable_conversation(&theirs.key).unwrap();
                app.after_report(&key, &conversation, SUSPECT, true)
                    .await
                    .unwrap();
                assert!(blocked(&app, SUSPECT));
                assert!(app.store.group(&theirs.key).unwrap().is_none());
                assert_eq!(app.store.earliest_incoming_content(&key).unwrap(), None);
                // They still hear that we left, or they answer with the group again.
                let queued = app.store.outbox().unwrap();
                assert_eq!(queued.len(), 1);
                assert_eq!(queued[0].kind, crate::groups::OUTBOX_GROUP_LEAVE);
                assert_eq!(
                    app.store
                        .pending_receivers(&theirs.key, queued[0].message_id)
                        .unwrap(),
                    theirs.members
                );
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_a_suspect_can_be_reported() {
        let (_dir, mut app) = app();
        suspect(&app, |c| c.first_name = Some("Ann".into()));
        message(&app, SUSPECT, CREATED + DAY);
        assert!(app.report_spam(SUSPECT, false).await.is_err());
    }
}
