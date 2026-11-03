// SPDX-License-Identifier: AGPL-3.0-only
//! Group commands (create, rename, members, leave, disband) and fetching unknown members.
//! The submit steps follow csp.struct.yml: the creator sends `group-setup` to all current and
//! removed members, then name and picture to new ones; members send `group-leave` to everyone.

use std::rc::Rc;

use libthreema::{
    common::{GroupIdentity, ThreemaId},
    protobuf::d2d_sync::contact as pc,
};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    connection::ToConnection,
    contacts::{self, Lookup},
    groups::{
        self, Group, GroupPicture, OUTBOX_GROUP_LEAVE, OUTBOX_GROUP_NAME, OUTBOX_GROUP_SETUP,
    },
    media::{self, MediaPart},
    messages::now_ms,
    providers::CoreEvent,
};

/// Longest group name (Threema Android limits the input to 256 bytes).
const NAME_LENGTH_MAX: usize = 256;

/// The name a group or distribution list goes by in lists: its own name, else its members'
/// names. `display_name` looks a member up — the chat list hands in a map it read in one go.
pub(crate) fn title(
    name: &str,
    members: &[String],
    display_name: &dyn Fn(&str) -> String,
) -> String {
    if !name.is_empty() {
        return name.to_owned();
    }
    members
        .iter()
        .map(|member| display_name(member))
        .collect::<Vec<_>>()
        .join(", ")
}

impl App {
    pub(crate) fn user(&self) -> Result<String, ApiError> {
        Ok(self
            .store
            .identity()?
            .ok_or(ApiError::NoIdentity)?
            .threema_id)
    }

    pub(crate) fn display_name_of(&self, identity: &str) -> String {
        self.store
            .contact(identity)
            .ok()
            .flatten()
            .map_or_else(|| identity.to_owned(), |c| contacts::display_name(&c))
    }

    pub(crate) fn group_json(&self, group: &Group) -> Result<Value, ApiError> {
        let user = self.user()?;
        let mut json = group.to_json(&user, |m| self.display_name_of(m));
        // The members' pictures, for the group page's member list.
        if let Some(members) = json["members"].as_array_mut() {
            for member in members {
                let identity = member["identity"].as_str().unwrap_or_default().to_owned();
                member["pictureVersion"] =
                    json!(self.store.contact_picture_version(&identity).unwrap_or(0));
            }
        }
        json["notifications"] = self.store.chat_notifications_json(&group.key);
        json["pinned"] = self
            .store
            .pinned_chats()
            .is_ok_and(|pinned| pinned.contains(&group.key))
            .into();
        Ok(json)
    }

    /// A group's name for lists: its name, else its members' names.
    pub(crate) fn group_title(&self, group: &Group) -> String {
        title(&group.name, &group.members, &|m| self.display_name_of(m))
    }

    pub(crate) fn groups_json(&self) -> Result<Value, ApiError> {
        let mut list = Vec::new();
        for group in self.store.groups()? {
            let mut value = self.group_json(&group)?;
            value["title"] = json!(self.group_title(&group));
            list.push(value);
        }
        Ok(json!({ "groups": list }))
    }

    pub(crate) fn group_info(&self, key: &str) -> Result<Value, ApiError> {
        let group = self.store.group(key)?.ok_or(ApiError::GroupNotFound)?;
        let mut value = self.group_json(&group)?;
        value["title"] = json!(self.group_title(&group));
        Ok(value)
    }

    /// Our group with `key`, checking we created it and are still in it.
    fn own_group(&self, key: &str) -> Result<(Group, String), ApiError> {
        let group = self.store.group(key)?.ok_or(ApiError::GroupNotFound)?;
        let user = self.user()?;
        if group.identity.creator_identity.to_string() != user {
            return Err(ApiError::NotGroupCreator);
        }
        if group.left {
            return Err(ApiError::GroupLeft);
        }
        Ok((group, user))
    }

