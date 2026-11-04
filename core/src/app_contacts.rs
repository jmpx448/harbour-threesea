// SPDX-License-Identifier: AGPL-3.0-only
//! Contact and profile commands.

use libthreema::{common::keys::ClientKey, protobuf::d2d_sync::contact as pc};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    connection::ToConnection,
    contacts::{self, Lookup},
    directory::DirectoryError,
    safe::backup,
    store::keys,
};

fn clean(name: Option<String>) -> Option<String> {
    name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty())
}

/// Settings the UI may change (`setSetting`), as Safe `settings` field names.
const UI_SETTINGS: &[&str] = &[
    "threemaCalls",
    "relayThreemaCalls",
    "readReceipts",
    "sendTyping",
    "blockUnknown",
    "incognitoKeyboard",
    "disableScreenshots",
];

/// What this client supports, announced to the directory, like Android's
/// `UserServiceImpl.getFeatureMask` (text, location, receipts and typing need no flag). Calls:
/// `app_calls::CALLS_ANNOUNCED`; video and group calls follow when they exist.
pub const FEATURE_MASK: u64 = libthreema::common::FeatureMask::VOICE_MESSAGE_SUPPORT
    | libthreema::common::FeatureMask::FORWARD_SECURITY_SUPPORT
    | libthreema::common::FeatureMask::REACTION_SUPPORT
    | libthreema::common::FeatureMask::EDIT_MESSAGE_SUPPORT
    | libthreema::common::FeatureMask::DELETE_MESSAGE_SUPPORT
    | libthreema::common::FeatureMask::FILE_MESSAGE_SUPPORT
    | libthreema::common::FeatureMask::GROUP_SUPPORT
    | libthreema::common::FeatureMask::POLL_SUPPORT;

impl App {
    /// Publish [`FEATURE_MASK`] if the directory doesn't have it yet; without forward security
    /// while multi-device is active (SE-199, like Android's `updateFeatureMask(isFsAllowed)`).
    pub(crate) async fn publish_feature_mask(&mut self) -> Result<(), ApiError> {
        let Some(identity) = self.store.identity()? else {
            return Ok(());
        };
        let fs_off =
            self.store.multi_device()?.is_some() || self.store.forward_security_disabled()?;
        let feature_mask = if fs_off {
            FEATURE_MASK & !libthreema::common::FeatureMask::FORWARD_SECURITY_SUPPORT
        } else {
            FEATURE_MASK
        };
        // Like Android, also with calls turned off: callers then get "disabled" back.
        let feature_mask = if crate::app_calls::CALLS_ANNOUNCED && self.calls_supported {
            feature_mask | libthreema::common::FeatureMask::O2O_AUDIO_CALL_SUPPORT
        } else {
            feature_mask
        };
        let feature_mask = if crate::app_group_calls::GROUP_CALLS_ANNOUNCED && self.calls_supported
        {
            feature_mask | libthreema::common::FeatureMask::GROUP_CALL_SUPPORT
        } else {
            feature_mask
        };
        let feature_mask = if crate::app_calls::VIDEO_CALLS_ANNOUNCED && self.calls_supported {
            feature_mask | libthreema::common::FeatureMask::O2O_VIDEO_CALL_SUPPORT
        } else {
            feature_mask
        };
        let mask = feature_mask.to_string();
        if self.store.setting(keys::PUBLISHED_FEATURE_MASK)?.as_deref() == Some(mask.as_str()) {
            return Ok(());
        }
        // Still different when maintenance comes by after the rest.
        if self.store.directory_resting() {
            return Ok(());
        }
        let client_key = ClientKey::from(*identity.client_key);
        crate::directory::set_feature_mask(
            &self.https,
            &identity.threema_id,
            &client_key,
            feature_mask,
        )
        .await
        .map_err(|error| self.directory_error(error))?;
        self.store
            .set_setting(keys::PUBLISHED_FEATURE_MASK, &mask)?;
        tracing::info!(mask = feature_mask, "feature mask published");
        Ok(())
    }

    /// Current app settings (defaults like Android's if never set).
    pub(crate) fn app_settings(&self) -> Result<backup::Settings, ApiError> {
        Ok(self
            .store
            .setting(keys::APP_SETTINGS)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default())
    }

    pub(crate) fn save_app_settings(&self, settings: &backup::Settings) -> Result<(), ApiError> {
        let json =
            serde_json::to_string(settings).map_err(|e| ApiError::Internal(e.to_string()))?;
        self.store.set_setting(keys::APP_SETTINGS, &json)?;
        Ok(())
    }

    /// The app settings plus the call video settings (kept apart: not in Threema Safe).
    pub(crate) fn settings_json(&self) -> Result<Value, ApiError> {
        let mut json = serde_json::to_value(self.app_settings()?)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        json["videoCalls"] = json!(self.video_calls_setting()?);
        json["videoQuality"] = json!(self.video_quality_setting()?);
        json["groupCalls"] = json!(self.group_calls_setting()?);
        json["receiveProfilePictures"] = json!(self.store.receive_pictures()?);
        Ok(json)
    }

