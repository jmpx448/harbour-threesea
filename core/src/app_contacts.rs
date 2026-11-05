// SPDX-License-Identifier: AGPL-3.0-only
//! Contact and profile commands.

use libthreema::{common::keys::ClientKey, protobuf::d2d_sync::contact as pc};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App, Shared},
    connection::ToConnection,
    contacts::{self, Lookup},
    directory::DirectoryError,
    safe::backup,
    store::keys,
};

/// How long the contact refresh waits unless the directory says otherwise (Android's default).
const CONTACT_REFRESH_DEFAULT_S: i64 = 86_400;

/// The wait between contact refreshes, from what the directory last gave as `checkInterval`.
fn contact_refresh_interval(check_interval: i64) -> i64 {
    if check_interval > 0 {
        check_interval
    } else {
        CONTACT_REFRESH_DEFAULT_S
    }
}

/// Whom a contact refresh asks about: revoked IDs only if the user asked for it.
fn identities_to_check(
    contacts: &[libthreema::model::contact::Contact],
    force: bool,
) -> Vec<String> {
    contacts
        .iter()
        .filter(|contact| force || contact.activity_state != pc::ActivityState::Invalid)
        .map(|contact| contact.identity.to_string())
        .collect()
}

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

impl Shared {
    /// The feature mask this device is to have in the directory: forward security only while
    /// neither multi-device nor the user's switch turns it off.
    pub(crate) fn feature_mask(&self) -> Result<u64, ApiError> {
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
        Ok(feature_mask)
    }

    /// Publish [`FEATURE_MASK`] if the directory doesn't have it yet; without forward security
    /// while multi-device is active (SE-199, like Android's `updateFeatureMask(isFsAllowed)`).
    pub(crate) async fn publish_feature_mask(&self) -> Result<(), ApiError> {
        let Some(identity) = self.store.identity()? else {
            return Ok(());
        };
        let feature_mask = self.feature_mask()?;
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
}

impl App {
    /// The app settings plus the call video settings (kept apart: not in Threema Safe).
    pub(crate) fn settings_json(&self) -> Result<Value, ApiError> {
        let mut json = serde_json::to_value(self.app_settings()?)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        json["videoCalls"] = json!(self.video_calls_setting()?);
        json["videoQuality"] = json!(self.video_quality_setting()?);
        json["groupCalls"] = json!(self.group_calls_setting()?);
        json["showProfilePictures"] = json!(self.store.pictures_shown()?);
        Ok(json)
    }

    pub(crate) fn video_calls_setting(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::VIDEO_CALLS)?.as_deref() != Some("0"))
    }
}

impl Shared {
    /// Group calls on (Android's group call policy; the feature mask keeps the bit either way,
    /// as on Android, so the setting only decides what this device does).
    pub(crate) fn group_calls_setting(&self) -> Result<bool, ApiError> {
        Ok(self.store.setting(keys::GROUP_CALLS)?.as_deref() != Some("0"))
    }
}

impl App {
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
        if name == "showProfilePictures" {
            self.store.set_setting(
                crate::profile_pictures::SETTING_SHOW,
                if value { "1" } else { "0" },
            )?;
            self.apply_picture_gate();
            // Every list and page with a contact's picture, all at once, and a running call.
            for event in [
                crate::providers::CoreEvent::ContactsChanged,
                crate::providers::CoreEvent::GroupsChanged,
            ] {
                let _ = self.media_events.send(event);
            }
            self.emit_group_call_participants();
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
        // Whatever is under way for this ID ends here, and nothing it did is finished any more.
        self.drop_detached_work();
        // A dropped task goes when it next gets a turn, and takes what it was writing along.
        tokio::task::yield_now().await;
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
        self.connection_gone().await;
        // A kill from here on: the next start finishes the job, whatever the bridge got to hear.
        if let Err(error) = crate::runtime::mark_removal(&self.data_dir, linked_devices_remain) {
            tracing::warn!(%error, "the removal could not be marked");
        }
        // And whatever started while the others were being told.
        self.drop_detached_work();
        self.identity_removed = true;
        if let Err(error) = self.store.wipe() {
            tracing::error!(%error, "the store could not be emptied, its file goes next");
        }
        self.media.remove_all();
        // The next identity starts locked; its private marks wait for its own PIN.
        self.unlocked.set(false);
        self.write_pass = None;
        self.update_media_gate();
        // Credentials and links belong to the deleted identity, and the empty store switches
        // off what was read from it into process-wide state.
        self.turn_servers.take();
        self.sfu.take();
        self.pending_link.take();
        self.last_group_check.set(None);
        self.last_import_attempt.set(None);
        self.apply_multi_device();
        contacts::LAST_NAME_FIRST.store(false, std::sync::atomic::Ordering::Relaxed);
        self.apply_picture_gate();
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
        let mut list: Vec<(bool, String, String, Value)> = self
            .store
            .contacts()?
            .iter()
            // Contacts only known from groups/deleted ones are not shown in the contact list.
            .filter(|c| c.acquaintance_level == pc::AcquaintanceLevel::Direct)
            // Inactive and revoked IDs alike, as Android's `showInactiveContacts`.
            .filter(|c| !hide_inactive || c.activity_state == pc::ActivityState::Active)
            .map(|c| {
                let mut value = self.contact_value(c);
                // Nicknames ("~nick") sort under their letter; the section comes from the same.
                let key = contacts::sort_key(c, by_last_name);
                let section = contacts::section(&key);
                let display = contacts::display_name(c).to_lowercase();
                value["section"] = json!(section);
                (section == "#", key, display, value)
            })
            .collect();
        // Letters, then all the rest in one section; the same letters by their accents.
        list.sort_by(|a, b| (a.0, &a.1, &a.2).cmp(&(b.0, &b.1, &b.2)));
        let list: Vec<Value> = list.into_iter().map(|(_, _, _, value)| value).collect();
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

    /// Whether the UI's picture readers hand out contacts' pictures (start, change, removal).
    pub(crate) fn apply_picture_gate(&self) {
        self.media
            .set_contact_pictures_shown(self.store.pictures_shown().unwrap_or(true));
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
}

impl Shared {
    /// Contact JSON with its picture version (0: none; `image://threesea/contact/<ID>?<v>`).
    pub(crate) fn contact_value(&self, contact: &libthreema::model::contact::Contact) -> Value {
        let mut value = contacts::to_json(contact);
        value["pictureVersion"] = json!(
            self.store
                .shown_contact_picture_version(&contact.identity.to_string())
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
        // Out of the contact list, and a group we are in needs it: nothing to delete. (Asked only
        // for those: every row of the contact list is a direct contact.)
        value["neededByGroup"] = json!(
            contact.acquaintance_level == pc::AcquaintanceLevel::GroupOrDeleted
                && self
                    .store
                    .is_group_member(&contact.identity.to_string())
                    .unwrap_or(false)
        );
        value
    }
}

impl App {
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
        if blocked {
            self.store.reflected_forget_sender(&id)?;
        }
        Ok(match self.store.contact(&id)? {
            Some(contact) => self.contact_value(&contact),
            None => json!({}),
        })
    }
}

impl Shared {
    /// The exclusion list is part of the app settings too (`syncExcludedIds`, also in Safe).
    pub(crate) fn set_sync_excluded(
        &self,
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
}

impl App {
    pub(crate) fn blocked_contacts_json(&self) -> Result<Value, ApiError> {
        Ok(json!({ "blocked": self.id_list_json(&self.app_settings()?.blocked_contacts) }))
    }
}

impl Shared {
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
}

impl App {
    pub(crate) fn set_chat_notifications(
        &mut self,
        identity: &str,
        policy: Option<&str>,
        until: Option<i64>,
    ) -> Result<Value, ApiError> {
        let key = self.visible_conversation(identity)?.0;
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
        let (key, _) = self.visible_conversation(identity)?;
        self.store.set_chat_tone(&key, tone)?;
        Ok(json!({}))
    }
}

impl Shared {
    pub(crate) async fn add_contact(&self, identity: &str) -> Result<Value, ApiError> {
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
            self.made_direct.borrow_mut().push(id);
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
        self.new_contacts.borrow_mut().push(id);
        tracing::info!("contact added");
        Ok(self.contact_value(&contact))
    }
}

impl App {
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
                self.made_direct
                    .borrow_mut()
                    .push(contact.identity.to_string());
                cleared += 1;
            }
        }
        tracing::info!(cleared, "individual receipt settings cleared");
        Ok(json!({ "cleared": cleared }))
    }