    /// Valid member identities: known contacts, without ourselves and duplicates.
    pub(crate) fn member_list(
        &self,
        members: &[String],
        user: &str,
    ) -> Result<Vec<String>, ApiError> {
        let mut list: Vec<String> = Vec::new();
        for member in members {
            let id = contacts::parse_identity(member)
                .ok_or(ApiError::IdentityFormat)?
                .to_string();
            if id == user || list.contains(&id) {
                continue;
            }
            if self.store.contact(&id)?.is_none() {
                return Err(ApiError::ContactNotFound);
            }
            list.push(id);
        }
        Ok(list)
    }

    pub(crate) fn check_name(name: &str) -> Result<String, ApiError> {
        let name = name.trim().to_owned();
        if name.len() > NAME_LENGTH_MAX {
            return Err(ApiError::MessageTooLong);
        }
        Ok(name)
    }

    /// `createGroup`: a new group with us as creator.
    pub(crate) fn create_group(
        &mut self,
        name: &str,
        members: &[String],
    ) -> Result<Value, ApiError> {
        let user = self.user()?;
        let name = Self::check_name(name)?;
        let members = self.member_list(members, &user)?;
        let identity = GroupIdentity {
            creator_identity: ThreemaId::try_from(user.as_str())
                .map_err(|e| ApiError::Internal(e.to_string()))?,
            group_id: rand::random(),
        };
        let group = Group {
            key: groups::key(&identity),
            identity,
            name,
            members,
            left: false,
            created_at: now_ms(),
            picture: None,
            picture_version: 0,
        };
        self.store.upsert_group(&group)?;
        groups::send_state(&self.store, &group, &group.members)?;
        self.store.insert_status(&group.key, "groupCreated")?;
        self.tell_connection(ToConnection::SendPending);
        tracing::info!(members = group.members.len(), "group created");
        self.group_info(&group.key)
    }

    /// `renameGroup` (creator).
    pub(crate) fn rename_group(&mut self, key: &str, name: &str) -> Result<Value, ApiError> {
        let (mut group, _) = self.own_group(key)?;
        let name = Self::check_name(name)?;
        if group.name != name {
            group.name = name;
            self.store.upsert_group(&group)?;
            self.store.group_outbox_add(
                key,
                &group.members,
                OUTBOX_GROUP_NAME,
                &json!({ "name": group.name }),
            )?;
            self.store
                .insert_status(key, &format!("groupRenamed|{}", group.name))?;
            self.tell_connection(ToConnection::SendPending);
        }
        self.group_info(key)
    }

    /// `setGroupMembers` (creator): setup to everyone old and new, the state to new members.
    pub(crate) fn set_group_members(
        &mut self,
        key: &str,
        members: &[String],
    ) -> Result<Value, ApiError> {
        let (mut group, user) = self.own_group(key)?;
        let members = self.member_list(members, &user)?;
        let added: Vec<String> = members
            .iter()
            .filter(|m| !group.members.contains(m))
            .cloned()
            .collect();
        let removed: Vec<String> = group
            .members
            .iter()
            .filter(|m| !members.contains(m))
            .cloned()
            .collect();
        if added.is_empty() && removed.is_empty() {
            return self.group_info(key);
        }
        let receivers: Vec<String> = members.iter().chain(&removed).cloned().collect();
        self.store.group_outbox_add(
            key,
            &receivers,
            OUTBOX_GROUP_SETUP,
            &json!({ "members": members }),
        )?;
        group.members = members;
        self.store.upsert_group(&group)?;
        if !added.is_empty() {
            self.store.group_outbox_add(
                key,
                &added,
                OUTBOX_GROUP_NAME,
                &json!({ "name": group.name }),
            )?;
            self.store.group_outbox_add(
                key,
                &added,
                groups::OUTBOX_GROUP_DELETE_PICTURE,
                &json!({}),
            )?;
        }
        for member in &added {
            self.store.insert_status(
                key,
                &format!("groupMemberAdded|{}", self.display_name_of(member)),
            )?;
        }
        for member in &removed {
            self.store.insert_status(
                key,
                &format!("groupMemberRemoved|{}", self.display_name_of(member)),
            )?;
        }
        self.tell_connection(ToConnection::SendPending);
        self.group_info(key)
    }