    pub(crate) fn video_calls_setting(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::VIDEO_CALLS)?.as_deref() != Some("0"))
    }

    /// Group calls on (Android's group call policy; the feature mask keeps the bit either way,
    /// as on Android, so the setting only decides what this device does).
    pub(crate) fn group_calls_setting(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::GROUP_CALLS)?.as_deref() != Some("0"))
    }

    pub(crate) fn video_quality_setting(&self) -> Result<String, ApiError> {
        Ok(self
            .store
            .setting(keys::VIDEO_QUALITY)?
            .unwrap_or_else(|| "auto".to_owned()))
    }

    /// `setVideoQuality`: `auto`, `low` or `max`.
    pub(crate) fn set_video_quality(&mut self, quality: &str) -> Result<Value, ApiError> {
        if !["auto", "low", "max"].contains(&quality) {
            return Err(ApiError::Internal(format!(
                "unknown video quality {quality}"
            )));
        }
        self.store.set_setting(keys::VIDEO_QUALITY, quality)?;
        self.settings_json()
    }

    pub(crate) fn set_app_setting(&mut self, name: &str, value: bool) -> Result<Value, ApiError> {
        if name == "videoCalls" {
            self.store
                .set_setting(keys::VIDEO_CALLS, if value { "1" } else { "0" })?;
            return self.settings_json();
        }
        if name == "receiveProfilePictures" {
            self.store.set_setting(
                crate::profile_pictures::SETTING_RECEIVE,
                if value { "1" } else { "0" },
            )?;
            return self.settings_json();
        }
        if name == "groupCalls" {
            self.store
                .set_setting(keys::GROUP_CALLS, if value { "1" } else { "0" })?;
            if !value {
                // Leave a call we are in; nothing of this device stays in it.
                self.group_call_leave();
            }
            return self.settings_json();
        }
        if !UI_SETTINGS.contains(&name) {
            return Err(ApiError::Internal(format!("unknown setting {name}")));
        }
        let mut settings = serde_json::to_value(self.app_settings()?)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        settings[name] = json!(value);
        let settings: backup::Settings =
            serde_json::from_value(settings).map_err(|e| ApiError::Internal(e.to_string()))?;
        self.save_app_settings(&settings)?;
        self.settings_json()
    }

    /// Remove the ID and all data from this device (Android's `ResetAppTask`, without
    /// revocation). The Threema Safe backup on the server stays and expires by itself.
    ///
    /// The core's part of one ordered wipe (ADR 0018): what runs stops, the device group is
    /// left while a connection can still do that, the store empties in one transaction, then
    /// media and logs go and nothing of the old identity stays in memory. On return the bridge
    /// stops the core and replaces the database and its key.
    ///
    /// There is no way back from the mark on: a store that refuses (a full disk has no room for
    /// the write-ahead log) doesn't stop it, because replacing the file needs no space. Only a
    /// refused PIN, before this is called, leaves the ID in place.
    ///
    /// `{linkedDevicesRemain}`: linked devices could not be removed and keep the ID.
    pub(crate) async fn delete_all_data(&mut self) -> Result<Value, ApiError> {
        self.stop_backups().await;
        // Both while the connection can still tell the others.
        self.call_end_for_removal();
        self.group_call_forget();
        // Paired browsers belong to this identity: none may carry over to the next one.
        self.web_stop_all(crate::app_web::StopReason::Delete);
        // The call's end goes out now: once the contact is gone it can't be encoded any more.
        self.connection_flushed().await;
        let linked_devices_remain = self.drop_linked_devices_for_removal().await;
        self.stop_connection();
        // A kill from here on: the next start finishes the job, whatever the bridge got to hear.
        if let Err(error) = crate::runtime::mark_removal(&self.data_dir, linked_devices_remain) {
            tracing::warn!(%error, "the removal could not be marked");
        }
        self.identity_removed = true;
        if let Err(error) = self.store.wipe() {
            tracing::error!(%error, "the store could not be emptied, its file goes next");
        }
        self.media.remove_all();
        // The next identity starts locked; its private marks wait for its own PIN.
        self.unlocked = false;
        self.write_pass = None;
        self.update_media_gate();
        // Credentials and links belong to the deleted identity, and the empty store switches
        // off what was read from it into process-wide state.
        self.turn_servers = None;
        self.sfu.take();
        self.pending_link = None;
        self.last_group_check = None;
        self.last_import_attempt = None;
        self.apply_multi_device();
        contacts::LAST_NAME_FIRST.store(false, std::sync::atomic::Ordering::Relaxed);
        tracing::info!("all local data deleted");
        // Last: anything logged after this would be the first line of the next identity's log.
        crate::runtime::log_clear(&self.data_dir);
        Ok(json!({ "linkedDevicesRemain": linked_devices_remain }))
    }

    pub(crate) fn contacts_json(&self) -> Result<Value, ApiError> {
        let flag = |key| -> Result<bool, ApiError> {
            Ok(self.store.setting(key)?.as_deref() == Some("1"))
        };
        let (by_last_name, hide_inactive) =
            (flag(keys::SORT_BY_LAST_NAME)?, flag(keys::HIDE_INACTIVE)?);
        let mut list: Vec<Value> = self
            .store
            .contacts()?
            .iter()
            // Contacts only known from groups/deleted ones are not shown in the contact list.
            .filter(|c| c.acquaintance_level == pc::AcquaintanceLevel::Direct)
            .filter(|c| !hide_inactive || c.activity_state != pc::ActivityState::Inactive)
            .map(|c| {
                let mut value = self.contact_value(c);
                // Nicknames ("~nick") sort under their letter; the UI takes the section from it.
                value["sortKey"] = json!(contacts::sort_key(c, by_last_name));
                value
            })
            .collect();
        list.sort_by(|a, b| a["sortKey"].as_str().cmp(&b["sortKey"].as_str()));
        Ok(json!({ "contacts": list }))
    }

    /// Settings → Appearance: `{lastNameFirst, sortByLastName, showInactive}`.
    pub(crate) fn display_settings_json(&self) -> Result<Value, ApiError> {
        let flag = |key| -> Result<bool, ApiError> {
            Ok(self.store.setting(key)?.as_deref() == Some("1"))
        };
        Ok(json!({
            "lastNameFirst": flag(keys::LAST_NAME_FIRST)?,
            "sortByLastName": flag(keys::SORT_BY_LAST_NAME)?,
            "showInactive": !flag(keys::HIDE_INACTIVE)?,
        }))
    }

    pub(crate) fn set_display_settings(
        &mut self,
        last_name_first: Option<bool>,
        sort_by_last_name: Option<bool>,
        show_inactive: Option<bool>,
    ) -> Result<Value, ApiError> {
        let flag = |on: bool| if on { "1" } else { "0" };
        if let Some(on) = last_name_first {
            self.store.set_setting(keys::LAST_NAME_FIRST, flag(on))?;
        }
        if let Some(on) = sort_by_last_name {
            self.store.set_setting(keys::SORT_BY_LAST_NAME, flag(on))?;
        }
        if let Some(on) = show_inactive {
            self.store.set_setting(keys::HIDE_INACTIVE, flag(!on))?;
        }
        self.apply_display_settings()?;
        self.display_settings_json()
    }

    /// Load the name order into [`contacts::LAST_NAME_FIRST`] (start, change).
    pub(crate) fn apply_display_settings(&self) -> Result<(), ApiError> {
        let on = self.store.setting(keys::LAST_NAME_FIRST)?.as_deref() == Some("1");
        contacts::LAST_NAME_FIRST.store(on, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub(crate) fn contact_json(&self, identity: &str) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let contact = self
            .store
            .contact(&id.to_string())?
            .ok_or(ApiError::ContactNotFound)?;
        Ok(self.contact_value(&contact))
    }

    /// Contact JSON with its picture version (0: none; `image://threesea/contact/<ID>?<v>`).
    pub(crate) fn contact_value(&self, contact: &libthreema::model::contact::Contact) -> Value {
        let mut value = contacts::to_json(contact);
        value["pictureVersion"] = json!(
            self.store
                .contact_picture_version(&contact.identity.to_string())
                .unwrap_or(0)
        );
        // Only while sharing with selected contacts (null otherwise): on the allow list?
        value["pictureRecipient"] = match self.store.picture_share() {
            Ok(crate::profile_pictures::Share::Selected) => json!(
                self.store
                    .is_picture_recipient(&contact.identity.to_string())
                    .unwrap_or(false)
            ),
            _ => Value::Null,
        };
        value["notifications"] = self
            .store
            .chat_notifications_json(&contact.identity.to_string());
        value["pinned"] = json!(
            self.store
                .pinned_chats()
                .is_ok_and(|pinned| pinned.contains(&contact.identity.to_string()))
        );
        value["blocked"] = json!(self.app_settings().is_ok_and(|settings| {
            settings
                .blocked_contacts
                .contains(&contact.identity.to_string())
        }));
        value
    }

    /// The block list is part of the app settings (as on Android, and in Threema Safe).
    pub(crate) fn set_blocked(&mut self, identity: &str, blocked: bool) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity)
            .ok_or(ApiError::IdentityFormat)?
            .to_string();
        let mut settings = self.app_settings()?;
        settings.blocked_contacts.retain(|blocked| *blocked != id);
        if blocked {
            settings.blocked_contacts.push(id.clone());
        }
        self.save_app_settings(&settings)?;
        tracing::info!(blocked, "block list changed");
        Ok(match self.store.contact(&id)? {
            Some(contact) => self.contact_value(&contact),
            None => json!({}),
        })
    }

    /// The exclusion list is part of the app settings too (`syncExcludedIds`, also in Safe).
    pub(crate) fn set_sync_excluded(
        &mut self,
        identity: &str,
        excluded: bool,
    ) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity)
            .ok_or(ApiError::IdentityFormat)?
            .to_string();
        let mut settings = self.app_settings()?;
        settings.sync_excluded_ids.retain(|other| *other != id);
        if excluded {
            settings.sync_excluded_ids.push(id);
        }
        self.save_app_settings(&settings)?;
        self.sync_excluded_json()
    }

    pub(crate) fn sync_excluded_json(&self) -> Result<Value, ApiError> {
        Ok(json!({ "excluded": self.id_list_json(&self.app_settings()?.sync_excluded_ids) }))
    }

    pub(crate) fn blocked_contacts_json(&self) -> Result<Value, ApiError> {
        Ok(json!({ "blocked": self.id_list_json(&self.app_settings()?.blocked_contacts) }))
    }

    /// `[{identity, displayName}]` for a list of IDs.
    fn id_list_json(&self, ids: &[String]) -> Vec<Value> {
        ids.iter()
            .map(|id| {
                let name = self
                    .store
                    .contact(id)
                    .ok()
                    .flatten()
                    .map_or_else(|| id.clone(), |c| contacts::display_name(&c));
                json!({ "identity": id, "displayName": name })
            })
            .collect()
    }

    pub(crate) fn set_chat_notifications(
        &mut self,
        identity: &str,
        policy: Option<&str>,
        until: Option<i64>,
    ) -> Result<Value, ApiError> {
        let key = self.conversation(identity)?.0;
        let policy = match policy {
            Some(policy) => Some(
                crate::chat_notifications::Policy::parse(policy)
                    .ok_or_else(|| ApiError::Internal("unknown notification policy".into()))?,
            ),
            None => None,
        };
        self.store
            .set_chat_notifications(&key, policy.map(|policy| (policy, until)))?;
        Ok(self.store.chat_notifications_json(&key))
    }

    /// Hidden private chats stay out, as their chats do (ADR 0009).
    pub(crate) fn chat_tones(&self) -> Result<Value, ApiError> {
        let mut tones = self.store.chat_tones()?;
        tones.retain(|conversation, _| self.check_readable(conversation).is_ok());
        Ok(json!({ "tones": tones }))
    }

    pub(crate) fn set_chat_tone(
        &mut self,
        identity: &str,
        tone: Option<&str>,
    ) -> Result<Value, ApiError> {
        let (key, _) = self.readable_conversation(identity)?;
        self.store.set_chat_tone(&key, tone)?;
        Ok(json!({}))
    }

    pub(crate) async fn add_contact(&mut self, identity: &str) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity)
            .ok_or(ApiError::IdentityFormat)?
            .to_string();
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        if own.threema_id == id {
            return Err(ApiError::ContactIsSelf);
        }
        if let Some(mut existing) = self.store.contact(&id)? {
            if !contacts::pin_predefined(&self.config, &mut existing) {
                return Err(ApiError::ContactInvalid);
            }
            // Known from a group or deleted before: make it a regular contact again.
            existing.acquaintance_level = pc::AcquaintanceLevel::Direct;
            self.store.upsert_contact(&existing)?;
            return Ok(self.contact_value(&existing));
        }
        let mut found =
            contacts::lookup(&self.https, &self.config, std::slice::from_ref(&id)).await?;
        let Some((_, Lookup::Found(contact))) = found.pop() else {
            return Err(ApiError::ContactInvalid);
        };
        let mut contact = *contact;
        contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
        self.store.upsert_contact(&contact)?;
        tracing::info!("contact added");
        Ok(self.contact_value(&contact))
    }

    pub(crate) fn update_contact(
        &mut self,
        identity: &str,
        first_name: Option<String>,
        last_name: Option<String>,
    ) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let mut contact = self
            .store
            .contact(&id.to_string())?
            .ok_or(ApiError::ContactNotFound)?;
        contact.first_name = clean(first_name);
        contact.last_name = clean(last_name);
        contact.sync_state = pc::SyncState::Custom;
        self.store.upsert_contact(&contact)?;
        Ok(self.contact_value(&contact))
    }

    pub(crate) fn set_contact_policies(
        &mut self,
        identity: &str,
        read_receipts: Option<u8>,
        typing: Option<u8>,
    ) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let mut contact = self
            .store
            .contact(&id.to_string())?
            .ok_or(ApiError::ContactNotFound)?;
        if let Some(choice) = read_receipts {
            contact.read_receipt_policy_override = contacts::read_receipt_policy(choice);
        }
        if let Some(choice) = typing {
            contact.typing_indicator_policy_override = contacts::typing_policy(choice);
        }
        self.store.upsert_contact(&contact)?;
        Ok(self.contact_value(&contact))
    }

    /// Returns how many contacts had their own choice.
    pub(crate) fn clear_contact_policies(&mut self) -> Result<Value, ApiError> {
        let mut cleared = 0;
        for mut contact in self.store.contacts()? {
            if contact.read_receipt_policy_override.is_some()
                || contact.typing_indicator_policy_override.is_some()
            {
                contact.read_receipt_policy_override = None;
                contact.typing_indicator_policy_override = None;
                self.store.upsert_contact(&contact)?;
                cleared += 1;
            }
        }
        tracing::info!(cleared, "individual receipt settings cleared");
        Ok(json!({ "cleared": cleared }))
    }

    pub(crate) fn delete_contact(&mut self, identity: &str) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let key = id.to_string();
        // Someone a group names stays as a group acquaintance, like Android's
        // AcquaintanceLevel.GROUP: the group needs the name and the public key, and such a
        // contact is out of the contact list either way. A group we left counts too: it still
        // goes to the other devices, and Threema Desktop refuses to link for a group naming
        // someone it has no contact for.
        let demoted = match self.store.contact(&key)? {
            Some(mut contact) if self.store.is_named_by_a_group(&key)? => {
                contact.acquaintance_level = pc::AcquaintanceLevel::GroupOrDeleted;
                self.store.upsert_contact(&contact)?;
                true
            }
            _ => false,
        };
        if !demoted {
            self.store.delete_contact(&key)?;
        }
        // Forward security sessions belong to the contact. Like Android, without a Terminate: it
        // would show them a reset line the moment they are deleted. Their next message is rejected.
        self.store.delete_fs_sessions(&key)?;
        Ok(json!({}))
    }

    /// The directory says that `contact` has dropped forward secrecy: end what is left of the
    /// sessions, and say so in its chat (Android's `OnFSFeatureMaskDowngradedTask`).
    pub(crate) fn fs_support_lost(
        &self,
        contact: &libthreema::model::contact::Contact,
    ) -> Result<(), ApiError> {
        let own = self.store.identity()?.map(|own| own.threema_id);
        let Some(own) = own.and_then(|own| own.parse().ok()) else {
            return Ok(());
        };
        let envelopes = match crate::fs_store::end_sessions(&self.store, own, contact.identity) {
            Ok(envelopes) => envelopes,
            Err(error) => {
                tracing::warn!(%error, "cannot end forward secrecy sessions");
                return Ok(());
            }
        };
        if envelopes.is_empty() {
            return Ok(());
        }
        let peer = contact.identity.to_string();
        for envelope in envelopes {
            self.tell_connection(ToConnection::FsControl {
                peer: peer.clone(),
                peer_key: Some(contact.public_key),
                envelope,
            });
        }
        if crate::providers::shows_fs_status(&self.store, &self.config, &peer)?
            && let Some(message) = self.store.insert_status(&peer, "fsUnavailable")?
        {
            let _ = self
                .media_events
                .send(crate::providers::CoreEvent::MessageAdded {
                    message,
                    send_delivery_receipt: false,
                });
        }
        Ok(())
    }

    fn revocation_json(&self, checked: bool) -> Result<Value, ApiError> {
        let at = self
            .store
            .setting(keys::REVOCATION_SET_AT)?
            .and_then(|v| v.parse::<i64>().ok());
        Ok(json!({
            "set": at.is_some(),
            "setAt": at.filter(|at| *at > 0),
            "checked": checked,
        }))
    }

    /// `linkEmail` (Android `linkWithEmail` / `unlinkEmail`). Only on the user's request; the
    /// address goes to Threema's directory and nowhere else, and is never logged.
    pub(crate) async fn link_email(&mut self, email: &str) -> Result<Value, ApiError> {
        let email = email.trim().to_lowercase();
        let valid = email.contains('@') && email.len() <= 254;
        if !email.is_empty() && !valid {
            return Err(ApiError::EmailInvalid);
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let pending = crate::directory::link_email(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
            &email,
            &self.version.language,
        )
        .await?;
        if email.is_empty() {
            self.store.delete_setting(keys::LINKED_EMAIL)?;
            self.store.delete_setting(keys::LINKED_EMAIL_PENDING)?;
        } else {
            self.store.set_setting(keys::LINKED_EMAIL, &email)?;
            if pending {
                self.store.set_setting(keys::LINKED_EMAIL_PENDING, "1")?;
            } else {
                self.store.delete_setting(keys::LINKED_EMAIL_PENDING)?;
            }
        }
        // log-ok: whether it is empty, not the address
        tracing::info!(unlink = email.is_empty(), pending, "e-mail link changed");
        self.profile_json()
    }

    /// A pending e-mail link: confirmed by now? (Android `checkEmailLinkState`, on opening the
    /// profile and in maintenance.) Returns whether it changed.
    pub(crate) async fn check_email_link(&mut self) -> Result<bool, ApiError> {
        let (Some(_), Some(email), Some(identity)) = (
            self.store.setting(keys::LINKED_EMAIL_PENDING)?,
            self.store.setting(keys::LINKED_EMAIL)?,
            self.store.identity()?,
        ) else {
            return Ok(false);
        };
        if self.store.directory_resting() {
            return Ok(false);
        }
        if !crate::directory::email_linked(&self.https, &identity.threema_id, &email)
            .await
            .map_err(|error| self.directory_error(error))?
        {
            return Ok(false);
        }
        self.store.delete_setting(keys::LINKED_EMAIL_PENDING)?;
        tracing::info!("e-mail link confirmed");
        Ok(true)
    }

    /// `linkMobile` (Android `linkWithMobileNumber` / `unlinkMobileNumber`). Only numbers that
    /// say their country: the SMS must not go to a number we guessed (ADR 0016).
    pub(crate) async fn link_mobile(&mut self, number: &str) -> Result<Value, ApiError> {
        let normalized = if number.trim().is_empty() {
            String::new()
        } else {
            crate::address_book::international_phone(number).ok_or(ApiError::PhoneInvalid)?
        };
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let verification = crate::directory::link_mobile(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
            &normalized,
            &self.version.language,
        )
        .await?;
        match verification {
            Some(id) => {
                // The server has dropped the old number as of now (Android:
                // `linkWithMobileNumber`, "it is removed immediately").
                self.store.delete_setting(keys::LINKED_MOBILE)?;
                self.store
                    .set_setting(keys::LINKED_MOBILE_PENDING, &format!("{normalized} {id}"))?;
            }
            None => {
                self.store.delete_setting(keys::LINKED_MOBILE_PENDING)?;
                if normalized.is_empty() {
                    self.store.delete_setting(keys::LINKED_MOBILE)?;
                } else {
                    // Already linked to this ID.
                    self.store.set_setting(keys::LINKED_MOBILE, &normalized)?;
                }
            }
        }
        tracing::info!(unlink = normalized.is_empty(), "phone link changed");
        self.profile_json()
    }

    fn pending_mobile(&self) -> Result<(String, String), ApiError> {
        let pending = self
            .store
            .setting(keys::LINKED_MOBILE_PENDING)?
            .ok_or(ApiError::NoVerification)?;
        let (number, id) = pending.split_once(' ').ok_or(ApiError::NoVerification)?;
        Ok((number.to_owned(), id.to_owned()))
    }

    /// `verifyMobile`: the SMS code.
    pub(crate) async fn verify_mobile(
        &mut self,
        code: &str,
        from_link: bool,
    ) -> Result<Value, ApiError> {
        // The SMS's link tapped twice, or after typing the code: it is confirmed (Android too).
        if from_link
            && self.store.setting(keys::LINKED_MOBILE_PENDING)?.is_none()
            && self.store.setting(keys::LINKED_MOBILE)?.is_some()
        {
            return self.profile_json();
        }
        let (number, id) = self.pending_mobile()?;
        crate::directory::verify_mobile(&self.https, &id, code.trim()).await?;
        self.store.set_setting(keys::LINKED_MOBILE, &number)?;
        self.store.delete_setting(keys::LINKED_MOBILE_PENDING)?;
        tracing::info!("phone link verified");
        // Also from a link in the SMS, with a profile page open or not.
        self.note_ui_events(&["profileChanged"]);
        self.profile_json()
    }

    /// `callMobile`: a call that reads the code.
    pub(crate) async fn call_mobile(&mut self) -> Result<Value, ApiError> {
        let (_, id) = self.pending_mobile()?;
        crate::directory::call_mobile(&self.https, &id).await?;
        self.profile_json()
    }

    /// `revocationStatus` (Android `checkRevocationKey`).
    pub(crate) async fn revocation_status(&mut self, cached: bool) -> Result<Value, ApiError> {
        if cached {
            return self.revocation_json(false);
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let at = crate::directory::check_revocation_key(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
        )
        .await?;
        match at {
            Some(at) => self
                .store
                .set_setting(keys::REVOCATION_SET_AT, &at.to_string())?,
            None => self.store.delete_setting(keys::REVOCATION_SET_AT)?,
        }
        self.revocation_json(true)
    }

    /// `setRevocationPassword` (Android `setRevocationKey`), then the server's state.
    pub(crate) async fn set_revocation_password(
        &mut self,
        password: &str,
    ) -> Result<Value, ApiError> {
        match password.chars().count() {
            0..=7 => return Err(ApiError::PasswordTooShort),
            257.. => return Err(ApiError::PasswordTooLong),
            _ => {}
        }
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        crate::directory::set_revocation_key(
            &self.https,
            &identity.threema_id,
            &ClientKey::from(*identity.client_key),
            password,
        )
        .await?;
        tracing::info!("revocation password set");
        self.revocation_status(false).await
    }

    pub(crate) fn profile_json(&self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let public_key = ClientKey::from(*identity.client_key).public_key();
        Ok(json!({
            "identity": identity.threema_id,
            "nickname": self.store.setting(keys::NICKNAME)?,
            "askNickname": self.store.setting(keys::NICKNAME)?.is_none()
                && self.store.setting(keys::NICKNAME_UNANSWERED)?.as_deref() == Some("ask"),
            "publicKey": public_key.to_string(),
            "linkedEmail": self.store.setting(keys::LINKED_EMAIL)?,
            "linkedEmailPending": self.store.setting(keys::LINKED_EMAIL_PENDING)?.is_some(),
            "linkedMobile": self.store.setting(keys::LINKED_MOBILE)?,
            // E.164 without `+`, waiting for its SMS code.
            "linkedMobilePending": self.store.setting(keys::LINKED_MOBILE_PENDING)?
                .and_then(|p| p.split(' ').next().map(str::to_owned)),
            "pictureVersion": self.store.setting(keys::PROFILE_PICTURE_VERSION)?
                .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0),
            "pictureShare": self.store.picture_share()?.as_str(),
            "pictureRecipients": self.store.picture_recipients()?,
        }))
    }

    pub(crate) fn set_nickname(&mut self, nickname: &str) -> Result<Value, ApiError> {
        // Threema limits nicknames to 32 bytes (`ProfileUtil` / CSP metadata).
        let mut nickname = nickname.trim().to_owned();
        while nickname.len() > 32 {
            nickname.pop();
        }
        if nickname.is_empty() {
            // The profile page saves when its field loses focus: only taking a name away is an
            // answer, and from there on contacts are told to drop it.
            if self.store.setting(keys::NICKNAME)?.is_some() {
                self.store.delete_setting(keys::NICKNAME)?;
                self.store.delete_setting(keys::NICKNAME_UNANSWERED)?;
            }
        } else {
            self.store.set_setting(keys::NICKNAME, &nickname)?;
            self.store.delete_setting(keys::NICKNAME_UNANSWERED)?;
        }
        // The nickname rides along in the metadata of every message we send
        // (connection.rs), so contacts see it with the next one.
        self.profile_json()
    }

    /// Not now: no more asking, and messages keep leaving contacts' stored name alone.
    pub(crate) fn decline_nickname(&mut self) -> Result<Value, ApiError> {
        if self.store.setting(keys::NICKNAME_UNANSWERED)?.is_some() {
            self.store.set_setting(keys::NICKNAME_UNANSWERED, "asked")?;
        }
        self.profile_json()
    }

    /// Refresh feature masks, activity states and keys of all contacts from the directory, at most
    /// once a day (Android refreshes feature masks periodically too). Returns whether something
    /// changed.
    /// Daily (or, `force`, now): the contacts' states, keys and feature masks.
    pub(crate) async fn refresh_contacts(&mut self, force: bool) -> Result<bool, ApiError> {
        let now = crate::messages::now_ms() / 1000;
        let last = self
            .store
            .setting(keys::CONTACTS_REFRESHED_AT)?
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if !force && (now - last < 86_400 || self.store.directory_resting()) {
            return Ok(false);
        }
        let stored = self.store.contacts()?;
        if stored.is_empty() {
            return Ok(false);
        }
        let ids: Vec<String> = stored.iter().map(|c| c.identity.to_string()).collect();
        let found = contacts::lookup(&self.https, &self.config, &ids)
            .await
            .map_err(|error| self.directory_error(error))?;
        let mut changed = false;
        for (id, result) in found {
            // Read again: the lookup took a while, and a rename or a deletion meanwhile stands.
            let Some(mut contact) = self.store.contact(&id)? else {
                continue;
            };
            let had_fs = contacts::has_fs_support(&contact);
            if contacts::apply_lookup(&self.config, &mut contact, result) {
                self.store.upsert_contact(&contact)?;
                if had_fs && !contacts::has_fs_support(&contact) {
                    self.fs_support_lost(&contact)?;
                }
                changed = true;
            }
        }
        self.store
            .set_setting(keys::CONTACTS_REFRESHED_AT, &now.to_string())?;
        tracing::info!(contacts = ids.len(), changed, "contacts refreshed");
        Ok(changed)
    }

    /// Contacts restored from Threema Safe but not imported yet (see `safe_pending_import`).
    pub(crate) fn pending_backup_contacts(&self) -> Result<Vec<backup::Contact>, ApiError> {
        Ok(self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|v| serde_json::from_value(v["contacts"].clone()).ok())
            .flatten()
            .unwrap_or_default())
    }

    /// Import restored Safe contacts (`ThreemaSafeServiceImpl` restore): one bulk lookup; IDs that
    /// are not valid any more are dropped, like on Android. Returns whether anything changed.
    /// On network errors the contacts stay pending and the next maintenance retries.
    pub(crate) async fn import_pending_contacts(&mut self) -> Result<bool, ApiError> {
        let pending = self.pending_backup_contacts()?;
        if pending.is_empty() {
            return Ok(false);
        }
        let own = self.store.identity()?.map(|i| i.threema_id);
        let ids: Vec<String> = pending
            .iter()
            .filter_map(|c| contacts::parse_identity(&c.identity).map(|id| id.to_string()))
            .filter(|id| Some(id) != own.as_ref())
            .collect();
        let found = contacts::lookup(&self.https, &self.config, &ids).await?;
        let mut imported = 0_usize;
        let mut revoked = 0_usize;
        for (id, result) in found {
            let contact = match result {
                Lookup::Found(contact) => contact,
                // Revoked or never existed: dropped, like on Android (counted for the summary).
                Lookup::Invalid => {
                    revoked += 1;
                    continue;
                }
                Lookup::Refused => continue,
            };
            let Some(entry) = pending
                .iter()
                .find(|c| c.identity.eq_ignore_ascii_case(&id))
            else {
                continue;
            };
            if self.store.contact(&id)?.is_none() {
                let mut restored = contacts::apply_backup(*contact, entry);
                if !contacts::pin_predefined(&self.config, &mut restored) {
                    continue;
                }
                self.store.upsert_contact(&restored)?;
                if entry.is_private {
                    self.mark_private(&id, true)?;
                }
                self.store.note_restored_chat(&id, entry.last_update)?;
                imported += 1;
            }
        }
        // Done with contacts; groups and lists follow (`import_pending_groups`).
        let mut rest: Value = self
            .store
            .setting(keys::SAFE_PENDING_IMPORT)?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({}));
        if let Some(object) = rest.as_object_mut() {
            object.remove("contacts");
        }
        self.store
            .set_setting(keys::SAFE_PENDING_IMPORT, &rest.to_string())?;
        self.update_import_summary(serde_json::json!({
            "contacts": imported,
            "revokedContacts": revoked,
        }))?;
        tracing::info!(
            imported,
            revoked,
            total = pending.len(),
            "Safe contacts imported"
        );
        Ok(true)
    }

    /// Ask up to `limit` direct contacts without a picture for one: a restored device has no
    /// pictures until it asks (Safe does not carry contact pictures). Called per round
    /// (connect, maintenance), so the requests stay bounded; each contact is also bounded by
    /// its own request backoff. Blocked contacts are left alone. Returns how many were queued.
    pub(crate) fn request_missing_pictures(&self, limit: usize) -> usize {
        let Ok(contacts) = self.store.contacts() else {
            return 0;
        };
        let blocked = self
            .app_settings()
            .map(|s| s.blocked_contacts)
            .unwrap_or_default();
        let mut queued = 0;
        for contact in contacts {
            if queued >= limit {
                break;
            }
            if contact.acquaintance_level
                != libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct
            {
                continue;
            }
            let id = contact.identity.to_string();
            if blocked.contains(&id) {
                continue;
            }
            if self.store.request_picture(&id).unwrap_or(false) {
                queued += 1;
            }
        }
        if queued > 0 {
            self.tell_connection(crate::connection::ToConnection::SendPending);
        }
        queued
    }
}