    /// The user deletes a contact (ADR 0022; Android's `MarkContactAsDeletedBackgroundTask`): out
    /// of the contact list, its row stays as an acquaintance, and its chat goes, as does what was
    /// chosen for them.
    pub(crate) fn delete_contact(&mut self, identity: &str) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity).ok_or(ApiError::IdentityFormat)?;
        let key = id.to_string();
        // A hidden private chat isn't changed from outside (ADR 0009): it stays as it is, hidden,
        // and can be opened and deleted once unlocked. Whether there is one isn't given away.
        let chat_readable = match self.check_readable(&key) {
            Ok(()) => true,
            Err(ApiError::Locked) => false,
            Err(error) => return Err(error),
        };
        if chat_readable {
            self.store.delete_conversation(&key)?;
            self.store.forget_chat_choices(&key)?;
            // What was queued for them (a reaction, an edit, a picture request) stays here too.
            self.store.outbox_clear(&key)?;
            self.chat_removed(&key);
        }
        self.store.forget_restored_picture(&key)?;
        if let Some(mut contact) = self.store.contact(&key)? {
            contact.acquaintance_level = pc::AcquaintanceLevel::GroupOrDeleted;
            self.store.upsert_contact(&contact)?;
        }
        // The group pages still show it for someone a group names, also one we left.
        if !self.store.is_named_by_a_group(&key)? {
            self.store.cancel_picture_download(&key)?;
            self.store
                .cancel_synced_picture(&crate::profile_pictures::contact_picture_name(&key))?;
            self.store.set_contact_picture(&key, None)?;
            self.media
                .remove_picture(&crate::profile_pictures::contact_picture_name(&key));
        }
        // Like Android, the forward security sessions stay: the row does too, and their next
        // message would only be rejected. Android's exclusion is a question, for contacts from
        // the address book: asked or not, the next match would otherwise bring them back.
        self.set_sync_excluded(&key, true)?;
        let _ = self
            .media_events
            .send(crate::providers::CoreEvent::ContactsChanged);
        Ok(json!({}))
    }
}

