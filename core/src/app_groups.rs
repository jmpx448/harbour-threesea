// SPDX-License-Identifier: AGPL-3.0-only
//! Group commands (create, rename, members, leave, disband) and fetching unknown members.
//! The submit steps follow csp.struct.yml: the creator sends `group-setup` to all current and
//! removed members, then name and picture to new ones; members send `group-leave` to everyone.

use std::rc::Rc;

use libthreema::common::{GroupIdentity, ThreemaId};
use serde_json::{Value, json};

use crate::{
    app::{ApiError, App},
    connection::ToConnection,
    contacts,
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

/// Fetch and decrypt a contact's or group's picture: refused by its claimed size before anything
/// is fetched, and cut off at the same limit whatever it claimed.
async fn fetch_picture(
    https: &crate::https::HttpsClient,
    picture: &GroupPicture,
) -> Result<zeroize::Zeroizing<Vec<u8>>, media::MediaError> {
    if picture.size as usize > media::MAX_PICTURE_DOWNLOAD {
        return Err(media::MediaError::TooLarge);
    }
    let blob = media::download(
        https,
        &picture.blob_id,
        media::BlobScope::Public,
        media::MAX_PICTURE_DOWNLOAD,
    )
    .await?;
    media::decrypt_blob(&picture.key, MediaPart::File, &blob)
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
        value["private"] = json!(self.store.is_private(key)?);
        // A note deleted "for everyone" would only differ from a plain delete on linked devices.
        value["canDelete"] = json!(!group.is_notes() || self.store.multi_device()?.is_some());
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

    /// `setGroupMembers` (creator): the setup to everyone old and new, the name and picture to new
    /// members.
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
        // Whoever is removed gets an empty list (csp.struct.yml): it says enough, and who else is
        // in the group from here on is none of their business.
        self.store.group_outbox_add(
            key,
            &removed,
            OUTBOX_GROUP_SETUP,
            &json!({ "members": [] }),
        )?;
        self.store.group_outbox_add(
            key,
            &members,
            OUTBOX_GROUP_SETUP,
            &json!({ "members": members }),
        )?;
        group.members = members;
        self.store.upsert_group(&group)?;
        groups::send_name_and_picture(&self.store, &group, &added)?;
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
        if !added.is_empty() {
            self.announce_running_group_call(key, added);
        }
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
        let connection = self.connection.clone();
        tokio::task::spawn_local(async move {
            let Ok(Some(group)) = store.group(&key) else {
                return;
            };
            let Some(picture) = group.picture.clone() else {
                return;
            };
            // Superseded while waiting for a slot? Then the newer picture's task does the work.
            let wanted = || matches!(store.group(&key), Ok(Some(group)) if group.picture.as_ref() == Some(&picture));
            let Some(_slot) = media::slot_if_wanted(&media::PICTURE_SLOTS, wanted).await else {
                return;
            };
            // No `done`: the blob is persisted for every member.
            let result = fetch_picture(&https, &picture).await;
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
                        if store.store_received_group_picture(&group).is_ok() {
                            let _ = events.send(CoreEvent::GroupsChanged);
                            // Now that it is here, for the other devices too.
                            crate::app_multidevice::reflect_group_picture(
                                &store,
                                &https,
                                &media,
                                &key,
                                connection.as_ref(),
                            )
                            .await;
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

    /// [`resolve_members`] off the connection's path, for what the connection didn't ask itself:
    /// the retry on reconnect and in maintenance, and what a group sync named.
    pub(crate) fn fetch_members(&self, identities: Vec<String>) {
        if let Err(error) = self.store.owe_member_lookups(&identities) {
            tracing::warn!(%error, "noting group members to look up failed");
        }
        if self.store.directory_resting() {
            return;
        }
        let https = self.https.clone();
        let config = std::sync::Arc::clone(&self.config);
        let store = Rc::clone(&self.store);
        let events = self.media_events.clone();
        let connection = self.connection.clone();
        tokio::task::spawn_local(async move {
            let Some(resolved) = resolve_members(&https, &config, &store, &identities).await else {
                return;
            };
            if resolved.synced
                && let Some(connection) = &connection
            {
                let _ = connection.send(ToConnection::ReflectSync);
            }
            if resolved.groups_changed {
                let _ = events.send(CoreEvent::GroupsChanged);
            }
            for message in resolved.lines {
                let _ = events.send(CoreEvent::MessageAdded {
                    message,
                    send_delivery_receipt: false,
                });
            }
            // Reload contact names; the media/progress channel also triggers sending.
            let _ = events.send(CoreEvent::ContactsChanged);
        });
    }
}

/// What [`resolve_members`] changed.
#[derive(Debug, Default)]
pub(crate) struct ResolvedMembers {
    /// Something went into the sync queue for the other devices.
    pub synced: bool,
    /// A group lost a member.
    pub groups_changed: bool,
    /// "Added" lines of those the directory knows, for the UI.
    pub lines: Vec<crate::messages::Message>,
}

/// Group members (and creators someone waits for) we have no contact for: look them up and store
/// them with acquaintance level _group_, not shown as contacts; what waited for them can then be
/// sent. Those the directory doesn't know leave their groups, so that nothing waits for them.
/// What the other devices are owed of them goes out in the same store step
/// (`sync::members_looked_up`). `None` if it failed: whoever is left without a contact is found
/// again by [`crate::store::Store::unresolved_members`] on reconnect and in maintenance. So is
/// whoever is over this hour's allowance, as they are not asked for now.
pub(crate) async fn resolve_members(
    https: &crate::https::HttpsClient,
    config: &libthreema::common::config::Config,
    store: &crate::store::Store,
    identities: &[String],
) -> Option<ResolvedMembers> {
    let allowed = store.member_lookup_allowance(identities.len());
    if allowed < identities.len() {
        tracing::info!(
            wanted = identities.len(),
            allowed,
            "group member lookups limited for now"
        );
    }
    if allowed == 0 {
        return None;
    }
    let results = match contacts::lookup(https, config, &identities[..allowed]).await {
        Ok(results) => results,
        Err(error) => {
            if matches!(error, crate::directory::DirectoryError::RateLimited) {
                store.rest_directory_calls();
            }
            tracing::warn!(%error, "looking up group members failed");
            return None;
        }
    };
    let invalid: Vec<String> = results
        .iter()
        .filter(|(_, result)| matches!(result, contacts::Lookup::Invalid))
        .map(|(identity, _)| identity.clone())
        .collect();
    store.note_invalid_ids(&invalid);
    let stored = crate::sync::members_looked_up(store, results)
        .and_then(|looked_up| Ok((looked_up, store.drop_invalid_members(&invalid)?)));
    match stored {
        Ok((looked_up, groups_changed)) => {
            tracing::info!(
                looked_up = allowed,
                invalid = invalid.len(),
                "group members looked up"
            );
            Some(ResolvedMembers {
                synced: looked_up.queued,
                groups_changed,
                lines: looked_up.lines,
            })
        }
        Err(error) => {
            tracing::warn!(%error, "storing group members failed");
            None
        }
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
            // Superseded while waiting for a slot? Then the newer picture's task does the work.
            let wanted =
                || matches!(store.picture_download(&identity), Ok(Some((_, g))) if g == generation);
            let Some(_slot) = media::slot_if_wanted(&media::PICTURE_SLOTS, wanted).await else {
                return;
            };
            let result = fetch_picture(&https, &picture).await;
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
                    let gone = matches!(
                        error,
                        media::MediaError::Decrypt
                            | media::MediaError::TooLarge
                            | media::MediaError::Status(404)
                    );
                    if gone {
                        // The blob expired (or the key is wrong, or it is far too large):
                        // retrying the download is useless; only a freshly requested picture
                        // helps.
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
        // What the other devices are still owed: a picture not on the mirror yet, a member whose
        // lookup failed or never got to run.
        if let Ok(owed) = self.store.group_picture_syncs() {
            for key in owed {
                let store = Rc::clone(&self.store);
                let https = self.https.clone();
                let media = std::sync::Arc::clone(&self.media);
                let connection = self.connection.clone();
                tokio::task::spawn_local(async move {
                    crate::app_multidevice::reflect_group_picture(
                        &store,
                        &https,
                        &media,
                        &key,
                        connection.as_ref(),
                    )
                    .await;
                });
            }
        }
        if let Err(error) = self.store.note_unresolved_member_lookups() {
            tracing::warn!(%error, "noting group members to look up failed");
        }
        let due = self.member_lookups_due();
        if !due.is_empty() {
            self.fetch_members(due);
        }
    }

    /// Members whose lookup failed, never got to run or waited for a rest to end.
    pub(crate) fn member_lookups_due(&self) -> Vec<String> {
        let mut due = self.store.outstanding_member_lookups().unwrap_or_default();
        due.extend(self.store.owed_member_lookups().unwrap_or_default());
        // Whoever a group or something queued still waits for, however that came about.
        due.extend(self.store.unresolved_members().unwrap_or_default());
        due.sort();
        due.dedup();
        due
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

#[cfg(test)]
mod tests {
    use libthreema::common::BlobId;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{app::StartConfig, store::StoredIdentity};

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
            .set_identity(&StoredIdentity {
                threema_id: "USER0001".into(),
                client_key: Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        (dir, app)
    }

    /// Our group with these members, all of them contacts; and who gets what is queued.
    fn own_group(app: &mut App, members: &[&str]) -> String {
        for member in ["AAAAAAAA", "BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&contacts::test_contact(member))
                .unwrap();
        }
        let members: Vec<String> = members.iter().map(|m| (*m).to_owned()).collect();
        let group = app.create_group("Crew", &members).unwrap();
        let key = group["key"].as_str().unwrap().to_owned();
        for entry in app.store.outbox().unwrap() {
            app.store.outbox_remove(&key, entry.message_id).unwrap();
        }
        key
    }

    fn queued(app: &App, key: &str) -> Vec<(String, Value, Vec<String>)> {
        app.store
            .outbox()
            .unwrap()
            .into_iter()
            .map(|entry| {
                let receivers = app.store.pending_receivers(key, entry.message_id).unwrap();
                (entry.kind, entry.body, receivers)
            })
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn someone_removed_does_not_learn_who_else_is_in_the_group() {
        let (_dir, mut app) = app();
        let key = own_group(&mut app, &["AAAAAAAA", "BBBBBBBB"]);
        app.set_group_members(&key, &["BBBBBBBB".to_owned(), "CCCCCCCC".to_owned()])
            .unwrap();
        let queued = queued(&app, &key);
        assert_eq!(
            queued[0],
            (
                OUTBOX_GROUP_SETUP.to_owned(),
                json!({ "members": [] }),
                vec!["AAAAAAAA".to_owned()]
            )
        );
        assert_eq!(
            queued[1],
            (
                OUTBOX_GROUP_SETUP.to_owned(),
                json!({ "members": ["BBBBBBBB", "CCCCCCCC"] }),
                vec!["BBBBBBBB".to_owned(), "CCCCCCCC".to_owned()]
            )
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn someone_added_gets_the_groups_picture() {
        let (_dir, mut app) = app();
        let key = own_group(&mut app, &["AAAAAAAA"]);
        let mut group = app.store.group(&key).unwrap().unwrap();
        let picture = GroupPicture {
            blob_id: BlobId([1; 16]),
            key: [2; 32],
            size: 3,
        };
        group.picture = Some(picture.clone());
        app.store.upsert_group(&group).unwrap();

        app.set_group_members(&key, &["AAAAAAAA".to_owned(), "BBBBBBBB".to_owned()])
            .unwrap();

        let added = vec!["BBBBBBBB".to_owned()];
        let queued = queued(&app, &key);
        assert_eq!(
            queued[1],
            (
                OUTBOX_GROUP_NAME.to_owned(),
                json!({ "name": "Crew" }),
                added.clone()
            )
        );
        assert_eq!(
            queued[2],
            (
                groups::OUTBOX_GROUP_SET_PICTURE.to_owned(),
                picture.to_json(),
                added
            )
        );
        assert_eq!(queued.len(), 3);
    }

    /// A picture that says it is too large isn't even asked for (the test has no network).
    #[tokio::test(flavor = "current_thread")]
    async fn a_picture_claiming_too_much_is_not_fetched() {
        let https = crate::https::HttpsClient::new().unwrap();
        let picture = GroupPicture {
            blob_id: BlobId([1; 16]),
            key: [2; 32],
            size: u32::try_from(media::MAX_PICTURE_DOWNLOAD).unwrap() + 1,
        };
        assert!(matches!(
            fetch_picture(&https, &picture).await,
            Err(media::MediaError::TooLarge)
        ));
    }
}