impl App {
    /// `myQrCode`: our contact QR code (text and modules for drawing).
    pub(crate) fn my_qr_code(&self) -> Result<Value, ApiError> {
        let identity = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let public_key = ClientKey::from(*identity.client_key).public_key();
        let text = crate::qr::contact_text(&identity.threema_id, &public_key.to_string());
        let (size, bits) =
            crate::qr::modules(&text).ok_or_else(|| ApiError::Internal("QR code".into()))?;
        Ok(json!({ "text": text, "size": size, "bits": bits }))
    }

    /// `verifyQrCode`: a scanned `3mid:` code. The key must be the one we have (or, for someone
    /// new, the directory's): then the contact is _fully verified_ (and added if new). A code
    /// that doesn't match, or has expired, changes nothing.
    pub(crate) async fn verify_qr_code(&mut self, text: &str) -> Result<Value, ApiError> {
        let code = crate::qr::parse(text).ok_or(ApiError::QrInvalid)?;
        if code.is_expired(crate::messages::now_ms() / 1000) {
            return Err(ApiError::QrExpired);
        }
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        if own.threema_id == code.identity {
            return Err(ApiError::ContactIsSelf);
        }
        let contact = match self.store.contact(&code.identity)? {
            Some(contact) => contact,
            None => {
                let id = contacts::parse_identity(&code.identity)
                    .ok_or(ApiError::IdentityFormat)?
                    .to_string();
                let mut found =
                    contacts::lookup(&self.https, &self.config, std::slice::from_ref(&id)).await?;
                let Some((_, Lookup::Found(contact))) = found.pop() else {
                    return Err(ApiError::ContactInvalid);
                };
                *contact
            }
        };
        self.verify_with_key(contact, &code.public_key)
    }