impl Shared {
    /// Adding someone by hand undoes what deleting them put on the exclusion list. `true` if it
    /// did, for the other devices.
    pub(crate) fn sync_included_again(&self, identity: &str) -> Result<bool, ApiError> {
        let excluded = self
            .app_settings()?
            .sync_excluded_ids
            .iter()
            .any(|id| id == identity);
        if excluded {
            self.set_sync_excluded(identity, false)?;
        }
        Ok(excluded)
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
                .send(crate::providers::CoreEvent::MessageAdded { message });
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
    pub(crate) async fn link_email(&self, email: &str) -> Result<Value, ApiError> {
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
                self.store
                    .set_setting(crate::store::keys::LINKED_EMAIL_PENDING, "1")?;
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
    pub(crate) async fn check_email_link(&self) -> Result<bool, ApiError> {
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
        if !self.email_link_confirmed(&email)? {
            return Ok(false);
        }
        tracing::info!("e-mail link confirmed");
        let change = crate::app_multidevice::SyncChange::Profile { picture: false };
        self.reflect_change(change, &json!({})).await;
        Ok(true)
    }

    /// The directory says `asked` is linked. The address may have changed during the question, and
    /// then that answer says nothing about the one waiting now. Whether it was the one.
    fn email_link_confirmed(&self, asked: &str) -> Result<bool, ApiError> {
        if self.store.setting(keys::LINKED_EMAIL)?.as_deref() != Some(asked) {
            return Ok(false);
        }
        self.store.delete_setting(keys::LINKED_EMAIL_PENDING)?;
        Ok(true)
    }

    /// `linkMobile` (Android `linkWithMobileNumber` / `unlinkMobileNumber`). Only numbers that
    /// say their country: the SMS must not go to a number we guessed (ADR 0016).
    pub(crate) async fn link_mobile(&self, number: &str) -> Result<Value, ApiError> {
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
        &self,
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
    pub(crate) async fn call_mobile(&self) -> Result<Value, ApiError> {
        let (_, id) = self.pending_mobile()?;
        crate::directory::call_mobile(&self.https, &id).await?;
        self.profile_json()
    }

    /// `revocationStatus` (Android `checkRevocationKey`).
    pub(crate) async fn revocation_status(&self, cached: bool) -> Result<Value, ApiError> {
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
    pub(crate) async fn set_revocation_password(&self, password: &str) -> Result<Value, ApiError> {
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
            "dataImportUnfinished": self.store.setting(keys::DATA_IMPORT_UNFINISHED)?.is_some(),
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
}

impl App {
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

    /// `dismissDataImportHint`: the user is done with the backup's unfinished import.
    pub(crate) fn dismiss_data_import_hint(&mut self) -> Result<Value, ApiError> {
        self.store.delete_setting(keys::DATA_IMPORT_UNFINISHED)?;
        self.profile_json()
    }

    /// Not now: no more asking, and messages keep leaving contacts' stored name alone.
    pub(crate) fn decline_nickname(&mut self) -> Result<Value, ApiError> {
        if self.store.setting(keys::NICKNAME_UNANSWERED)?.is_some() {
            self.store.set_setting(keys::NICKNAME_UNANSWERED, "asked")?;
        }
        self.profile_json()
    }
}

impl Shared {
    /// The contacts' states and feature masks, in one `identity/check` (Android's
    /// `ContactUpdateWorker`). Every `checkInterval` the directory last gave, a day if it gave
    /// none, and without revoked IDs, which stay revoked; `force` (the user asked): now, and them
    /// too, as nothing else would ever correct one marked by mistake. Returns whether something
    /// changed.
    pub(crate) async fn refresh_contacts(&self, force: bool) -> Result<bool, ApiError> {
        let now = crate::messages::now_ms() / 1000;
        let setting = |key| -> Result<i64, ApiError> {
            Ok(self
                .store
                .setting(key)?
                .and_then(|value| value.parse::<i64>().ok())
                .unwrap_or(0))
        };
        let last = setting(keys::CONTACTS_REFRESHED_AT)?;
        let interval = contact_refresh_interval(setting(keys::CONTACTS_CHECK_INTERVAL)?);
        if !force && (now - last < interval || self.store.directory_resting()) {
            return Ok(false);
        }
        let ids = identities_to_check(&self.store.contacts()?, force);
        if ids.is_empty() {
            return Ok(false);
        }
        let checks = crate::directory::check_identities(&self.https, &ids)
            .await
            .map_err(|error| self.directory_error(error))?;
        let mut changed = false;
        for (id, check) in ids.iter().zip(checks.identities) {
            // Read again: the lookup took a while, and a rename or a deletion meanwhile stands.
            let Some(mut contact) = self.store.contact(id)? else {
                continue;
            };
            let before = contact.clone();
            let had_fs = contacts::has_fs_support(&contact);
            if contacts::apply_identity_check(&self.config, &mut contact, check) {
                let step = self.store.step()?;
                self.store.upsert_contact(&contact)?;
                crate::sync::queue_directory_change(&self.store, &before, &contact)?;
                step.commit()?;
                if had_fs && !contacts::has_fs_support(&contact) {
                    self.fs_support_lost(&contact)?;
                }
                changed = true;
            }
        }
        if changed {
            self.tell_connection(crate::connection::ToConnection::ReflectSync);
        }
        if checks.check_interval > 0 {
            self.store.set_setting(
                keys::CONTACTS_CHECK_INTERVAL,
                &checks.check_interval.to_string(),
            )?;
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
    pub(crate) async fn import_pending_contacts(&self) -> Result<bool, ApiError> {
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
                self.store.mark_picture_restored(&id)?;
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

    /// Ask up to `limit` of the contacts a restore left without a picture for one (Safe does not
    /// carry them), as far as there is reason to: a chat or a shared group, like Android's
    /// _Application Setup Steps_. The others stay marked, in case that changes. Called per round
    /// (connect, maintenance), so that the requests stay bounded; with nobody marked it is one
    /// query. Returns how many were queued.
    pub(crate) fn request_restored_pictures(&self, limit: usize) -> usize {
        let Ok(marked) = self.store.restored_pictures() else {
            return 0;
        };
        if marked.is_empty() {
            return 0;
        }
        let me = self.store.identity().ok().flatten();
        let me = me.as_ref().map_or("", |identity| &identity.threema_id);
        let Ok(solicited) = self.solicited_contacts(me) else {
            return 0;
        };
        let mut queued = 0;
        for id in marked.iter().filter(|id| solicited.contains(id)) {
            if queued >= limit {
                break;
            }
            if self.store.request_restored_picture(id).unwrap_or(false) {
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
}

impl Shared {
    /// `verifyQrCode`: a scanned `3mid:` code. The key must be the one we have (or, for someone
    /// new, the directory's): then the contact is _fully verified_ (and added if new). A code
    /// that doesn't match, or has expired, changes nothing.
    pub(crate) async fn verify_qr_code(&self, text: &str) -> Result<Value, ApiError> {
        let code = crate::qr::parse(text).ok_or(ApiError::QrInvalid)?;
        if code.is_expired(crate::messages::now_ms() / 1000) {
            return Err(ApiError::QrExpired);
        }
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        if own.threema_id == code.identity {
            return Err(ApiError::ContactIsSelf);
        }
        let known = self.store.contact(&code.identity)?;
        let new = known.is_none();
        let contact = match known {
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
        self.verify_with_key(contact, &code.public_key, new)
    }

    /// The key of a scanned code against the contact's (stored or just looked up, then stored
    /// only if it matches).
    fn verify_with_key(
        &self,
        mut contact: libthreema::model::contact::ContactInit,
        public_key: &[u8; 32],
        new: bool,
    ) -> Result<Value, ApiError> {
        if contacts::hex_bytes(&contact.public_key.to_string()).as_slice() != public_key {
            tracing::warn!("scanned QR code doesn't match the contact's public key");
            return Err(ApiError::QrMismatch);
        }
        contact.verification_level = pc::VerificationLevel::FullyVerified;
        contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
        self.store.upsert_contact(&contact)?;
        let to_the_others = if new {
            &self.new_contacts
        } else {
            &self.made_direct
        };
        to_the_others
            .borrow_mut()
            .push(contact.identity.to_string());
        Ok(self.contact_value(&contact))
    }

    /// What "Automatic" stands for here.
    fn automatic_address_book_country(
        &self,
    ) -> Result<Option<&'static crate::address_book::Country>, ApiError> {
        let linked = self.store.setting(keys::LINKED_MOBILE)?;
        Ok(crate::address_book::automatic_country(
            linked.as_deref(),
            &self.version.country,
        ))
    }

    /// The country of numbers that don't say their own: the user's choice, else automatic. A
    /// choice is checked when made, and a later version may list fewer countries.
    pub(crate) fn address_book_region(&self) -> Result<&'static str, ApiError> {
        let chosen = self
            .store
            .setting(keys::ADDRESS_BOOK_COUNTRY)?
            .and_then(|region| crate::address_book::country_of(&region));
        let country = match chosen {
            Some(country) => Some(country),
            None => self.automatic_address_book_country()?,
        };
        Ok(country.map_or("", |country| country.region))
    }
}

impl App {
    /// `addressBookCountry`: what the setting offers and has.
    pub(crate) fn address_book_country_json(&self) -> Result<Value, ApiError> {
        let countries: Vec<Value> = crate::address_book::COUNTRIES
            .iter()
            .map(|country| json!({ "region": country.region, "code": country.code }))
            .collect();
        Ok(json!({
            "chosen": self.store.setting(keys::ADDRESS_BOOK_COUNTRY)?.unwrap_or_default(),
            "automaticCode": self.automatic_address_book_country()?.map_or("", |c| c.code),
            "countries": countries,
        }))
    }

    /// `setAddressBookCountry`: empty is automatic.
    pub(crate) fn set_address_book_country(&mut self, region: &str) -> Result<Value, ApiError> {
        if region.is_empty() {
            self.store.delete_setting(keys::ADDRESS_BOOK_COUNTRY)?;
        } else {
            let country = crate::address_book::country_of(region)
                .ok_or_else(|| ApiError::Internal("unknown country".into()))?;
            self.store
                .set_setting(keys::ADDRESS_BOOK_COUNTRY, country.region)?;
        }
        self.address_book_country_json()
    }
}

impl Shared {
    /// `matchAddressBook`: find address book entries on Threema (hashes only) and make them
    /// contacts, named as in the address book (unless the user named them), server verified.
    pub(crate) async fn match_address_book(
        &self,
        entries: Vec<crate::address_book::Entry>,
    ) -> Result<Value, ApiError> {
        let own = self.store.identity()?.ok_or(ApiError::NoIdentity)?;
        let client_key = ClientKey::from(*own.client_key);
        let hashes = crate::address_book::hash_entries(&entries, self.address_book_region()?);
        if hashes.emails.is_empty() && hashes.phones.is_empty() {
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
        let email_list: Vec<String> = hashes.emails.keys().cloned().collect();
        let phone_list: Vec<String> = hashes.phones.keys().cloned().collect();
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
        let claims = crate::address_book::claims(&found, &hashes, &entries);
        let claims = self.claims_to_take(claims, &own.threema_id)?;
        // One request per hundred new people, not one per person: nothing else is dispatched
        // meanwhile. What came before a failure is used, and the failure is said afterwards.
        let mut missing = Vec::new();
        for claim in &claims {
            if self.store.contact(&claim.identity)?.is_none() {
                missing.push(claim.identity.clone());
            }
        }
        let mut looked_up = std::collections::HashMap::new();
        let mut failure = None;
        for chunk in missing.chunks(100) {
            match contacts::lookup(&self.https, &self.config, chunk).await {
                Ok(results) => looked_up.extend(results),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let (matched, added) = self.take_claims(&claims, looked_up)?;
        tracing::info!(matched, added, "address book matched");
        match failure {
            Some(error) => Err(self.directory_error(error)),
            None => Ok(json!({ "matched": matched, "added": added })),
        }
    }

    /// Makes contacts of the claims: the ones we have, and the new ones `looked_up` found. How
    /// many matched and how many are new in the contact list.
    fn take_claims(
        &self,
        claims: &[crate::address_book::Claim<'_>],
        mut looked_up: std::collections::HashMap<String, Lookup>,
    ) -> Result<(usize, usize), ApiError> {
        let same_key = |contact: &libthreema::model::contact::Contact, key: &[u8; 32]| {
            contacts::hex_bytes(&contact.public_key.to_string()).as_slice() == key
        };
        let (mut matched, mut added) = (0, 0);
        for hit in claims {
            let (contact, fresh) = match self.store.contact(&hit.identity)? {
                Some(mut contact) => {
                    if !same_key(&contact, &hit.public_key) {
                        tracing::warn!("address book match with a different public key, skipped");
                        if contacts::distrust_changed_key(&mut contact) {
                            self.store.upsert_contact(&contact)?;
                            self.made_direct.borrow_mut().push(hit.identity.clone());
                        }
                        continue;
                    }
                    (contact, false)
                }
                None => {
                    // Revoked, refused or not asked about for now: only this one is skipped.
                    let Some(Lookup::Found(contact)) = looked_up.remove(&hit.identity) else {
                        continue;
                    };
                    let mut contact = *contact;
                    if !same_key(&contact, &hit.public_key) {
                        tracing::warn!("address book match with a different public key, skipped");
                        continue;
                    }
                    contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
                    self.store.upsert_contact(&contact)?;
                    added += 1;
                    (contact, true)
                }
            };
            matched += 1;
            let change = self.take_from_address_book(contact, hit.entry)?;
            if change == AddressBookChange::Raised {
                added += 1;
            }
            // `handle` tells the other devices, as for someone made direct by writing to them.
            if fresh {
                self.new_contacts.borrow_mut().push(hit.identity.clone());
            } else if change != AddressBookChange::None {
                self.made_direct.borrow_mut().push(hit.identity.clone());
            }
        }
        Ok((matched, added))
    }

    /// Not the user themself, and not whoever they deleted or excluded: a deleted contact stays
    /// deleted, however often the address book says otherwise, until added by hand.
    fn claims_to_take<'a>(
        &self,
        mut claims: Vec<crate::address_book::Claim<'a>>,
        own: &str,
    ) -> Result<Vec<crate::address_book::Claim<'a>>, ApiError> {
        let excluded = self.app_settings()?.sync_excluded_ids;
        claims.retain(|claim| claim.identity != own && !excluded.contains(&claim.identity));
        Ok(claims)
    }

    /// A contact the address book matched: named from it, server verified, in the contact list.
    /// The name follows the entry while it came from there (`Imported`, as on Android); what the
    /// user wrote or cleared (`Custom`) stays. A name from before we kept track (`Initial`) can't be
    /// told from the user's, so it stays too.
    fn take_from_address_book(
        &self,
        mut contact: libthreema::model::contact::Contact,
        entry: &crate::address_book::Entry,
    ) -> Result<AddressBookChange, ApiError> {
        let mut change = AddressBookChange::None;
        let unnamed = contact.first_name.as_deref().unwrap_or("").is_empty()
            && contact.last_name.as_deref().unwrap_or("").is_empty();
        let name_follows = match contact.sync_state {
            pc::SyncState::Imported => true,
            pc::SyncState::Initial => unnamed,
            pc::SyncState::Custom => false,
        };
        let first_name = clean(Some(entry.first_name.clone()));
        let last_name = clean(Some(entry.last_name.clone()));
        let entry_has_name = first_name.is_some() || last_name.is_some();
        if name_follows
            && entry_has_name
            && (contact.first_name != first_name
                || contact.last_name != last_name
                || contact.sync_state != pc::SyncState::Imported)
        {
            contact.first_name = first_name;
            contact.last_name = last_name;
            contact.sync_state = pc::SyncState::Imported;
            change = AddressBookChange::Updated;
        }
        if contact.verification_level == pc::VerificationLevel::Unverified {
            contact.verification_level = pc::VerificationLevel::ServerVerified;
            change = AddressBookChange::Updated;
        }
        if contact.acquaintance_level != pc::AcquaintanceLevel::Direct {
            contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
            change = AddressBookChange::Raised;
        }
        if change != AddressBookChange::None {
            self.store.upsert_contact(&contact)?;
        }
        Ok(change)
    }
}

/// What an address book match did to a contact we had.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AddressBookChange {
    None,
    Updated,
    /// New in the contact list.
    Raised,
}

#[cfg(test)]
mod contact_refresh_tests {
    use libthreema::protobuf::d2d_sync::contact as pc;

    use super::*;

    #[test]
    fn the_directory_says_how_often_and_revoked_ids_are_left_out() {
        assert_eq!(contact_refresh_interval(0), 86_400);
        assert_eq!(contact_refresh_interval(-1), 86_400);
        assert_eq!(contact_refresh_interval(604_800), 604_800);

        let mut revoked = crate::contacts::test_contact("BBBBBBBB");
        revoked.activity_state = pc::ActivityState::Invalid;
        let mut inactive = crate::contacts::test_contact("CCCCCCCC");
        inactive.activity_state = pc::ActivityState::Inactive;
        let contacts = [revoked, inactive, crate::contacts::test_contact("DDDDDDDD")];
        assert_eq!(
            identities_to_check(&contacts, false),
            ["CCCCCCCC", "DDDDDDDD"]
        );
        assert_eq!(
            identities_to_check(&contacts, true),
            ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"]
        );
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
    async fn an_answer_about_the_old_address_confirms_no_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        for (key, value) in [
            (crate::store::keys::LINKED_EMAIL, "new@example.com"),
            (crate::store::keys::LINKED_EMAIL_PENDING, "1"),
        ] {
            app.store.set_setting(key, value).unwrap();
        }
        assert!(!app.email_link_confirmed("old@example.com").unwrap());
        assert!(
            app.store
                .setting(crate::store::keys::LINKED_EMAIL_PENDING)
                .unwrap()
                .is_some()
        );
        assert!(app.email_link_confirmed("new@example.com").unwrap());
        assert!(
            app.store
                .setting(crate::store::keys::LINKED_EMAIL_PENDING)
                .unwrap()
                .is_none()
        );
    }

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
        let app = app(dir.path());
        let contact = crate::contacts::test_contact("BBBBBBBB");
        assert!(matches!(
            app.verify_with_key(contact, &[1; 32], true),
            Err(crate::app::ApiError::QrMismatch)
        ));
        assert!(app.store.contact("BBBBBBBB").unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_new_contact_with_the_right_key_is_added_verified() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let contact = crate::contacts::test_contact("BBBBBBBB");
        app.verify_with_key(contact, &[9; 32], true).unwrap();
        assert_eq!(*app.new_contacts.borrow(), ["BBBBBBBB"]);
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
        // Its page has nothing to delete while the group needs it.
        let shown = call(&mut app, json!({"cmd": "contact", "identity": "BBBBBBBB"})).await;
        assert_eq!(shown["result"]["hidden"], true);
        assert_eq!(shown["result"]["neededByGroup"], true);
    }

    fn with_a_chat(app: &App, identity: &str) {
        app.store
            .upsert_contact(&crate::contacts::test_contact(identity))
            .unwrap();
        app.store
            .insert_incoming(identity, 7, "text", "hello", 1000)
            .unwrap();
    }

    fn unread_total(app: &App) -> i64 {
        app.conversations_json(false, false).unwrap()["unreadTotal"]
            .as_i64()
            .unwrap()
    }

    fn listed_chats(app: &App) -> Vec<String> {
        app.conversations_json(false, false).unwrap()["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|chat| chat["identity"].as_str().unwrap().to_owned())
            .collect()
    }

    /// What is left of someone with a chat, a picture and choices of their own when deleted.
    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_contact_keeps_the_row_and_takes_the_chat_along() {
        use crate::profile_pictures::contact_picture_name;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        let picture = contact_picture_name("BBBBBBBB");
        app.media.write_picture(&picture, b"picture").unwrap();
        app.store.set_contact_picture("BBBBBBBB", Some(5)).unwrap();
        app.store.mark_picture_restored("BBBBBBBB").unwrap();
        app.store.set_pinned("BBBBBBBB", true).unwrap();
        app.store
            .set_chat_notifications(
                "BBBBBBBB",
                Some((crate::chat_notifications::Policy::Never, None)),
            )
            .unwrap();
        app.store.set_chat_tone("BBBBBBBB", Some("a.ogg")).unwrap();
        app.store.set_setting("draft:BBBBBBBB", "unsent").unwrap();
        assert_eq!(unread_total(&app), 1);

        let reply = call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(reply["ok"], true, "{reply}");

        let kept = app.store.contact("BBBBBBBB").unwrap().expect("kept");
        assert_eq!(
            kept.acquaintance_level,
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted
        );
        assert!(listed_chats(&app).is_empty());
        assert_eq!(unread_total(&app), 0);
        assert_eq!(app.store.setting("draft:BBBBBBBB").unwrap(), None);
        assert!(app.store.pinned_chats().unwrap().is_empty());
        assert_eq!(app.store.chat_notifications("BBBBBBBB").unwrap(), None);
        assert!(app.store.chat_tones().unwrap().is_empty());
        assert_eq!(app.store.contact_picture_version("BBBBBBBB").unwrap(), 0);
        assert!(app.media.read_picture(&picture).unwrap().is_none());
        // Nobody deleted is asked for a picture.
        assert!(!app.store.request_restored_picture("BBBBBBBB").unwrap());
        let shown = call(&mut app, json!({"cmd": "contact", "identity": "BBBBBBBB"})).await;
        assert_eq!(shown["result"]["hidden"], true);
        assert_eq!(shown["result"]["neededByGroup"], false);

        // And back, with what it was known by.
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let added = call(
            &mut app,
            json!({"cmd": "addContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(added["result"]["hidden"], false);
        assert_eq!(
            app.contacts_json().unwrap()["contacts"][0]["identity"],
            "BBBBBBBB"
        );
    }

    /// What was queued for them doesn't go out after they are deleted; a group's own does.
    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_contact_takes_what_was_queued_for_them_along() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        app.store
            .outbox_add("BBBBBBBB", 1, crate::reactions::OUTBOX_EDIT, &json!({}))
            .unwrap();
        app.store
            .outbox_add("CCCCCCCC", 2, crate::reactions::OUTBOX_EDIT, &json!({}))
            .unwrap();
        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        let left: Vec<String> = app
            .store
            .outbox()
            .unwrap()
            .into_iter()
            .map(|entry| entry.conversation)
            .collect();
        assert_eq!(left, ["CCCCCCCC"]);
    }

    /// As on Android: the row stays, so their next message has a session to come in on.
    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_contact_keeps_the_forward_security_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        app.store
            .conn
            .execute(
                "INSERT INTO fs_sessions (peer, session_id, my_ephemeral_public, last_outgoing_at)
                 VALUES ('BBBBBBBB', x'00', x'00', 0)",
                [],
            )
            .unwrap();
        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        let sessions: i64 = app
            .store
            .conn
            .query_row(
                "SELECT count(*) FROM fs_sessions WHERE peer = 'BBBBBBBB'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sessions, 1);
    }

    /// Numbers without a country code: the user's choice, else the linked number, else the locale.
    #[tokio::test(flavor = "current_thread")]
    async fn the_address_book_country_goes_by_choice_then_linked_number_then_locale() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        // The tests' locale is Switzerland.
        assert_eq!(app.address_book_region().unwrap(), "CH");
        let reply = call(&mut app, json!({"cmd": "addressBookCountry"})).await;
        assert_eq!(reply["result"]["chosen"], "");
        assert_eq!(reply["result"]["automaticCode"], "41");
        assert!(
            reply["result"]["countries"]
                .as_array()
                .unwrap()
                .contains(&json!({"region": "DE", "code": "49"}))
        );

        app.store
            .set_setting(crate::store::keys::LINKED_MOBILE, "436641234567")
            .unwrap();
        assert_eq!(app.address_book_region().unwrap(), "AT");
        let reply = call(&mut app, json!({"cmd": "addressBookCountry"})).await;
        assert_eq!(reply["result"]["automaticCode"], "43");

        let reply = call(
            &mut app,
            json!({"cmd": "setAddressBookCountry", "country": "IT"}),
        )
        .await;
        assert_eq!(reply["result"]["chosen"], "IT");
        assert_eq!(reply["result"]["automaticCode"], "43");
        assert_eq!(app.address_book_region().unwrap(), "IT");

        let reply = call(
            &mut app,
            json!({"cmd": "setAddressBookCountry", "country": "XX"}),
        )
        .await;
        assert_eq!(reply["ok"], false);
        assert_eq!(app.address_book_region().unwrap(), "IT");

        call(
            &mut app,
            json!({"cmd": "setAddressBookCountry", "country": ""}),
        )
        .await;
        assert_eq!(app.address_book_region().unwrap(), "AT");
    }

    /// The next match doesn't bring someone deleted back, and adding them by hand ends that.
    #[tokio::test(flavor = "current_thread")]
    async fn someone_deleted_stays_out_of_the_address_book_match_until_added_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let excluded = |app: &App| app.app_settings().unwrap().sync_excluded_ids;
        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(excluded(&app), ["BBBBBBBB"]);
        let entry = crate::address_book::Entry {
            first_name: "Anna".into(),
            last_name: String::new(),
            emails: Vec::new(),
            phones: Vec::new(),
        };
        let claim = || crate::address_book::Claim {
            identity: "BBBBBBBB".into(),
            public_key: [7; 32],
            entry: &entry,
        };
        let taken = |app: &App| app.claims_to_take(vec![claim()], "ABCD1234").unwrap().len();
        assert_eq!(taken(&app), 0);
        call(
            &mut app,
            json!({"cmd": "addContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert!(excluded(&app).is_empty());
        assert_eq!(taken(&app), 1);
    }

    /// New people come from one lookup: a revoked ID or a wrong key costs only that one, and
    /// someone the lookup didn't get to is left for the next sync.
    #[tokio::test(flavor = "current_thread")]
    async fn new_people_in_the_address_book_are_added_from_the_lookup() {
        use libthreema::protobuf::d2d_sync::contact::{AcquaintanceLevel, VerificationLevel};

        use crate::contacts::{Lookup, test_contact};
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let entry = crate::address_book::Entry {
            first_name: "Anna".into(),
            last_name: String::new(),
            emails: Vec::new(),
            phones: Vec::new(),
        };
        let key_of = |id: &str| -> [u8; 32] {
            crate::contacts::hex_bytes(&test_contact(id).public_key.to_string())
                .try_into()
                .unwrap()
        };
        let claim = |id: &str, public_key| crate::address_book::Claim {
            identity: id.into(),
            public_key,
            entry: &entry,
        };
        let claims = [
            claim("AAAAAAAA", key_of("AAAAAAAA")),
            claim("BBBBBBBB", key_of("BBBBBBBB")),
            claim("CCCCCCCC", [0xee; 32]),
            claim("DDDDDDDD", key_of("DDDDDDDD")),
        ];
        let looked_up = [
            (
                "AAAAAAAA",
                Lookup::Found(Box::new(test_contact("AAAAAAAA"))),
            ),
            ("BBBBBBBB", Lookup::Invalid),
            (
                "CCCCCCCC",
                Lookup::Found(Box::new(test_contact("CCCCCCCC"))),
            ),
        ]
        .into_iter()
        .map(|(id, result)| (id.to_owned(), result))
        .collect();
        assert_eq!(app.take_claims(&claims, looked_up).unwrap(), (1, 1));

        let added = app.store.contact("AAAAAAAA").unwrap().unwrap();
        assert_eq!(added.acquaintance_level, AcquaintanceLevel::Direct);
        assert_eq!(added.verification_level, VerificationLevel::ServerVerified);
        assert_eq!(added.first_name.as_deref(), Some("Anna"));
        for left_out in ["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"] {
            assert!(app.store.contact(left_out).unwrap().is_none(), "{left_out}");
        }
        assert_eq!(*app.new_contacts.borrow(), ["AAAAAAAA"]);
        assert!(app.made_direct.borrow().is_empty());

        // Once they are contacts, no lookup is needed, and nothing is new.
        let again = [claim("AAAAAAAA", key_of("AAAAAAAA"))];
        let nothing = std::collections::HashMap::new();
        assert_eq!(app.take_claims(&again, nothing).unwrap(), (1, 0));
    }

    /// A contact whose key isn't the one the directory has now: no more than one dot of trust.
    #[tokio::test(flavor = "current_thread")]
    async fn a_stored_contact_with_another_key_is_not_taken_from_the_address_book() {
        use libthreema::protobuf::d2d_sync::contact::VerificationLevel;
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.verification_level = VerificationLevel::ServerVerified;
        app.store.upsert_contact(&contact).unwrap();
        let entry = crate::address_book::Entry {
            first_name: "Anna".into(),
            last_name: String::new(),
            emails: Vec::new(),
            phones: Vec::new(),
        };
        let claims = [crate::address_book::Claim {
            identity: "BBBBBBBB".into(),
            public_key: [0xee; 32],
            entry: &entry,
        }];
        let nothing = std::collections::HashMap::new();
        assert_eq!(app.take_claims(&claims, nothing).unwrap(), (0, 0));
        let stored = app.store.contact("BBBBBBBB").unwrap().unwrap();
        assert_eq!(stored.verification_level, VerificationLevel::Unverified);
        assert!(stored.first_name.is_none());
    }

    /// Someone out of the contact list who isn't excluded (a group member): back in it, and it
    /// says so, for the count and the other devices (`match_address_book`).
    #[tokio::test(flavor = "current_thread")]
    async fn an_address_book_match_says_when_it_raises_someone() {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        app.store.upsert_contact(&contact).unwrap();
        let entry = crate::address_book::Entry {
            first_name: "Anna".into(),
            last_name: String::new(),
            emails: Vec::new(),
            phones: Vec::new(),
        };
        let change = app.take_from_address_book(contact, &entry).unwrap();
        assert!(change == super::AddressBookChange::Raised);
        let stored = app.store.contact("BBBBBBBB").unwrap().unwrap();
        assert_eq!(stored.acquaintance_level, AcquaintanceLevel::Direct);
        assert_eq!(stored.first_name.as_deref(), Some("Anna"));
        // Nothing left to do the second time.
        let change = app.take_from_address_book(stored, &entry).unwrap();
        assert!(change == super::AddressBookChange::None);
    }

    /// A name from the address book follows it; a name the user wrote or cleared doesn't.
    #[tokio::test(flavor = "current_thread")]
    async fn a_name_from_the_address_book_follows_it_until_the_user_changes_it() {
        use libthreema::protobuf::d2d_sync::contact::SyncState;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let named = |first: &str, last: &str| crate::address_book::Entry {
            first_name: first.into(),
            last_name: last.into(),
            emails: Vec::new(),
            phones: Vec::new(),
        };
        let take = |app: &App, entry: &crate::address_book::Entry| {
            let contact = app.store.contact("BBBBBBBB").unwrap().unwrap();
            app.take_from_address_book(contact, entry).unwrap()
        };
        let name = |app: &App| {
            let contact = app.store.contact("BBBBBBBB").unwrap().unwrap();
            (contact.first_name, contact.last_name, contact.sync_state)
        };
        let anna = |last: &str, state| (Some("Anna".to_owned()), Some(last.to_owned()), state);
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.verification_level =
            libthreema::protobuf::d2d_sync::contact::VerificationLevel::ServerVerified;
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::Direct;
        app.store.upsert_contact(&contact).unwrap();

        // Named from the entry, and from then on following it.
        assert!(take(&app, &named("Anna", "Muster")) == super::AddressBookChange::Updated);
        assert_eq!(name(&app), anna("Muster", SyncState::Imported));
        assert!(take(&app, &named("Anna", "Muster")) == super::AddressBookChange::None);
        assert!(take(&app, &named(" Anna ", "Beispiel")) == super::AddressBookChange::Updated);
        assert_eq!(name(&app), anna("Beispiel", SyncState::Imported));
        // An entry without a name takes nothing away.
        assert!(take(&app, &named("", "  ")) == super::AddressBookChange::None);
        assert_eq!(name(&app), anna("Beispiel", SyncState::Imported));

        // The user's own name stays, and so does clearing it.
        call(
            &mut app,
            json!({"cmd": "updateContact", "identity": "BBBBBBBB",
                   "firstName": "Anni", "lastName": ""}),
        )
        .await;
        assert!(take(&app, &named("Anna", "Muster")) == super::AddressBookChange::None);
        assert_eq!(
            name(&app),
            (Some("Anni".to_owned()), None, SyncState::Custom)
        );
        call(
            &mut app,
            json!({"cmd": "updateContact", "identity": "BBBBBBBB",
                   "firstName": "", "lastName": ""}),
        )
        .await;
        assert!(take(&app, &named("Anna", "Muster")) == super::AddressBookChange::None);
        assert_eq!(name(&app), (None, None, SyncState::Custom));

        // A name from before this was kept track of stays.
        let mut contact = app.store.contact("BBBBBBBB").unwrap().unwrap();
        contact.first_name = Some("Bea".into());
        contact.sync_state = SyncState::Initial;
        app.store.upsert_contact(&contact).unwrap();
        assert!(take(&app, &named("Anna", "Muster")) == super::AddressBookChange::None);
        assert_eq!(
            name(&app),
            (Some("Bea".to_owned()), None, SyncState::Initial)
        );
    }

    /// The group pages go on showing someone's picture, whoever else they are to us.
    #[tokio::test(flavor = "current_thread")]
    async fn a_group_member_deleted_as_a_contact_loses_the_chat_and_keeps_the_picture() {
        use crate::profile_pictures::contact_picture_name;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
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
                left: true,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        let picture = contact_picture_name("BBBBBBBB");
        app.media.write_picture(&picture, b"picture").unwrap();
        app.store.set_contact_picture("BBBBBBBB", Some(5)).unwrap();

        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;

        assert!(!listed_chats(&app).contains(&"BBBBBBBB".to_owned()));
        assert!(app.store.contact_picture_version("BBBBBBBB").unwrap() > 0);
        assert!(app.media.read_picture(&picture).unwrap().is_some());
        // A group we left needs nothing of them: this contact can go on being deleted.
        let shown = call(&mut app, json!({"cmd": "contact", "identity": "BBBBBBBB"})).await;
        assert_eq!(shown["result"]["neededByGroup"], false);
    }

    /// A hidden private chat isn't changed from outside, and no error says there is one.
    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_contact_leaves_a_hidden_private_chat_to_be_opened_later() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        app.store.set_private("BBBBBBBB", true).unwrap();
        app.unlocked.set(false);

        let reply = call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(listed_chats(&app).is_empty(), "stays hidden");
        let read = json!({"cmd": "messages", "identity": "BBBBBBBB"});
        let hidden = call(&mut app, read.clone()).await;
        assert_eq!(hidden["error"]["kind"], "locked");

        app.unlocked.set(true);
        let opened = call(&mut app, read).await;
        assert_eq!(opened["result"]["messages"][0]["text"], "hello");
        let deleted = call(
            &mut app,
            json!({"cmd": "deleteConversation", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(deleted["ok"], true, "{deleted}");
        assert!(listed_chats(&app).is_empty());
    }

    /// Older versions removed the row and left the chat: it can be read, marked read, emptied and
    /// deleted, and stays hidden if it is private.
    #[tokio::test(flavor = "current_thread")]
    async fn a_chat_whose_contact_row_is_gone_stays_usable() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        app.store.delete_contact("BBBBBBBB").unwrap();
        let read = json!({"cmd": "messages", "identity": "BBBBBBBB"});
        let mark = json!({"cmd": "markRead", "identity": "BBBBBBBB"});
        let empty = json!({"cmd": "emptyConversation", "identity": "BBBBBBBB"});
        let delete = json!({"cmd": "deleteConversation", "identity": "BBBBBBBB"});

        app.store.set_private("BBBBBBBB", true).unwrap();
        app.unlocked.set(false);
        for request in [&read, &mark, &empty, &delete] {
            let reply = call(&mut app, request.clone()).await;
            assert_eq!(reply["error"]["kind"], "locked", "{request}");
        }
        app.store.set_private("BBBBBBBB", false).unwrap();
        app.unlocked.set(true);
        assert_eq!(unread_total(&app), 1);

        let shown = call(&mut app, read).await;
        assert_eq!(shown["result"]["messages"][0]["text"], "hello");
        let marked = call(&mut app, mark).await;
        assert_eq!(marked["result"]["read"], 1);
        assert_eq!(unread_total(&app), 0);
        assert_eq!(call(&mut app, empty).await["ok"], true);
        assert_eq!(listed_chats(&app), ["BBBBBBBB"]);
        assert_eq!(call(&mut app, delete).await["ok"], true);
        assert!(listed_chats(&app).is_empty());
        // Writing takes a contact, which can be added back.
        let refused = call(
            &mut app,
            json!({"cmd": "sendText", "identity": "BBBBBBBB", "text": "hello"}),
        )
        .await;
        assert_eq!(refused["error"]["kind"], "contactNotFound");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn someone_deleted_is_found_by_a_list_they_are_on() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        with_a_chat(&app, "BBBBBBBB");
        let list = app
            .store
            .create_distribution_list("Crew", vec!["BBBBBBBB".into()])
            .unwrap();
        call(
            &mut app,
            json!({"cmd": "deleteContact", "identity": "BBBBBBBB"}),
        )
        .await;
        assert_eq!(app.list_receivers(&list).unwrap(), ["BBBBBBBB"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_contact_list_sorts_accents_with_their_letter_and_the_rest_at_the_end() {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        for (id, name) in [
            ("BBBBBBBB", "Zoe"),
            ("CCCCCCCC", "Özlem"),
            ("DDDDDDDD", "😀 Party"),
            ("EEEEEEEE", "42 Crew"),
            ("FFFFFFFF", "Ärne"),
            ("GGGGGGGG", "Arne"),
            ("HHHHHHHH", "Oskar"),
        ] {
            let mut contact = crate::contacts::test_contact(id);
            contact.acquaintance_level = AcquaintanceLevel::Direct;
            contact.first_name = Some(name.into());
            app.store.upsert_contact(&contact).unwrap();
        }
        let contacts = app.contacts_json().unwrap();
        let listed: Vec<(&str, &str)> = contacts["contacts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["firstName"].as_str().unwrap(),
                    c["section"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("Arne", "A"),
                ("Ärne", "A"),
                ("Oskar", "O"),
                ("Özlem", "O"),
                ("Zoe", "Z"),
                ("42 Crew", "#"),
                ("😀 Party", "#"),
            ]
        );
    }

    /// One gate for showing a contact's picture, for whichever way it reached the device: the
    /// lists, the pages and the UI's reader, while the core keeps it. Our own and groups' stay.
    #[tokio::test(flavor = "current_thread")]
    async fn profile_pictures_off_hides_them_everywhere_and_keeps_them() {
        use crate::profile_pictures::{OWN_PICTURE, contact_picture_name};
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level = AcquaintanceLevel::Direct;
        app.store.upsert_contact(&contact).unwrap();
        app.store
            .insert_incoming("BBBBBBBB", 7, "text", "hello", 1000)
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 0x1234,
        };
        let group = crate::groups::Group {
            key: crate::groups::key(&identity),
            identity,
            name: "Crew".into(),
            members: vec!["BBBBBBBB".into()],
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let name = contact_picture_name("BBBBBBBB");
        // As a picture synced from another device or one sent by the contact ends up.
        app.media.write_picture(&name, b"theirs").unwrap();
        app.media.write_picture(OWN_PICTURE, b"ours").unwrap();
        app.media.write_picture(&group.key, b"the group's").unwrap();
        app.store.set_contact_picture("BBBBBBBB", Some(5)).unwrap();

        let versions = |app: &App| -> Vec<i64> {
            let contact = app.contact_json("BBBBBBBB").unwrap();
            let list = app.contacts_json().unwrap();
            let chats = app.conversations_json(false, false).unwrap();
            let chat = chats["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .find(|chat| chat["identity"] == "BBBBBBBB")
                .unwrap()
                .clone();
            let members = app.group_json(&group).unwrap();
            [
                &contact["pictureVersion"],
                &list["contacts"][0]["pictureVersion"],
                &chat["pictureVersion"],
                &members["members"][0]["pictureVersion"],
            ]
            .iter()
            .map(|version| version.as_i64().unwrap())
            .collect()
        };
        assert!(versions(&app).iter().all(|version| *version > 0));
        assert!(app.media.read_picture_for_ui(&name).unwrap().is_some());

        let off = json!({"cmd": "setSetting", "name": "showProfilePictures", "value": false});
        let reply = call(&mut app, off).await;
        assert_eq!(reply["result"]["showProfilePictures"], false);

        assert_eq!(versions(&app), [0, 0, 0, 0]);
        assert!(app.media.read_picture_for_ui(&name).unwrap().is_none());
        assert!(app.media.read_picture(&name).unwrap().is_some(), "kept");
        assert!(app.store.contact_picture_version("BBBBBBBB").unwrap() > 0);
        let own = app.media.read_picture_for_ui(OWN_PICTURE).unwrap();
        assert!(own.is_some(), "our own picture");
        let groups = app.media.read_picture_for_ui(&group.key).unwrap();
        assert!(groups.is_some(), "a group's picture");

        // Also after a restart, and back.
        app.apply_picture_gate();
        assert!(app.media.read_picture_for_ui(&name).unwrap().is_none());
        let on = json!({"cmd": "setSetting", "name": "showProfilePictures", "value": true});
        call(&mut app, on).await;
        assert!(versions(&app).iter().all(|version| *version > 0));
        assert!(app.media.read_picture_for_ui(&name).unwrap().is_some());
    }

    /// Not shown, so nobody is asked; nothing but the switch stands in the way afterwards.
    #[tokio::test(flavor = "current_thread")]
    async fn pictures_are_not_asked_for_while_not_shown() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store.mark_picture_restored("BBBBBBBB").unwrap();
        let off = json!({"cmd": "setSetting", "name": "showProfilePictures", "value": false});
        call(&mut app, off).await;
        assert!(!app.store.request_restored_picture("BBBBBBBB").unwrap());
        assert!(app.store.outbox().unwrap().is_empty());
        let on = json!({"cmd": "setSetting", "name": "showProfilePictures", "value": true});
        call(&mut app, on).await;
        assert!(app.store.request_restored_picture("BBBBBBBB").unwrap());
    }

    /// Whoever a restore brought with a chat is asked once, in bounded rounds; nobody else, however
    /// long they go without a picture.
    #[tokio::test(flavor = "current_thread")]
    async fn a_round_asks_only_restored_contacts_with_a_chat() {
        let dir = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let requests = |app: &App| {
            app.store
                .outbox()
                .unwrap()
                .iter()
                .filter(|e| e.kind == crate::profile_pictures::OUTBOX_REQUEST_PICTURE)
                .count()
        };
        // In a chat, but not restored.
        with_a_chat(&app, "BBBBBBBB");
        // Restored, nothing to talk about yet.
        app.store
            .upsert_contact(&crate::contacts::test_contact("CCCCCCCC"))
            .unwrap();
        app.store.mark_picture_restored("CCCCCCCC").unwrap();
        // Restored and in a chat.
        for id in ["DDDDDDDD", "EEEEEEEE", "FFFFFFFF"] {
            with_a_chat(&app, id);
            app.store.mark_picture_restored(id).unwrap();
        }
        assert_eq!(app.request_restored_pictures(2), 2);
        assert_eq!(requests(&app), 2);
        assert_eq!(app.request_restored_pictures(2), 1);
        assert_eq!(requests(&app), 3);
        assert_eq!(app.request_restored_pictures(2), 0);
        assert_eq!(requests(&app), 3);
        // A chat begins with the one still marked.
        app.store
            .insert_incoming("CCCCCCCC", 8, "text", "hello", 1000)
            .unwrap();
        assert_eq!(app.request_restored_pictures(2), 1);
        assert_eq!(requests(&app), 4);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn hiding_inactive_ids_hides_revoked_ones_too() {
        use libthreema::protobuf::d2d_sync::contact::{AcquaintanceLevel, ActivityState};
        let dir = tempfile::tempdir().unwrap();
        let mut app = app(dir.path());
        for (id, state) in [
            ("BBBBBBBB", ActivityState::Active),
            ("CCCCCCCC", ActivityState::Inactive),
            ("DDDDDDDD", ActivityState::Invalid),
        ] {
            let mut contact = crate::contacts::test_contact(id);
            contact.acquaintance_level = AcquaintanceLevel::Direct;
            contact.activity_state = state;
            app.store.upsert_contact(&contact).unwrap();
        }
        let listed = |app: &App| -> Vec<String> {
            app.contacts_json().unwrap()["contacts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["identity"].as_str().unwrap().to_owned())
                .collect()
        };
        assert_eq!(listed(&app).len(), 3);
        app.set_display_settings(None, None, Some(false)).unwrap();
        assert_eq!(listed(&app), ["BBBBBBBB"]);
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