    /// `leaveGroup`: a member leaves (tells everyone); the creator disbands the group instead.
    pub(crate) fn leave_group(&mut self, key: &str) -> Result<Value, ApiError> {
        let mut group = self.store.group(key)?.ok_or(ApiError::GroupNotFound)?;
        if group.left {
            return self.group_info(key);
        }
        let user = self.user()?;
        let code = if group.identity.creator_identity.to_string() == user {
            self.store.group_outbox_add(
                key,
                &group.members,
                OUTBOX_GROUP_SETUP,
                &json!({ "members": [] }),
            )?;
            "groupDisbanded"
        } else {
            self.store
                .group_outbox_add(key, &group.members, OUTBOX_GROUP_LEAVE, &json!({}))?;
            "groupLeft"
        };
        group.left = true;
        self.store.upsert_group(&group)?;
        self.store.insert_status(key, code)?;
        self.tell_connection(ToConnection::SendPending);
        self.group_info(key)
    }

    /// Fetch a group's (new) picture into the media store; the UI reloads it via its version.
    pub(crate) fn download_group_picture(&self, key: String) {
        let https = self.https.clone();
        let store = Rc::clone(&self.store);
        let media = std::sync::Arc::clone(&self.media);
        let events = self.media_events.clone();
        tokio::task::spawn_local(async move {
            let Ok(Some(group)) = store.group(&key) else {
                return;
            };
            let Some(picture) = group.picture.clone() else {
                return;
            };
            let result = async {
                // No `done`: the blob is persisted for every member.
                let blob =
                    media::download(&https, &picture.blob_id, media::BlobScope::Public).await?;
                media::decrypt_blob(&picture.key, MediaPart::File, &blob)
            }
            .await;
            match result {
                Ok(data) => {
                    // Still the same picture? Then write and apply it — the check and the write
                    // run without awaiting in between, so a newer download's bytes can't be
                    // overwritten by these (a deletion meanwhile also wins).
                    if let Ok(Some(mut group)) = store.group(&key)
                        && group.picture.as_ref() == Some(&picture)
                        && media.write_picture(&key, &data).is_ok()
                    {
                        group.picture_version =
                            crate::profile_pictures::next_version(group.picture_version);
                        if store.upsert_group(&group).is_ok() {
                            let _ = events.send(CoreEvent::GroupsChanged);
                        }
                    }
                }
                Err(error) => tracing::warn!(%error, "group picture download failed"),
            }
        });
    }

    /// `setGroupPicture` (creator): attachment 0 is the JPEG (prepared by the bridge); uploaded
    /// persistently, then sent to every member.
    pub(crate) async fn set_group_picture(&mut self, key: &str) -> Result<Value, ApiError> {
        let (mut group, _) = self.own_group(key)?;
        let data = std::mem::take(&mut self.attachments)
            .into_iter()
            .next()
            .filter(|data| !data.is_empty())
            .ok_or_else(|| ApiError::Internal("no picture".into()))?;
        let mut picture_key = [0_u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut picture_key);
        let blob_id = media::upload(
            &self.https,
            &media::encrypt_blob(&picture_key, MediaPart::File, &data),
            true,
        )
        .await
        .map_err(|e| ApiError::Network(e.to_string()))?;
        self.media
            .write_picture(key, &data)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        let picture = GroupPicture {
            blob_id,
            key: picture_key,
            size: u32::try_from(data.len()).unwrap_or(u32::MAX),
        };
        self.store.group_outbox_add(
            key,
            &group.members,
            groups::OUTBOX_GROUP_SET_PICTURE,
            &picture.to_json(),
        )?;
        group.picture = Some(picture);
        group.picture_version = now_ms();
        self.store.upsert_group(&group)?;
        self.tell_connection(ToConnection::SendPending);
        self.group_info(key)
    }