    /// The key of a scanned code against the contact's (stored or just looked up, then stored
    /// only if it matches).
    fn verify_with_key(
        &mut self,
        mut contact: libthreema::model::contact::ContactInit,
        public_key: &[u8; 32],
    ) -> Result<Value, ApiError> {
        if contacts::hex_bytes(&contact.public_key.to_string()).as_slice() != public_key {
            tracing::warn!("scanned QR code doesn't match the contact's public key");
            return Err(ApiError::QrMismatch);
        }
        contact.verification_level = pc::VerificationLevel::FullyVerified;
        contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
        self.store.upsert_contact(&contact)?;
        Ok(self.contact_value(&contact))
    }
}

impl App {
    /// `matchAddressBook`: find address book entries on Threema (hashes only) and make them
    /// contacts, named as in the address book (unless the user named them), server verified.
    pub(crate) async fn match_address_book(
        &mut self,
        entries: Vec<crate::address_book::Entry>,
    ) -> Result<Value, ApiError> {
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let client_key = ClientKey::from(*own.client_key);
        let (emails, phones) = crate::address_book::hash_entries(&entries, &self.version.country);
        if emails.is_empty() && phones.is_empty() {
            return Ok(json!({ "matched": 0, "added": 0 }));
        }
        let token = match self.store.setting(keys::MATCH_TOKEN)? {
            Some(token) => token,
            None => {
                let token =
                    crate::directory::match_token(&self.https, &own.threema_id, &client_key)
                        .await?;
                self.store.set_setting(keys::MATCH_TOKEN, &token)?;
                token
            }
        };
        let email_list: Vec<String> = emails.keys().cloned().collect();
        let phone_list: Vec<String> = phones.keys().cloned().collect();
        let found =
            match crate::directory::match_identities(&self.https, &token, &email_list, &phone_list)
                .await
            {
                Ok(found) => found,
                Err(DirectoryError::RateLimited) => return Err(ApiError::RateLimited),
                // The token may have expired: once more with a new one (like Android).
                Err(_) => {
                    let token =
                        crate::directory::match_token(&self.https, &own.threema_id, &client_key)
                            .await?;
                    self.store.set_setting(keys::MATCH_TOKEN, &token)?;
                    crate::directory::match_identities(
                        &self.https,
                        &token,
                        &email_list,
                        &phone_list,
                    )
                    .await?
                }
            };
        let (mut matched, mut added) = (0, 0);
        let excluded = self.app_settings()?.sync_excluded_ids;
        for hit in found {
            if hit.identity == own.threema_id || excluded.contains(&hit.identity) {
                continue;
            }
            let Some(entry) = hit
                .email_hash
                .as_ref()
                .and_then(|h| emails.get(h))
                .or_else(|| hit.mobile_hash.as_ref().and_then(|h| phones.get(h)))
                .and_then(|index| entries.get(*index))
            else {
                continue;
            };
            let mut contact = match self.store.contact(&hit.identity)? {
                Some(contact) => contact,
                None => {
                    match self.add_contact(&hit.identity).await {
                        Ok(_) => {}
                        Err(ApiError::RateLimited) => return Err(ApiError::RateLimited),
                        Err(_) => continue,
                    }
                    added += 1;
                    self.store
                        .contact(&hit.identity)?
                        .ok_or(ApiError::ContactNotFound)?
                }
            };
            if contacts::hex_bytes(&contact.public_key.to_string()).as_slice() != hit.public_key {
                tracing::warn!("address book match with a different public key, skipped");
                if contacts::distrust_changed_key(&mut contact) {
                    self.store.upsert_contact(&contact)?;
                }
                continue;
            }
            matched += 1;
            let unnamed = contact.first_name.as_deref().unwrap_or("").is_empty()
                && contact.last_name.as_deref().unwrap_or("").is_empty();
            if unnamed {
                contact.first_name = Some(entry.first_name.clone()).filter(|n| !n.is_empty());
                contact.last_name = Some(entry.last_name.clone()).filter(|n| !n.is_empty());
            }
            if contact.verification_level == pc::VerificationLevel::Unverified {
                contact.verification_level = pc::VerificationLevel::ServerVerified;
            }
            contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
            self.store.upsert_contact(&contact)?;
        }
        tracing::info!(matched, added, "address book matched");
        Ok(json!({ "matched": matched, "added": added }))
    }
}