    /// `removeGroupPicture` (creator).
    pub(crate) fn remove_group_picture(&mut self, key: &str) -> Result<Value, ApiError> {
        let (mut group, _) = self.own_group(key)?;
        self.store.group_outbox_add(
            key,
            &group.members,
            groups::OUTBOX_GROUP_DELETE_PICTURE,
            &json!({}),
        )?;
        group.picture = None;
        group.picture_version = 0;
        self.store.upsert_group(&group)?;
        self.media.remove_picture(key);
        self.tell_connection(ToConnection::SendPending);
        self.group_info(key)
    }

    /// Group members we have no contact for: look them up and store them with acquaintance level
    /// _group_ (not shown as contacts). Pending group messages are sent once they are known.
    pub(crate) fn fetch_members(&self, identities: Vec<String>) {
        let https = self.https.clone();
        let store = Rc::clone(&self.store);
        let events = self.media_events.clone();
        tokio::task::spawn_local(async move {
            match contacts::lookup(&https, &identities).await {
                Ok(found) => {
                    for (_, result) in found {
                        if let Lookup::Found(contact) = result {
                            let mut contact = *contact;
                            if store
                                .contact(&contact.identity.to_string())
                                .ok()
                                .flatten()
                                .is_some()
                            {
                                continue;
                            }
                            contact.acquaintance_level = pc::AcquaintanceLevel::GroupOrDeleted;
                            if let Err(error) = store.upsert_contact(&contact) {
                                tracing::warn!(%error, "storing a group member failed");
                            }
                        }
                    }
                    // Reload contact names; the media/progress channel also triggers sending.
                    let _ = events.send(CoreEvent::ContactsChanged);
                }
                Err(error) => tracing::warn!(%error, "looking up group members failed"),
            }
        });
    }
}

impl App {
    /// Fetch a contact's (new) picture, whose descriptor `picture_downloads` keeps until this
    /// succeeds. Published only if it is still the newest state after the download: a set or
    /// delete that arrived meanwhile wins (the generation changed or the row is gone), and the
    /// stale bytes are dropped before anything is written. Transient failures are retried with
    /// backoff (reconnect, maintenance); a gone blob allows a new request after a cooldown.
    pub(crate) fn download_contact_picture(&self, identity: String) {
        let https = self.https.clone();
        let store = Rc::clone(&self.store);
        let media = std::sync::Arc::clone(&self.media);
        let events = self.media_events.clone();
        tokio::task::spawn_local(async move {
            let Ok(Some((picture, generation))) = store.picture_download(&identity) else {
                return;
            };
            let result = async {
                let blob =
                    media::download(&https, &picture.blob_id, media::BlobScope::Public).await?;
                media::decrypt_blob(&picture.key, MediaPart::File, &blob)
            }
            .await;
            match result {
                Ok(data) => {
                    // Still the newest? Everything from the check to the version advance runs
                    // without awaiting, so nothing can slip in between (current-thread core).
                    let current = store.picture_download(&identity).ok().flatten();
                    if current.is_some_and(|(_, g)| g == generation)
                        && media
                            .write_picture(
                                &crate::profile_pictures::contact_picture_name(&identity),
                                &data,
                            )
                            .is_ok()
                        && store.set_contact_picture(&identity, Some(now_ms())).is_ok()
                    {
                        let _ = store.clear_picture_download(&identity, generation);
                        let _ = events.send(CoreEvent::ContactsChanged);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "contact picture download failed");
                    let gone = matches!(error, media::MediaError::Decrypt)
                        || matches!(&error, media::MediaError::Server(s) if s.contains("404"));
                    if gone {
                        // The blob expired (or the key is wrong): retrying the download is
                        // useless; only a freshly requested picture helps.
                        let _ = store.cancel_picture_download(&identity);
                        let _ = store.pause_picture_requests(
                            &identity,
                            crate::profile_pictures::REQUEST_PAUSE_EXPIRED_MS,
                        );
                    } else {
                        let _ = store.picture_download_failed(&identity, generation);
                    }
                }
            }
        });
    }

    /// Start the picture downloads that are due (startup, reconnect, maintenance): kept
    /// contact descriptors past their backoff, and group pictures that never made it onto the
    /// device (their descriptor lives in `groups.picture`; version 0 means not stored yet).
    pub(crate) fn resume_picture_downloads(&self) {
        // Contacts' pictures only while they are wanted (Android's "Receive profile pictures",
        // which also stops ones that were still pending); groups' pictures are not affected.
        if self.store.receive_pictures().unwrap_or(true)
            && let Ok(due) = self.store.due_picture_downloads()
        {
            for identity in due {
                self.download_contact_picture(identity);
            }
        }
        if let Ok(pending) = self.store.groups_with_pending_picture() {
            for key in pending {
                self.download_group_picture(key);
            }
        }
    }

    /// `setProfilePicture`: our picture (attachment 0, a JPEG prepared by the bridge). Everyone
    /// gets it with our next message to them.
    /// `setProfilePicture`: attachments are the picture and (optionally) its Threema Safe copy.
    pub(crate) fn set_profile_picture(&mut self) -> Result<Value, ApiError> {
        let mut attachments = std::mem::take(&mut self.attachments).into_iter();
        let data = attachments
            .next()
            .filter(|data| !data.is_empty())
            .ok_or_else(|| ApiError::Internal("no picture".into()))?;
        self.media
            .write_picture(crate::profile_pictures::OWN_PICTURE, &data)
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        match attachments.next().filter(|data| !data.is_empty()) {
            Some(safe_copy) => self
                .media
                .write_picture(crate::profile_pictures::OWN_PICTURE_SAFE, &safe_copy)
                .map_err(|e| ApiError::Internal(e.to_string()))?,
            None => self
                .media
                .remove_picture(crate::profile_pictures::OWN_PICTURE_SAFE),
        }
        self.store.set_picture_upload(None)?;
        self.store.purge_picture_distribution(None)?;
        self.store.bump_profile_picture_version()?;
        self.profile_json()
    }

    /// `removeProfilePicture`: contacts get a removal with our next message.
    pub(crate) fn remove_profile_picture(&mut self) -> Result<Value, ApiError> {
        self.media
            .remove_picture(crate::profile_pictures::OWN_PICTURE);
        self.media
            .remove_picture(crate::profile_pictures::OWN_PICTURE_SAFE);
        self.store.set_picture_upload(None)?;
        self.store.purge_picture_distribution(None)?;
        self.store
            .delete_setting(crate::store::keys::PROFILE_PICTURE_VERSION)?;
        self.profile_json()
    }

    /// `setProfilePictureShare`: `everyone`, `nobody` or `selected` (with `recipients`, the new
    /// allow list, if given).
    pub(crate) fn set_profile_picture_share(
        &mut self,
        share: &str,
        recipients: Option<Vec<String>>,
    ) -> Result<Value, ApiError> {
        let share = crate::profile_pictures::Share::parse(share)
            .ok_or_else(|| ApiError::Internal(format!("unknown share {share}")))?;
        self.store.set_picture_share(share)?;
        if let Some(recipients) = recipients {
            let recipients = recipients
                .iter()
                .map(|id| {
                    contacts::parse_identity(id)
                        .map(|id| id.to_string())
                        .ok_or(ApiError::IdentityFormat)
                })
                .collect::<Result<Vec<_>, _>>()?;
            self.store.set_picture_recipients(&recipients)?;
        }
        // Everyone gets the new state with the next message.
        self.store.purge_picture_distribution(None)?;
        self.profile_json()
    }

    /// `setProfilePictureRecipient`: put a contact on the allow list or take it off.
    pub(crate) fn set_profile_picture_recipient(
        &mut self,
        identity: &str,
        share: bool,
    ) -> Result<Value, ApiError> {
        let id = contacts::parse_identity(identity)
            .ok_or(ApiError::IdentityFormat)?
            .to_string();
        self.store.set_picture_recipient(&id, share)?;
        self.contact_json(&id)
    }
}