#[cfg(test)]
mod contact_policy_tests {
    use serde_json::{Value, json};

    use crate::app::{App, StartConfig};

    async fn call(app: &mut App, mut command: Value) -> Value {
        command["id"] = json!(1);
        serde_json::from_str(&app.handle(&command.to_string()).await).unwrap()
    }

    fn app(dir: &std::path::Path) -> App {
        App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
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
        .unwrap()
    }

    // No identity here: an answer other than `phoneInvalid` would mean it went further, towards
    // the network.
    #[tokio::test(flavor = "current_thread")]
    async fn a_national_number_is_refused_before_any_network_access() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        for number in ["079 123 45 67", "0664 1234567", "+0 79 123 45 67"] {
            let answer = call(&mut app, json!({ "cmd": "linkMobile", "number": number })).await;
            assert_eq!(answer["error"]["kind"], "phoneInvalid", "{answer}");
        }
        let answer = call(
            &mut app,
            json!({ "cmd": "linkMobile", "number": "+41791234567" }),
        )
        .await;
        assert_eq!(answer["error"]["kind"], "noIdentity", "{answer}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn normalize_mobile_answers_without_an_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let answer = call(
            &mut app,
            json!({ "cmd": "normalizeMobile", "number": "+49 (0) 171 1234567" }),
        )
        .await;
        assert_eq!(answer["result"]["number"], "491711234567", "{answer}");
        let answer = call(
            &mut app,
            json!({ "cmd": "normalizeMobile", "number": "0171 1234567" }),
        )
        .await;
        assert_eq!(answer["ok"], true, "{answer}");
        assert_eq!(answer["result"]["number"], Value::Null, "{answer}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_new_contact_with_another_key_is_not_added() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let contact = crate::contacts::test_contact("BBBBBBBB");
        assert!(matches!(
            app.verify_with_key(contact, &[1; 32]),
            Err(crate::app::ApiError::QrMismatch)
        ));
        assert!(app.store.contact("BBBBBBBB").unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_new_contact_with_the_right_key_is_added_verified() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let contact = crate::contacts::test_contact("BBBBBBBB");
        app.verify_with_key(contact, &[9; 32]).unwrap();
        let stored = app.store.contact("BBBBBBBB").unwrap().expect("added");
        assert_eq!(
            stored.verification_level,
            libthreema::protobuf::d2d_sync::contact::VerificationLevel::FullyVerified
        );
        assert_eq!(
            stored.acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct
        );
    }

    // Before anything else, so also here where there is no identity and no network.
    #[tokio::test(flavor = "current_thread")]
    async fn an_expired_code_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let key = "09".repeat(32);
        let answer = call(
            &mut app,
            json!({"cmd": "verifyQrCode", "text": format!("3mid:BBBBBBBB,{key},1700000000")}),
        )
        .await;
        assert_eq!(answer["error"]["kind"], "qrExpired", "{answer}");
        let stored = app.store.contact("BBBBBBBB").unwrap().expect("kept");
        assert_eq!(
            stored.verification_level,
            libthreema::protobuf::d2d_sync::contact::VerificationLevel::Unverified
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn qr_kind_asks_for_a_web_server_only_until_it_is_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let code = |host: &str| {
            let mut bytes = 2_u16.to_be_bytes().to_vec();
            bytes.push(0x03);
            bytes.extend([1; 32]);
            bytes.extend([2; 32]);
            bytes.extend([0; 32]);
            bytes.extend(443_u16.to_be_bytes());
            bytes.extend(host.as_bytes());
            data_encoding::BASE64.encode(&bytes)
        };
        let asked = call(
            &mut app,
            json!({"cmd": "qrKind", "text": code("web.example.org")}),
        )
        .await;
        assert_eq!(asked["result"]["confirmHost"], true, "{asked}");
        let ours = call(
            &mut app,
            json!({"cmd": "qrKind", "text": code("saltyrtc-01.threema.ch")}),
        )
        .await;
        assert_eq!(ours["result"]["confirmHost"], false, "{ours}");
        app.store.set_web_host("web.example.org", None).unwrap();
        let allowed = call(
            &mut app,
            json!({"cmd": "qrKind", "text": code("web.example.org")}),
        )
        .await;
        assert_eq!(allowed["result"]["confirmHost"], false, "{allowed}");
    }

    // Deleting a contact who is in one of our groups keeps it as a group acquaintance, like
    // Android: the group needs the name and the public key, and the contact list hides it
    // either way.
    #[tokio::test(flavor = "current_thread")]
    async fn deleting_someone_a_left_group_names_keeps_it_for_the_other_devices() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "CCCCCCCC".parse().unwrap(),
            group_id: 0x1234,
        };
        app.store
            .upsert_group(&crate::groups::Group {
                key: crate::groups::key(&identity),
                identity,
                name: "Crew".into(),
                members: vec!["BBBBBBBB".into()],
                left: true,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();

        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        let kept = app.store.contact("BBBBBBBB").unwrap().expect("kept");
        assert_eq!(
            kept.acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_group_member_keeps_it_as_acquaintance() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("CCCCCCCC"))
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 0x1234,
        };
        app.store
            .upsert_group(&crate::groups::Group {
                key: crate::groups::key(&identity),
                identity,
                name: "Crew".into(),
                members: vec!["BBBBBBBB".into()],
                left: false,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();

        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        let kept = app.store.contact("BBBBBBBB").unwrap().expect("kept");
        assert_eq!(
            kept.acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted
        );
        // Not in a group: deleted for good.
        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "CCCCCCCC"}),
        )
        .await;
        assert!(app.store.contact("CCCCCCCC").unwrap().is_none());
    }

    /// The contact page shows what the reply holds until something else reloads it.
    #[tokio::test(flavor = "current_thread")]
    async fn changing_a_name_answers_with_the_whole_contact() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store.set_pinned("BBBBBBBB", true).unwrap();
        app.store.set_contact_picture("BBBBBBBB", Some(5)).unwrap();
        app.store
            .set_chat_notifications(
                "BBBBBBBB",
                Some((crate::chat_notifications::Policy::Never, None)),
            )
            .unwrap();
        call(
            &mut app,
            json!({"cmd": "setBlocked", "identity": "BBBBBBBB", "blocked": true}),
        )
        .await;

        let reply = call(
            &mut app,
            json!({"cmd": "updateContact", "identity": "BBBBBBBB", "firstName": "Anna"}),
        )
        .await;
        let contact = &reply["result"];
        assert_eq!(contact["firstName"], "Anna");
        assert_eq!(contact["pinned"], true);
        assert_eq!(contact["blocked"], true);
        assert!(contact["pictureVersion"].as_i64().unwrap() > 0);
        assert_eq!(contact["notifications"]["policy"], "never");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_revoked_id_cannot_be_written_to() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.activity_state = libthreema::protobuf::d2d_sync::contact::ActivityState::Invalid;
        app.store.upsert_contact(&contact).unwrap();
        let send = json!({"cmd": "sendText", "identity": "BBBBBBBB", "text": "hello"});
        let reply = call(&mut app, send).await;
        assert_eq!(reply["error"]["kind"], "contactRevoked");
        let shown = call(&mut app, json!({"cmd": "contact", "identity": "BBBBBBBB"})).await;
        assert_eq!(shown["result"]["invalid"], true);
        assert_eq!(shown["result"]["canCall"], false);
        assert_eq!(shown["result"]["canVideoCall"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn writing_to_someone_makes_them_a_direct_contact() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let mut events = app.take_media_events().unwrap();
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted;
        app.store.upsert_contact(&contact).unwrap();
        let send = json!({"cmd": "sendText", "identity": "BBBBBBBB", "text": "hello"});
        assert_eq!(call(&mut app, send).await["ok"], true);
        assert_eq!(
            app.store
                .contact("BBBBBBBB")
                .unwrap()
                .unwrap()
                .acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct
        );
        assert!(matches!(
            events.try_recv(),
            Ok(crate::providers::CoreEvent::ContactsChanged)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_list_goes_to_those_who_can_get_it_and_makes_them_direct_contacts() {
        use libthreema::protobuf::d2d_sync::contact::{AcquaintanceLevel, ActivityState};
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let add = |id: &str, level, state| {
            let mut contact = crate::contacts::test_contact(id);
            contact.acquaintance_level = level;
            contact.activity_state = state;
            app.store.upsert_contact(&contact).unwrap();
        };
        add(
            "BBBBBBBB",
            AcquaintanceLevel::GroupOrDeleted,
            ActivityState::Active,
        );
        add(
            "CCCCCCCC",
            AcquaintanceLevel::GroupOrDeleted,
            ActivityState::Invalid,
        );
        add(
            "DDDDDDDD",
            AcquaintanceLevel::GroupOrDeleted,
            ActivityState::Active,
        );
        app.store
            .set_setting(
                crate::store::keys::APP_SETTINGS,
                &json!({ "blockUnknown": true, "blockedContacts": ["DDDDDDDD"] }).to_string(),
            )
            .unwrap();
        let list = crate::distribution_lists::DistributionList {
            key: "d:0000000000000001".into(),
            name: "List".into(),
            members: ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD", "EEEEEEEE"]
                .map(str::to_owned)
                .to_vec(),
            created_at: 0,
        };
        assert_eq!(app.list_receivers(&list).unwrap(), vec!["BBBBBBBB"]);
        let level = |id: &str| app.store.contact(id).unwrap().unwrap().acquaintance_level;
        assert_eq!(level("BBBBBBBB"), AcquaintanceLevel::Direct);
        assert_eq!(level("CCCCCCCC"), AcquaintanceLevel::GroupOrDeleted);
        assert_eq!(level("DDDDDDDD"), AcquaintanceLevel::GroupOrDeleted);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn individual_settings_and_clearing_them() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        let set = json!({"cmd": "setContactPolicies", "identity": "BBBBBBBB",
                         "readReceipts": 2, "typing": 1});
        let contact = call(&mut app, set).await;
        assert_eq!(contact["result"]["readReceipts"], 2);
        assert_eq!(contact["result"]["typing"], 1);
        // Leaving a field out keeps it.
        let set = json!({"cmd": "setContactPolicies", "identity": "BBBBBBBB", "typing": 0});
        let contact = call(&mut app, set).await;
        assert_eq!(
            (
                &contact["result"]["readReceipts"],
                &contact["result"]["typing"]
            ),
            (&json!(2), &json!(0))
        );
        let cleared = call(&mut app, json!({"cmd": "clearContactPolicies"})).await;
        assert_eq!(cleared["result"]["cleared"], 1);
        let contact = call(&mut app, json!({"cmd": "contact", "identity": "BBBBBBBB"})).await;
        assert_eq!(contact["result"]["readReceipts"], 0);
    }
}
