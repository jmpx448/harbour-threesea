// SPDX-License-Identifier: AGPL-3.0-only
//! Multi-device commands (ADR 0011): state, linked devices, removing devices, turning it on
//! (from linking) and off.

use std::{collections::HashMap, time::Duration};

use libthreema::protobuf::{
    common::{DeltaImage, Unit, delta_image},
    d2d::{
        ContactSync, GroupSync, contact_sync, envelope::Content, group_sync,
        group_sync::update::MemberStateChange, transaction_scope::Scope,
    },
};
use serde_json::{Value, json};
use tokio::{
    sync::{mpsc::UnboundedSender, oneshot},
    time::timeout,
};

use crate::{
    app::{ApiError, App},
    connection::{
        DeviceGroupError, DropOutcome, JOIN_REPLY_LIMIT, JoinOutcome, LinkedDevice, ToConnection,
    },
    groups::Group,
    multidevice::{Properties, SETTING_FS_TERMINATE},
    store::{Store, StoreError},
};

/// A question to the mediator (30 s) and the wait for a connection coming up (15 s).
const DEVICES_REPLY_LIMIT: Duration = Duration::from_secs(50);
/// Removing devices: the transaction's start, the list, the mediator's acks; past this the
/// answer counts as missing, and what the connection still does arrives as an event.
const DROP_REPLY_LIMIT: Duration = Duration::from_secs(200);
/// Removing the ID: how long to wait for the device group; past this the ID goes all the same.
const REMOVAL_DROP_LIMIT: Duration = Duration::from_secs(60);
/// How often to look whether this device is alone in its group (Android: hourly, and a removal on
/// another device gives no other signal); a little under the hourly maintenance.
const ALONE_CHECK_INTERVAL: Duration = Duration::from_secs(50 * 60);

/// No connection is shown as such by the UI; the server's or protocol's "no" keeps its text.
fn device_group_error(error: DeviceGroupError) -> ApiError {
    match error {
        DeviceGroupError::NotConnected => ApiError::Network(error.to_string()),
        DeviceGroupError::Failed(reason) => ApiError::Internal(reason),
    }
}

/// The connection never answered (ended, or still busy after the limit).
fn no_connection() -> ApiError {
    device_group_error(DeviceGroupError::NotConnected)
}

/// A name the user typed: trimmed, and short enough for the protobuf fields it travels in
/// (the label goes out in every D2M client-hello).
fn trimmed_name(input: &str) -> String {
    let mut name = input.trim().to_owned();
    while name.len() > 64 {
        name.pop();
    }
    name
}

impl App {
    /// Point blob transfers at the device group's mirror (multi-device) or the usual server.
    pub(crate) fn apply_multi_device(&self) {
        let properties = self.store.multi_device().ok().flatten();
        let key = properties.as_ref().and_then(Properties::device_group_key);
        crate::media::set_mirror(
            key.as_ref()
                .zip(properties.as_ref().map(|p| p.mediator_device_id)),
        );
    }

    /// A picture changed on another device: fetch it from the mirror (local scope, as Android's
    /// `ReflectedBlobDownloader`) and store it like a received one; or remove it.
    pub(crate) fn apply_synced_picture(
        &self,
        target: String,
        blob: Option<crate::sync::SyncedBlob>,
    ) {
        use crate::{media, profile_pictures::OWN_PICTURE, providers::CoreEvent};
        let https = self.https.clone();
        let store = std::rc::Rc::clone(&self.store);
        let media_store = std::sync::Arc::clone(&self.media);
        let events = self.media_events.clone();
        tokio::task::spawn_local(async move {
            let data = match &blob {
                Some(blob) => {
                    let result = async {
                        let _slot = media::PICTURE_SLOTS.acquire().await;
                        let encrypted = media::download(
                            &https,
                            &blob.id,
                            media::BlobScope::Local,
                            media::MAX_PICTURE_DOWNLOAD,
                        )
                        .await?;
                        let data =
                            media::decrypt_blob_with_nonce(&blob.key, &blob.nonce, &encrypted)?;
                        media::mark_done(&https, &blob.id, media::BlobScope::Local).await;
                        Ok::<_, media::MediaError>(data)
                    }
                    .await;
                    match result {
                        Ok(data) => Some(data),
                        Err(error) => {
                            tracing::warn!(%error, "synced picture not fetched");
                            return;
                        }
                    }
                }
                None => None,
            };
            let now = crate::messages::now_ms();
            let stored = (|| -> Result<CoreEvent, Box<dyn std::error::Error>> {
                match &data {
                    Some(data) => media_store.write_picture(&target, data)?,
                    None => media_store.remove_picture(&target),
                }
                Ok(if target == OWN_PICTURE {
                    // A new own picture: contacts get it again; Safe's copy is made on demand.
                    media_store.remove_picture(crate::profile_pictures::OWN_PICTURE_SAFE);
                    store.set_picture_upload(None)?;
                    store.purge_picture_distribution(None)?;
                    match data {
                        Some(_) => store.bump_profile_picture_version()?,
                        None => {
                            store.delete_setting(crate::store::keys::PROFILE_PICTURE_VERSION)?
                        }
                    }
                    // The own card listens for the profile, not the contacts.
                    CoreEvent::ProfileChanged
                } else if let Some(identity) = target.strip_prefix("contact:") {
                    // The synced state is the newest; a download still under way is obsolete.
                    store.cancel_picture_download(identity)?;
                    store.set_contact_picture(identity, data.as_ref().map(|_| now))?;
                    CoreEvent::ContactsChanged
                } else {
                    if let Some(mut group) = store.group(&target)? {
                        group.picture_version = if data.is_some() {
                            crate::profile_pictures::next_version(group.picture_version)
                        } else {
                            // Its descriptor goes too, or it would be downloaded again as one
                            // that never made it onto the device.
                            group.picture = None;
                            0
                        };
                        store.upsert_group(&group)?;
                    }
                    CoreEvent::GroupsChanged
                })
            })();
            match stored {
                Ok(event) => {
                    let _ = events.send(event);
                }
                Err(error) => tracing::warn!(%error, "synced picture not stored"),
            }
        });
    }

    /// `multiDevice`: `{active}`.
    pub(crate) fn multi_device_json(&self) -> Result<Value, ApiError> {
        Ok(json!({ "active": self.store.multi_device()?.is_some() }))
    }

    /// `linkedDevices`: `{devices: [LinkedDevice]}` (this device first). Each device also
    /// carries the `alias` the user gave it here, and this device its own `label` (what the
    /// others show for it).
    pub(crate) async fn linked_devices(&mut self) -> Result<Value, ApiError> {
        if self.store.multi_device()?.is_none() {
            return Ok(json!({ "devices": [] }));
        }
        let (reply, answer) = oneshot::channel();
        self.tell_connection(ToConnection::Devices(reply));
        let devices = timeout(DEVICES_REPLY_LIMIT, answer)
            .await
            .map_err(|_| no_connection())?
            .map_err(|_| no_connection())?
            .map_err(device_group_error)?;
        self.devices_json(&devices)
    }

    /// The linked devices for the UI, with the names the user gave them here.
    fn devices_json(&self, devices: &[LinkedDevice]) -> Result<Value, ApiError> {
        let mut devices =
            serde_json::to_value(devices).map_err(|error| ApiError::Internal(error.to_string()))?;
        for device in devices.as_array_mut().into_iter().flatten() {
            let Some(id) = device["id"].as_str().map(str::to_owned) else {
                continue;
            };
            if let Some(alias) = self.device_alias(&id)? {
                device["alias"] = alias.into();
            }
        }
        Ok(json!({ "devices": devices, "label": self.device_label()?, "active": true }))
    }

    /// `linkedDevices` as the page asks for it: a list of just this device means nothing else
    /// is linked any more (removed on the other device, say), so multi-device ends now.
    pub(crate) async fn linked_devices_checked(&mut self) -> Result<Value, ApiError> {
        let devices = self.linked_devices().await?;
        let others = devices["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|device| device["current"] != true)
            .count();
        if others == 0 {
            self.deactivate_if_alone();
        }
        Ok(devices)
    }

    /// Multi-device ends if this device turns out to be the only one in the group; the
    /// connection decides from the mediator's list and reports it as an event. Android's
    /// `DeactivateMultiDeviceIfAloneTask`.
    fn deactivate_if_alone(&self) {
        if matches!(self.store.multi_device(), Ok(Some(_))) {
            let (reply, _answer) = oneshot::channel();
            self.tell_connection(ToConnection::DropDevices {
                ids: Some(Vec::new()),
                reply,
            });
        }
    }

    /// The periodic form of that, from `maintenance` and when connected.
    pub(crate) fn check_device_group(&mut self) {
        let now = tokio::time::Instant::now();
        if self
            .last_group_check
            .is_some_and(|at| now.duration_since(at) < ALONE_CHECK_INTERVAL)
        {
            return;
        }
        self.last_group_check = Some(now);
        self.deactivate_if_alone();
    }

    /// This device's label, as the other devices show it.
    pub(crate) fn device_label(&self) -> Result<String, ApiError> {
        Ok(self
            .store
            .setting(crate::multidevice::SETTING_DEVICE_LABEL)?
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| crate::multidevice::DEVICE_LABEL.to_owned()))
    }

    /// `setDeviceLabel`: what the other devices show for this one. It travels in the D2M
    /// client-hello, so the connection is started again for the mediator to learn it.
    pub(crate) fn set_device_label(&mut self, label: &str) -> Result<Value, ApiError> {
        let label = trimmed_name(label);
        if label.is_empty() {
            self.store
                .delete_setting(crate::multidevice::SETTING_DEVICE_LABEL)?;
        } else {
            self.store
                .set_setting(crate::multidevice::SETTING_DEVICE_LABEL, &label)?;
        }
        if self.store.multi_device()?.is_some() {
            self.tell_connection(ToConnection::Reconnect);
        }
        Ok(json!({ "label": self.device_label()? }))
    }

    /// The name the user gave another device of the group, for our own lists.
    fn device_alias(&self, id: &str) -> Result<Option<String>, ApiError> {
        Ok(self
            .store
            .setting(&format!("{}{id}", crate::multidevice::SETTING_ALIAS_PREFIX))?
            .filter(|alias| !alias.is_empty()))
    }

    /// `setDeviceAlias`: name another device of the group for this device's lists (a device's
    /// own label is encrypted with the device group key and only it can change it).
    pub(crate) fn set_device_alias(&mut self, id: &str, alias: &str) -> Result<Value, ApiError> {
        if id.len() != 16 || !id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(ApiError::Internal("bad device id".into()));
        }
        let key = format!("{}{id}", crate::multidevice::SETTING_ALIAS_PREFIX);
        let alias = trimmed_name(alias);
        if alias.is_empty() {
            self.store.delete_setting(&key)?;
        } else {
            self.store.set_setting(&key, &alias)?;
        }
        Ok(json!({}))
    }

    /// Remove devices through the connection (see `ToConnection::DropDevices`).
    async fn drop_from_group(&self, ids: Option<Vec<u64>>) -> Result<DropOutcome, ApiError> {
        let (reply, answer) = oneshot::channel();
        self.tell_connection(ToConnection::DropDevices { ids, reply });
        timeout(DROP_REPLY_LIMIT, answer)
            .await
            .map_err(|_| no_connection())?
            .map_err(|_| no_connection())?
            .map_err(device_group_error)
    }

    /// `dropDevices`: remove devices (hex ids) from the group. Once none but this one would
    /// remain, this one leaves too: multi-device ends and forward security returns.
    pub(crate) async fn drop_devices(&mut self, ids: &[String]) -> Result<Value, ApiError> {
        let ids = ids
            .iter()
            .map(|id| u64::from_str_radix(id, 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApiError::Internal("bad device id".into()))?;
        let outcome = self.drop_from_group(Some(ids)).await;
        let outcome = self.tell_failed_removal(outcome)?;
        if outcome.left_group {
            return Ok(json!({ "devices": [], "active": false }));
        }
        self.devices_json(&outcome.remaining)
    }

    /// `deactivateMultiDevice`: remove every other device, then this one (Android's
    /// `DeactivateMultiDeviceTask`); the connection goes back to the chat server. When that
    /// doesn't work nothing changes here, so it can be tried again: a phone that has left can't
    /// remove anything any more, and the others keep the ID. The UI then offers
    /// `leave_locally`, which leaves the group on this device alone: the way out when the
    /// mediator stays out of reach (user report 2026-10-28: offline until the ID was set up
    /// anew).
    pub(crate) async fn deactivate_multi_device(
        &mut self,
        leave_locally: bool,
    ) -> Result<Value, ApiError> {
        if self.store.multi_device()?.is_none() {
            return self.multi_device_json();
        }
        if leave_locally {
            tracing::warn!("leaving the device group on this device only");
            self.leave_device_group_locally().await;
        } else {
            let outcome = self.drop_from_group(None).await;
            self.tell_failed_removal(outcome)?;
        }
        self.multi_device_json()
    }

    /// The ID is being removed: take the linked devices out of the group first (Android's
    /// `ResetAppTask` does too), because afterwards this phone can't reach them any more. True
    /// if some may have stayed: no connection, or no answer in time. That doesn't stop the
    /// removal; the user asked for it and is told.
    pub(crate) async fn drop_linked_devices_for_removal(&mut self) -> bool {
        if !matches!(self.store.multi_device(), Ok(Some(_))) {
            return false;
        }
        let dropped = timeout(REMOVAL_DROP_LIMIT, self.drop_from_group(None)).await;
        let remain = !matches!(dropped, Ok(Ok(_)));
        if remain {
            tracing::warn!("removing the ID without dropping the linked devices");
        }
        remain
    }

    /// The reply can take minutes and the page that asked may be gone by then, so a failed
    /// removal is also an event the UI hears wherever the user is.
    fn tell_failed_removal(
        &mut self,
        outcome: Result<DropOutcome, ApiError>,
    ) -> Result<DropOutcome, ApiError> {
        if outcome.is_err() {
            self.note_ui_events(&["deviceRemovalFailed"]);
        }
        outcome
    }

    /// Multi-device off on this device, whatever the mediator knows: clear the stored
    /// properties, tell the UI, publish the feature mask (with forward security again, best
    /// effort) and reconnect — straight to the chat server from now on.
    pub(crate) async fn leave_device_group_locally(&mut self) {
        match self.store.set_multi_device(None) {
            Ok(()) => self.note_ui_events(&["multiDeviceChanged"]),
            Err(error) => {
                tracing::error!(%error, "removing multi-device failed");
                return;
            }
        }
        // A session termination the activation asked for and the mediator connection never
        // ran must not end the sessions now that forward security is back.
        if !matches!(self.store.forward_security_disabled(), Ok(true))
            && let Err(error) = self.store.delete_setting(SETTING_FS_TERMINATE)
        {
            tracing::warn!(%error, "pending session termination not dropped");
        }
        self.apply_multi_device();
        if let Err(error) = self.publish_feature_mask().await {
            // Published again at the next start.
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
        }
        self.tell_connection(ToConnection::Reconnect);
    }

    /// Turn multi-device on (linking, after the emoji check): new device group, forward
    /// security off (feature mask now, sessions terminated by the connection once logged in),
    /// then connect through the mediator. Android: `DeviceLinkingPartOneTask.activateMultiDevice`.
    pub(crate) async fn activate_multi_device(&mut self) -> Result<(), ApiError> {
        if self.store.multi_device()?.is_some() {
            return Ok(());
        }
        self.store.set_multi_device(Some(&Properties::generate()))?;
        self.store.set_setting(SETTING_FS_TERMINATE, "1")?;
        self.apply_multi_device();
        if let Err(error) = self.publish_feature_mask().await {
            // Published again at the next start; the sessions end regardless.
            tracing::warn!(kind = error.kind(), "publishing the feature mask failed");
        }
        self.tell_connection(ToConnection::Reconnect);
        Ok(())
    }

    /// `startLinking`: parse the code, connect, return the emoji check (one right row, two
    /// random ones, like Android's `LinkNewDeviceVerifyFragment`).
    pub(crate) async fn start_linking(&mut self, code: &str) -> Result<Value, ApiError> {
        use crate::{multidevice::RENDEZVOUS_EMOJIS, rendezvous::LinkError};
        if let Some(old) = self.pending_link.take() {
            old.close().await;
        }
        let offer = crate::rendezvous::parse_join_code(code).map_err(|error| match error {
            LinkError::WebCode => ApiError::LinkWebCode,
            LinkError::Version => ApiError::LinkVersion,
            _ => ApiError::LinkUnknownCode,
        })?;
        let link = crate::rendezvous::RendezvousLink::connect(&offer)
            .await
            .map_err(|error| ApiError::LinkFailed(error.to_string()))?;
        let row = |indices: [usize; 3]| -> Vec<&str> {
            indices
                .iter()
                .map(|&i| RENDEZVOUS_EMOJIS[i % 128])
                .collect()
        };
        let right = link.emoji_indices();
        let correct = usize::from(rand::random::<u8>() % 3);
        let mut choices = Vec::new();
        for index in 0..3 {
            if index == correct {
                choices.push(row(right));
            } else {
                // A different random row.
                let mut decoy = right;
                while decoy == right {
                    decoy = [
                        usize::from(rand::random::<u8>() % 128),
                        usize::from(rand::random::<u8>() % 128),
                        usize::from(rand::random::<u8>() % 128),
                    ];
                }
                choices.push(row(decoy));
            }
        }
        self.pending_link = Some(link);
        Ok(json!({ "choices": choices, "correct": correct }))
    }

    /// `confirmLinking`: like Android's device linking part one (activation) and two (data).
    pub(crate) async fn confirm_linking(&mut self) -> Result<Value, ApiError> {
        let link = self
            .pending_link
            .take()
            .ok_or_else(|| ApiError::LinkFailed("no linking in progress".into()))?;
        let was_active = self.store.multi_device()?.is_some();
        self.activate_multi_device().await?;
        let (reply, answer) = oneshot::channel();
        // Queued behind the reconnect: it runs once connected through the mediator.
        self.tell_connection(ToConnection::NewDeviceSync {
            link: Box::new(link),
            reply,
        });
        // Not forever: the core handles nothing else meanwhile (deep audit 2026-10-27, P01).
        // The connection answers within this limit; if it doesn't, all that is known is that
        // it may still be working on the join, so nothing is rolled back.
        let outcome = match timeout(JOIN_REPLY_LIMIT, answer).await {
            Err(_) => {
                tracing::warn!("no answer on the device join");
                return Err(ApiError::LinkUncertain);
            }
            Ok(Err(_)) => JoinOutcome::FailedBeforeHandover("connection gone".into()),
            Ok(Ok(outcome)) => outcome,
        };
        self.finish_linking(outcome, was_active).await
    }

    /// What the join's end means for this device. Before the handover, a group that exists only
    /// for this join is simply left: nobody else ever had it (without that, a failed first
    /// linking left the phone alone behind the mediator, offline for good when the mediator
    /// connection was the problem: user report 2026-10-28). After it, the new device may have
    /// the ID and be in the group, so this device stays in charge until the mediator has
    /// removed what is there; a device group that was there before is the user's to manage.
    async fn finish_linking(
        &mut self,
        outcome: JoinOutcome,
        was_active: bool,
    ) -> Result<Value, ApiError> {
        match outcome {
            JoinOutcome::Joined => Ok(match self.linked_devices().await {
                Ok(devices) => devices,
                // The new device is in. The list is shown when it can be fetched again.
                Err(error) => {
                    tracing::warn!(kind = error.kind(), "linked devices not read after linking");
                    json!({ "devices": [], "active": true })
                }
            }),
            JoinOutcome::FailedBeforeHandover(reason) => {
                if !was_active {
                    tracing::warn!("linking failed; leaving the new device group again");
                    self.leave_device_group_locally().await;
                }
                Err(ApiError::LinkFailed(reason))
            }
            JoinOutcome::FailedAfterHandover(_) if was_active => Err(ApiError::LinkUncertain),
            JoinOutcome::FailedAfterHandover(_) => match self.drop_from_group(None).await {
                Ok(dropped) if dropped.left_group => Err(ApiError::LinkRemoved),
                Ok(_) => Err(ApiError::LinkUncertain),
                // Still in the group: Linked devices tells, and can remove it once online; if
                // nothing else is in it, the regular check ends multi-device.
                Err(error) => {
                    tracing::warn!(
                        kind = error.kind(),
                        "device group not removed after linking"
                    );
                    Err(ApiError::LinkUncertain)
                }
            },
        }
    }

    /// A group we left is deleted here: from the other devices too (Android's
    /// `ReflectGroupSyncDeleteTask`), unless they never got it. Queued while the row is there.
    pub(crate) fn reflect_group_deleted(&self, group: &Group) -> Result<(), ApiError> {
        if self.store.multi_device()?.is_none() || !crate::join::group_syncable(&self.store, group)?
        {
            return Ok(());
        }
        queue_syncs(
            &self.store,
            vec![(
                Scope::GroupSync,
                Content::GroupSync(GroupSync {
                    action: Some(group_sync::Action::Delete(group_sync::Delete {
                        group_identity: Some(libthreema::protobuf::common::GroupIdentity {
                            group_id: group.identity.group_id,
                            creator_identity: group.identity.creator_identity.to_string(),
                        }),
                    })),
                }),
            )],
        );
        self.tell_connection(ToConnection::ReflectSync);
        Ok(())
    }

    /// After a command that changed something other devices keep too: reflect it.
    pub(crate) async fn reflect_change(&self, change: SyncChange, result: &Value) {
        use libthreema::protobuf::d2d::{
            SettingsSync, UserProfileSync, settings_sync, user_profile_sync,
        };
        if !matches!(self.store.multi_device(), Ok(Some(_))) {
            return;
        }
        let built = (|| -> Result<Vec<(Scope, Content)>, crate::store::StoreError> {
            Ok(match change {
                SyncChange::NewContacts(identities) => {
                    let mut out = Vec::new();
                    for identity in identities {
                        let Some(contact) = self.store.contact(&identity)? else {
                            continue;
                        };
                        let synced = crate::join::contact_sync(&self.store, &contact)?;
                        out.push((
                            Scope::ContactSync,
                            Content::ContactSync(ContactSync {
                                action: Some(contact_sync::Action::Create(contact_sync::Create {
                                    contact: Some(synced),
                                })),
                            }),
                        ));
                    }
                    out
                }
                SyncChange::Contacts(identities) => {
                    let mut out = Vec::new();
                    for identity in identities {
                        let Some(contact) = self.store.contact(&identity)? else {
                            continue;
                        };
                        let synced = crate::join::contact_sync(&self.store, &contact)?;
                        out.push((
                            Scope::ContactSync,
                            Content::ContactSync(ContactSync {
                                action: Some(contact_sync::Action::Update(contact_sync::Update {
                                    contact: Some(synced),
                                })),
                            }),
                        ));
                    }
                    out
                }
                SyncChange::Group {
                    key,
                    created,
                    previous_members,
                } => {
                    let key = key.or_else(|| result["key"].as_str().map(str::to_owned));
                    let Some(group) = key.and_then(|key| self.store.group(&key).ok().flatten())
                    else {
                        return Ok(Vec::new());
                    };
                    if !crate::join::group_syncable(&self.store, &group)? {
                        return Ok(Vec::new());
                    }
                    let mut synced = crate::join::group_sync(&self.store, &group)?;
                    // Unchanged here: leave it alone there.
                    synced.profile_picture = None;
                    let action = if created {
                        group_sync::Action::Create(group_sync::Create {
                            group: Some(synced),
                        })
                    } else {
                        group_sync::Action::Update(group_sync::Update {
                            group: Some(synced),
                            member_state_changes: previous_members
                                .map(|previous| member_changes_by_command(&previous, &group))
                                .unwrap_or_default(),
                        })
                    };
                    vec![(
                        Scope::GroupSync,
                        Content::GroupSync(GroupSync {
                            action: Some(action),
                        }),
                    )]
                }
                SyncChange::Conversation(conversation) => {
                    return Ok(if conversation.starts_with("g:") {
                        self.store
                            .group(&conversation)?
                            .filter(|group| {
                                crate::join::group_syncable(&self.store, group).unwrap_or(false)
                            })
                            .map_or_else(Vec::new, |group| {
                                crate::join::group_sync(&self.store, &group)
                                    .map(|mut synced| {
                                        // Only what a chat's marks change: the rest is the
                                        // creator's and may have moved on elsewhere.
                                        synced.profile_picture = None;
                                        synced.member_identities = None;
                                        synced.name = None;
                                        synced.created_at = None;
                                        synced.user_state = None;
                                        vec![(
                                            Scope::GroupSync,
                                            Content::GroupSync(GroupSync {
                                                action: Some(group_sync::Action::Update(
                                                    group_sync::Update {
                                                        group: Some(synced),
                                                        member_state_changes:
                                                            std::collections::HashMap::new(),
                                                    },
                                                )),
                                            }),
                                        )]
                                    })
                                    .unwrap_or_default()
                            })
                    } else {
                        let mut out = Vec::new();
                        if let Some(contact) = self.store.contact(&conversation)? {
                            out.push((
                                Scope::ContactSync,
                                Content::ContactSync(ContactSync {
                                    action: Some(contact_sync::Action::Update(
                                        contact_sync::Update {
                                            contact: Some(crate::join::contact_sync(
                                                &self.store,
                                                &contact,
                                            )?),
                                        },
                                    )),
                                }),
                            ));
                        }
                        out
                    });
                }
                SyncChange::GroupPicture(key) => {
                    let Some(group) = self.store.group(&key)? else {
                        return Ok(Vec::new());
                    };
                    let mut synced = crate::sync::group_only(&group);
                    synced.profile_picture = Some(DeltaImage {
                        image: Some(delta_image::Image::Removed(Unit {})),
                    });
                    vec![(
                        Scope::GroupSync,
                        Content::GroupSync(GroupSync {
                            action: Some(group_sync::Action::Update(group_sync::Update {
                                group: Some(synced),
                                member_state_changes: std::collections::HashMap::new(),
                            })),
                        }),
                    )]
                }
                SyncChange::Settings => vec![(
                    Scope::SettingsSync,
                    Content::SettingsSync(SettingsSync {
                        action: Some(settings_sync::Action::Update(settings_sync::Update {
                            settings: Some(crate::join::settings_sync(&self.store)?),
                        })),
                    }),
                )],
                SyncChange::Profile { picture } => {
                    let mut profile = crate::join::profile_sync(&self.store)?;
                    // `profile_sync` says "removed"; a picture is filled in below.
                    if !picture {
                        profile.profile_picture = None;
                    }
                    vec![(
                        Scope::UserProfileSync,
                        Content::UserProfileSync(UserProfileSync {
                            action: Some(user_profile_sync::Action::Update(
                                user_profile_sync::Update {
                                    user_profile: Some(profile),
                                },
                            )),
                        }),
                    )]
                }
            })
        })();
        let mut built = built;
        // A picture that is here goes to the mirror first (Android: BlobScope.Local).
        if let Ok(items) = built.as_mut() {
            for (_, content) in items.iter_mut() {
                let (target, slot) = match content {
                    Content::UserProfileSync(UserProfileSync {
                        action:
                            Some(user_profile_sync::Action::Update(user_profile_sync::Update {
                                user_profile: Some(profile),
                            })),
                    }) => (
                        crate::profile_pictures::OWN_PICTURE.to_owned(),
                        &mut profile.profile_picture,
                    ),
                    Content::GroupSync(GroupSync {
                        action:
                            Some(group_sync::Action::Update(group_sync::Update {
                                group: Some(group),
                                ..
                            })),
                    }) => match group.group_identity.as_ref().and_then(|g| {
                        Some(crate::groups::key(&libthreema::common::GroupIdentity {
                            group_id: g.group_id,
                            creator_identity: g.creator_identity.parse().ok()?,
                        }))
                    }) {
                        Some(key) => (key, &mut group.profile_picture),
                        None => continue,
                    },
                    _ => continue,
                };
                // Only where the picture is part of this change ("removed" placeholder set).
                if slot.is_none() {
                    continue;
                }
                match upload_sync_picture(&self.https, &self.media, &target).await {
                    Ok(Some(image)) => *slot = Some(image),
                    Ok(None) => {}
                    Err(error) => {
                        tracing::warn!(%error, "picture not uploaded to the mirror");
                        *slot = None;
                    }
                }
            }
        }
        match built {
            Ok(items) => {
                queue_syncs(&self.store, items);
                self.tell_connection(ToConnection::ReflectSync);
            }
            Err(error) => tracing::warn!(%error, "sync not reflected"),
        }
    }
}

/// Queue encoded sync envelopes; the connection reflects them in order, each in its transaction.
fn queue_syncs(store: &Store, items: Vec<(Scope, Content)>) {
    for (scope, content) in items {
        if let Err(error) = store.queue_sync_content(scope, content) {
            tracing::warn!(%error, "sync not queued");
        }
    }
}

/// `member_state_changes` of a member change the user made here: who is new, who was removed.
fn member_changes_by_command(previous: &[String], group: &Group) -> HashMap<String, i32> {
    let added = group
        .members
        .iter()
        .filter(|member| !previous.contains(member))
        .map(|member| (member.clone(), MemberStateChange::Added as i32));
    let removed = previous
        .iter()
        .filter(|member| !group.members.contains(member))
        .map(|member| (member.clone(), MemberStateChange::Kicked as i32));
    added.chain(removed).collect()
}

/// A group's picture from its creator reached this device: the others get it from the mirror.
/// Noted as owed until the entry is queued (`Store::store_received_group_picture`), so a failed
/// upload or a crash is made up for by the next `resume_picture_downloads`.
pub(crate) async fn reflect_group_picture(
    store: &Store,
    https: &crate::https::HttpsClient,
    media: &crate::media::MediaStore,
    key: &str,
    connection: Option<&UnboundedSender<ToConnection>>,
) {
    let done = |store: &Store| {
        if let Err(error) = store.group_picture_synced(key) {
            tracing::warn!(%error, "owed group picture not cleared");
        }
    };
    let group = match store.group(key) {
        // Version 0: deleted meanwhile, which has been queued by itself. Its file may still
        // be there.
        Ok(Some(group))
            if group.picture_version != 0 && matches!(store.multi_device(), Ok(Some(_))) =>
        {
            group
        }
        // Nobody left to tell, or nothing to tell them.
        Ok(_) => return done(store),
        Err(error) => {
            tracing::warn!(%error, "owed group picture not read");
            return;
        }
    };
    let image = match upload_sync_picture(https, media, key).await {
        Ok(Some(image)) => image,
        // Removed meanwhile; that has been queued by itself.
        Ok(None) => return done(store),
        Err(error) => {
            tracing::warn!(%error, "picture not uploaded to the mirror");
            return;
        }
    };
    let mut synced = crate::sync::group_only(&group);
    synced.profile_picture = Some(image);
    match queue_owed_picture(store, key, group.picture_version, synced) {
        Ok(true) => {
            if let Some(connection) = connection {
                let _ = connection.send(ToConnection::ReflectSync);
            }
        }
        Ok(false) => {}
        Err(error) => tracing::warn!(%error, "group picture sync not queued"),
    }
}

/// One store step: only if it is still owed and still the picture that was uploaded (`version`).
/// Two tries for one picture send it once; a newer picture's try sends that, and the note stays
/// for it meanwhile.
fn queue_owed_picture(
    store: &Store,
    key: &str,
    version: i64,
    synced: libthreema::protobuf::d2d_sync::Group,
) -> Result<bool, StoreError> {
    let step = store.step()?;
    let same = store
        .group(key)?
        .is_some_and(|now| now.picture_version == version);
    if !same || !store.group_picture_synced(key)? {
        return Ok(false);
    }
    store.queue_sync_content(
        Scope::GroupSync,
        Content::GroupSync(GroupSync {
            action: Some(group_sync::Action::Update(group_sync::Update {
                group: Some(synced),
                member_state_changes: HashMap::new(),
            })),
        }),
    )?;
    step.commit()?;
    Ok(true)
}

/// The picture stored as `target`, encrypted and uploaded to the mirror (local scope) as a sync
/// `DeltaImage`; `None` if there is none (the "removed" placeholder stays).
async fn upload_sync_picture(
    https: &crate::https::HttpsClient,
    media: &crate::media::MediaStore,
    target: &str,
) -> Result<Option<libthreema::protobuf::common::DeltaImage>, crate::media::MediaError> {
    use libthreema::protobuf::common::{Blob, DeltaImage, Image, delta_image, image};
    let Some(data) = media.read_picture(target)? else {
        return Ok(None);
    };
    let key: [u8; 32] = rand::random();
    let nonce = *crate::media::MediaPart::File.nonce();
    let encrypted = crate::media::encrypt_blob(&key, crate::media::MediaPart::File, &data);
    let blob_id =
        crate::media::upload_scoped(https, &encrypted, false, crate::media::BlobScope::Local)
            .await?;
    Ok(Some(DeltaImage {
        image: Some(delta_image::Image::Updated(Image {
            r#type: image::Type::Jpeg as i32,
            blob: Some(Blob {
                id: blob_id.0.to_vec(),
                nonce: nonce.to_vec(),
                key: key.to_vec(),
                uploaded_at: u64::try_from(crate::messages::now_ms()).unwrap_or_default(),
            }),
        })),
    }))
}

/// What a command changed that other devices keep too (multi-device).
pub(crate) enum SyncChange {
    /// Contacts the other devices have.
    Contacts(Vec<String>),
    /// Contacts new here, so to them too.
    NewContacts(Vec<String>),
    /// A group (`None`: the new group's key from the command's result).
    Group {
        key: Option<String>,
        created: bool,
        /// The members before a member change: the difference goes along, as a status line there.
        previous_members: Option<Vec<String>>,
    },
    /// A chat's pin, archive, private state or notifications: contact or group.
    Conversation(String),
    Settings,
    /// Nickname, sharing; with `picture` also our picture (set or removed).
    Profile {
        picture: bool,
    },
    /// A group's picture (set or removed).
    GroupPicture(String),
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        app::StartConfig,
        groups::{GroupChange, MemberChange},
        sync::queue_received_group_change,
    };

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
        (dir, app)
    }

    /// A device group this phone is in, and the other end of its connection.
    fn in_a_group(app: &mut App) -> mpsc::UnboundedReceiver<ToConnection> {
        app.store
            .set_multi_device(Some(&Properties::generate()))
            .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        app.connection = Some(tx);
        rx
    }

    /// What the connection would reflect next, removed from the queue.
    fn next_group_sync(app: &App) -> group_sync::Action {
        use prost::Message as _;
        let (id, scope, content) = app.store.next_sync().unwrap().unwrap();
        app.store.sync_done(id).unwrap();
        assert_eq!(scope, Scope::GroupSync as i32);
        let envelope = libthreema::protobuf::d2d::Envelope::decode(content.as_slice()).unwrap();
        let Some(Content::GroupSync(sync)) = envelope.content else {
            panic!("not a group sync");
        };
        sync.action.unwrap()
    }

    /// What the connection would reflect next for a contact, removed from the queue.
    fn next_contact_sync(app: &App) -> libthreema::protobuf::d2d_sync::Contact {
        use prost::Message as _;
        let (id, scope, content) = app.store.next_sync().unwrap().unwrap();
        app.store.sync_done(id).unwrap();
        assert_eq!(scope, Scope::ContactSync as i32);
        let envelope = libthreema::protobuf::d2d::Envelope::decode(content.as_slice()).unwrap();
        let Some(Content::ContactSync(ContactSync {
            action: Some(contact_sync::Action::Update(update)),
        })) = envelope.content
        else {
            panic!("not a contact update");
        };
        update.contact.unwrap()
    }

    fn update_of(action: group_sync::Action) -> group_sync::Update {
        let group_sync::Action::Update(update) = action else {
            panic!("not an update");
        };
        update
    }

    fn foreign_group(app: &App) -> Group {
        let identity = libthreema::common::GroupIdentity {
            creator_identity: libthreema::common::ThreemaId::try_from("CREATOR1").unwrap(),
            group_id: 7,
        };
        let group = Group {
            key: crate::groups::key(&identity),
            identity,
            name: "Crew".into(),
            members: vec!["CREATOR1".into(), "BBBBBBBB".into(), "CCCCCCCC".into()],
            left: false,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        for id in ["CREATOR1", "BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        group
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_picture_removed_on_another_device_does_not_come_back() {
        let (_dir, app) = app();
        let mut group = foreign_group(&app);
        group.picture = Some(crate::groups::GroupPicture {
            blob_id: libthreema::common::BlobId([1; 16]),
            key: [2; 32],
            size: 3,
        });
        group.picture_version = 5;
        app.store.upsert_group(&group).unwrap();
        tokio::task::LocalSet::new()
            .run_until(async {
                app.apply_synced_picture(group.key.clone(), None);
                while app
                    .store
                    .group(&group.key)
                    .unwrap()
                    .unwrap()
                    .picture_version
                    != 0
                {
                    tokio::task::yield_now().await;
                }
            })
            .await;
        // Not a download still to be retried.
        assert!(
            app.store
                .group(&group.key)
                .unwrap()
                .unwrap()
                .picture
                .is_none()
        );
        assert!(app.store.groups_with_pending_picture().unwrap().is_empty());
    }

    fn group_change(name: bool) -> GroupChange {
        GroupChange {
            name,
            ..GroupChange::default()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_received_group_change_goes_out_as_only_what_changed() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        let queue = |change: &GroupChange| {
            assert!(queue_received_group_change(&app.store, &group.key, change, &[]).unwrap());
        };

        // A member left.
        let mut change = GroupChange {
            members: true,
            ..GroupChange::default()
        };
        change
            .member_changes
            .insert("CCCCCCCC".into(), MemberChange::Left);
        let mut left = group.clone();
        left.members.retain(|member| member != "CCCCCCCC");
        app.store.upsert_group(&left).unwrap();
        queue(&change);
        let update = update_of(next_group_sync(&app));
        assert_eq!(
            update.member_state_changes.get("CCCCCCCC"),
            Some(&(MemberStateChange::Left as i32))
        );
        let synced = update.group.unwrap();
        assert_eq!(
            synced.member_identities.unwrap().identities,
            vec!["BBBBBBBB"]
        );
        assert_eq!(
            (synced.name, synced.user_state, synced.created_at),
            (None, None, None)
        );
        assert!(synced.profile_picture.is_none());

        // Renamed.
        queue(&group_change(true));
        let synced = update_of(next_group_sync(&app)).group.unwrap();
        assert_eq!(synced.name.as_deref(), Some("Crew"));
        assert!(synced.member_identities.is_none());

        // The picture was removed.
        queue(&GroupChange {
            picture_removed: true,
            ..GroupChange::default()
        });
        let synced = update_of(next_group_sync(&app)).group.unwrap();
        assert!(matches!(
            synced.profile_picture.and_then(|picture| picture.image),
            Some(delta_image::Image::Removed(_))
        ));

        // We were removed.
        group.left = true;
        app.store.upsert_group(&group).unwrap();
        app.store.set_group_kicked(&group.key, true).unwrap();
        queue(&GroupChange {
            user_state: true,
            ..GroupChange::default()
        });
        let synced = update_of(next_group_sync(&app)).group.unwrap();
        assert_eq!(
            synced.user_state,
            Some(libthreema::protobuf::d2d_sync::group::UserState::Kicked as i32)
        );
        assert!(synced.member_identities.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn no_group_sync_names_someone_before_their_contact_has_gone_out() {
        use prost::Message as _;
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        group.members.push("DDDDDDDD".into());
        group.members.push("EEEEEEEE".into());
        app.store.upsert_group(&group).unwrap();

        // The new group goes out now, without the two who have no contact yet.
        let change = GroupChange {
            created: true,
            ..GroupChange::default()
        };
        let unknown = ["DDDDDDDD".to_owned(), "EEEEEEEE".to_owned()];
        assert!(queue_received_group_change(&app.store, &group.key, &change, &unknown).unwrap());
        let group_sync::Action::Create(create) = next_group_sync(&app) else {
            panic!("not a create");
        };
        let synced = create.group.unwrap();
        assert_eq!(synced.name.as_deref(), Some("Crew"));
        assert_eq!(
            synced.member_identities.unwrap().identities,
            vec!["BBBBBBBB", "CCCCCCCC"]
        );
        assert!(synced.profile_picture.is_none());
        assert!(app.store.next_sync().unwrap().is_none());
        assert_eq!(app.store.outstanding_member_lookups().unwrap(), unknown);

        // The lookup finds one of them and doesn't know the other: first the contact, then the
        // list with them in it, and nothing is left waiting.
        let found = crate::contacts::test_contact("DDDDDDDD");
        assert!(
            crate::sync::members_looked_up(
                &app.store,
                vec![
                    (
                        "DDDDDDDD".to_owned(),
                        crate::contacts::Lookup::Found(Box::new(found))
                    ),
                    ("EEEEEEEE".to_owned(), crate::contacts::Lookup::Invalid),
                ],
            )
            .unwrap()
            .queued
        );
        let (id, scope, content) = app.store.next_sync().unwrap().unwrap();
        assert_eq!(scope, Scope::ContactSync as i32);
        let envelope = libthreema::protobuf::d2d::Envelope::decode(content.as_slice()).unwrap();
        assert!(matches!(
            envelope.content,
            Some(Content::ContactSync(ContactSync {
                action: Some(contact_sync::Action::Create(_))
            }))
        ));
        app.store.sync_done(id).unwrap();
        let update = update_of(next_group_sync(&app));
        // A new group's members didn't "come": the map says nothing about them.
        assert!(update.member_state_changes.is_empty());
        assert_eq!(
            update.group.unwrap().member_identities.unwrap().identities,
            vec!["BBBBBBBB", "CCCCCCCC", "DDDDDDDD"]
        );
        assert!(app.store.next_sync().unwrap().is_none());
        assert!(app.store.outstanding_member_lookups().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn someone_added_gets_their_line_once_their_contact_is_there() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        group.members.push("DDDDDDDD".into());
        app.store.upsert_group(&group).unwrap();
        let mut change = GroupChange {
            members: true,
            ..GroupChange::default()
        };
        change
            .member_changes
            .insert("DDDDDDDD".into(), MemberChange::Added);
        change
            .member_changes
            .insert("CCCCCCCC".into(), MemberChange::Kicked);
        let unknown = ["DDDDDDDD".to_owned()];
        queue_received_group_change(&app.store, &group.key, &change, &unknown).unwrap();
        // Who went goes out now, who came doesn't yet.
        let update = update_of(next_group_sync(&app));
        assert_eq!(
            update.member_state_changes,
            [("CCCCCCCC".to_owned(), MemberStateChange::Kicked as i32)]
                .into_iter()
                .collect()
        );
        let found = crate::contacts::test_contact("DDDDDDDD");
        crate::sync::members_looked_up(
            &app.store,
            vec![(
                "DDDDDDDD".to_owned(),
                crate::contacts::Lookup::Found(Box::new(found)),
            )],
        )
        .unwrap();
        let (id, ..) = app.store.next_sync().unwrap().unwrap();
        app.store.sync_done(id).unwrap();
        assert_eq!(
            update_of(next_group_sync(&app)).member_state_changes,
            [("DDDDDDDD".to_owned(), MemberStateChange::Added as i32)]
                .into_iter()
                .collect()
        );
    }

    /// What the connection would reflect next is a new contact: whose, removed from the queue.
    fn next_contact_create(app: &App) -> String {
        use prost::Message as _;
        let (id, scope, content) = app.store.next_sync().unwrap().unwrap();
        app.store.sync_done(id).unwrap();
        assert_eq!(scope, Scope::ContactSync as i32);
        let envelope = libthreema::protobuf::d2d::Envelope::decode(content.as_slice()).unwrap();
        let Some(Content::ContactSync(sync)) = envelope.content else {
            panic!("not a contact sync");
        };
        let Some(contact_sync::Action::Create(create)) = sync.action else {
            panic!("not a new contact");
        };
        create.contact.unwrap().identity
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_contact_any_lookup_stores_goes_to_the_other_devices() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let found = crate::contacts::test_contact("DDDDDDDD");
        // Nobody noted waiting for them, as for a sync request's creator: no list to go out.
        assert!(
            crate::sync::members_looked_up(
                &app.store,
                vec![(
                    "DDDDDDDD".to_owned(),
                    crate::contacts::Lookup::Found(Box::new(found)),
                )],
            )
            .unwrap()
            .queued
        );
        assert!(app.store.contact("DDDDDDDD").unwrap().is_some());
        assert_eq!(next_contact_create(&app), "DDDDDDDD");
        assert!(app.store.next_sync().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_member_found_by_the_retry_gets_into_the_list_for_the_other_devices() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        group.members.push("DDDDDDDD".to_owned());
        app.store.upsert_group(&group).unwrap();

        app.store.note_unresolved_member_lookups().unwrap();
        let found = crate::contacts::test_contact("DDDDDDDD");
        crate::sync::members_looked_up(
            &app.store,
            vec![(
                "DDDDDDDD".to_owned(),
                crate::contacts::Lookup::Found(Box::new(found)),
            )],
        )
        .unwrap();

        assert_eq!(next_contact_create(&app), "DDDDDDDD");
        let update = update_of(next_group_sync(&app));
        assert!(
            update
                .group
                .unwrap()
                .member_identities
                .unwrap()
                .identities
                .contains(&"DDDDDDDD".to_owned())
        );
        // It was in the group all along, so nobody came.
        assert!(update.member_state_changes.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_owed_picture_that_is_gone_is_no_longer_owed() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        group.picture_version = 2;
        app.store.store_received_group_picture(&group).unwrap();
        assert_eq!(
            app.store.group_picture_syncs().unwrap(),
            vec![group.key.clone()]
        );
        // Removed before the upload: nothing to upload, no network, nothing queued.
        reflect_group_picture(&app.store, &app.https, &app.media, &group.key, None).await;
        assert!(app.store.group_picture_syncs().unwrap().is_empty());
        assert!(app.store.next_sync().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn two_tries_for_a_picture_send_it_once_and_a_newer_one_keeps_its_note() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let mut group = foreign_group(&app);
        group.picture_version = 2;
        app.store.store_received_group_picture(&group).unwrap();
        let synced = crate::sync::group_only(&group);

        // An upload of an older picture finishes: not sent, and the newer one is still owed.
        assert!(!queue_owed_picture(&app.store, &group.key, 1, synced.clone()).unwrap());
        assert!(app.store.next_sync().unwrap().is_none());
        assert_eq!(
            app.store.group_picture_syncs().unwrap(),
            vec![group.key.clone()]
        );

        assert!(queue_owed_picture(&app.store, &group.key, 2, synced.clone()).unwrap());
        assert!(app.store.group_picture_syncs().unwrap().is_empty());
        let (id, ..) = app.store.next_sync().unwrap().unwrap();
        app.store.sync_done(id).unwrap();
        // The second try of the same picture: already sent.
        assert!(!queue_owed_picture(&app.store, &group.key, 2, synced).unwrap());
        assert!(app.store.next_sync().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_deleted_pictures_file_is_not_sent_to_the_others() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let group = foreign_group(&app);
        assert_eq!(group.picture_version, 0);
        // A try still under way or a removal that didn't finish: the file is still there.
        app.media.write_picture(&group.key, b"old").unwrap();
        app.store.store_received_group_picture(&group).unwrap();
        reflect_group_picture(&app.store, &app.https, &app.media, &group.key, None).await;
        assert!(app.store.group_picture_syncs().unwrap().is_empty());
        assert!(app.store.next_sync().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nothing_is_reflected_without_a_device_group() {
        let (_dir, app) = app();
        with_identity(&app);
        let group = foreign_group(&app);
        assert!(
            !queue_received_group_change(&app.store, &group.key, &group_change(true), &[]).unwrap()
        );
        assert!(app.store.next_sync().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn changing_members_here_says_who_came_and_went() {
        let (_dir, mut app) = app();
        with_identity(&app);
        for id in ["BBBBBBBB", "CCCCCCCC"] {
            app.store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        let created: Value = serde_json::from_str(
            &app.handle(
                &json!({"id": 1, "cmd": "createGroup", "name": "Crew", "members": ["BBBBBBBB"]})
                    .to_string(),
            )
            .await,
        )
        .unwrap();
        assert_eq!(created["ok"], true, "{created}");
        let key = created["result"]["key"].as_str().unwrap().to_owned();
        let _rx = in_a_group(&mut app);

        let change =
            json!({"id": 2, "cmd": "setGroupMembers", "key": key, "members": ["CCCCCCCC"]});
        let reply: Value = serde_json::from_str(&app.handle(&change.to_string()).await).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        let update = update_of(next_group_sync(&app));
        assert_eq!(
            update.member_state_changes,
            [
                ("CCCCCCCC".to_owned(), MemberStateChange::Added as i32),
                ("BBBBBBBB".to_owned(), MemberStateChange::Kicked as i32),
            ]
            .into_iter()
            .collect()
        );
        assert_eq!(
            update.group.unwrap().member_identities.unwrap().identities,
            vec!["CCCCCCCC"]
        );

        // A rename says nobody came or went.
        let rename = json!({"id": 3, "cmd": "renameGroup", "key": key, "name": "Ship"});
        app.handle(&rename.to_string()).await;
        assert!(
            update_of(next_group_sync(&app))
                .member_state_changes
                .is_empty()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn marking_someone_elses_group_sends_only_the_mark() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let _rx = in_a_group(&mut app);
        let group = foreign_group(&app);
        let pin = json!({"id": 1, "cmd": "setPinned", "identity": group.key, "pinned": true});
        let reply: Value = serde_json::from_str(&app.handle(&pin.to_string()).await).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        let synced = update_of(next_group_sync(&app)).group.unwrap();
        assert!(synced.conversation_visibility.is_some());
        assert!(synced.member_identities.is_none());
        assert_eq!(
            (synced.name, synced.user_state, synced.created_at),
            (None, None, None)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_contact_made_direct_by_writing_reaches_the_other_devices() {
        let (_dir, mut app) = app();
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level =
            libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel::GroupOrDeleted;
        app.store.upsert_contact(&contact).unwrap();
        let mut rx = in_a_group(&mut app);
        let send = json!({"id": 1, "cmd": "sendText", "identity": "BBBBBBBB", "text": "hello"});
        let reply: Value = serde_json::from_str(&app.handle(&send.to_string()).await).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        let mut reflected = false;
        while let Ok(message) = rx.try_recv() {
            reflected |= matches!(message, ToConnection::ReflectSync);
        }
        assert!(reflected);

        // Nothing changed: nothing to tell.
        let send = json!({"id": 2, "cmd": "sendText", "identity": "BBBBBBBB", "text": "again"});
        app.handle(&send.to_string()).await;
        while let Ok(message) = rx.try_recv() {
            assert!(!matches!(message, ToConnection::ReflectSync));
        }
    }

    /// Writing in an archived chat brings it back here, and on the other devices.
    #[tokio::test(flavor = "current_thread")]
    async fn a_chat_written_in_leaves_the_archive_everywhere() {
        let (_dir, mut app) = app();
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store.set_archived("BBBBBBBB", true).unwrap();
        let mut rx = in_a_group(&mut app);
        let send = json!({"id": 1, "cmd": "sendText", "identity": "BBBBBBBB", "text": "hello"});
        let reply: Value = serde_json::from_str(&app.handle(&send.to_string()).await).unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(app.store.archived_chats().unwrap().is_empty());
        let mut reflected = false;
        while let Ok(message) = rx.try_recv() {
            reflected |= matches!(message, ToConnection::ReflectSync);
        }
        assert!(reflected);

        // Nothing changed: nothing to tell.
        let send = json!({"id": 2, "cmd": "sendText", "identity": "BBBBBBBB", "text": "again"});
        app.handle(&send.to_string()).await;
        while let Ok(message) = rx.try_recv() {
            assert!(!matches!(message, ToConnection::ReflectSync));
        }
    }

    fn device(id: u64, current: bool) -> LinkedDevice {
        LinkedDevice {
            id: format!("{id:016x}"),
            raw_id: id,
            current,
            label: String::new(),
            platform: "desktop",
            platform_details: String::new(),
            app_version: String::new(),
            connected_since: None,
            last_disconnect_at: None,
        }
    }

    /// A connection waiting for its next try, as `connection::run` answers then.
    async fn offline(mut rx: mpsc::UnboundedReceiver<ToConnection>) {
        while let Some(message) = rx.recv().await {
            match message {
                ToConnection::Devices(reply) => {
                    let _ = reply.send(Err(DeviceGroupError::NotConnected));
                }
                ToConnection::DropDevices { reply, .. } => {
                    let _ = reply.send(Err(DeviceGroupError::NotConnected));
                }
                _ => {}
            }
        }
    }

    fn with_identity(app: &App) {
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([7; 32]),
                server_group: "a".into(),
            })
            .unwrap();
    }

    /// Removing the ID leaves the group first, while the ID is still there for the connection,
    /// and its mirror stops being used.
    #[tokio::test(flavor = "current_thread")]
    async fn removing_the_id_drops_the_linked_devices_first() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let mut rx = in_a_group(&mut app);
        app.apply_multi_device();
        let group = app
            .store
            .multi_device()
            .unwrap()
            .and_then(|properties| properties.device_group_key())
            .unwrap()
            .device_group_id();
        let store = std::rc::Rc::clone(&app.store);
        let local = tokio::task::LocalSet::new();
        let result = local
            .run_until(async {
                let connection = tokio::task::spawn_local(async move {
                    let Some(ToConnection::Flush(flushed)) = rx.recv().await else {
                        panic!("no wait for what was queued");
                    };
                    flushed.send(()).unwrap();
                    let Some(ToConnection::DropDevices { ids: None, reply }) = rx.recv().await
                    else {
                        panic!("no request to remove all the others");
                    };
                    assert!(store.identity().unwrap().is_some(), "not wiped yet");
                    let _ = reply.send(Ok(DropOutcome {
                        remaining: vec![device(1, true)],
                        left_group: true,
                    }));
                });
                let result = app.delete_all_data().await.unwrap();
                connection.await.unwrap();
                result
            })
            .await;
        assert_eq!(result["linkedDevicesRemain"], false);
        assert!(app.store.identity().unwrap().is_none());
        assert!(app.store.multi_device().unwrap().is_none());
        assert_ne!(crate::media::mirror_device_group(), Some(group));
    }

    /// Without a connection the ID goes all the same, and the UI gets to say what stayed.
    #[tokio::test(flavor = "current_thread")]
    async fn removing_the_id_offline_says_the_devices_stay() {
        let (_dir, mut app) = app();
        with_identity(&app);
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        let result = app.delete_all_data().await.unwrap();
        assert_eq!(result["linkedDevicesRemain"], true);
        assert_eq!(crate::runtime::removal_mark(&app.data_dir), Some(true));
        assert!(app.store.identity().unwrap().is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn offline_is_a_network_error_not_an_internal_one() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        assert_eq!(app.linked_devices().await.unwrap_err().kind(), "network");
        let id = format!("{:016x}", 7);
        assert_eq!(app.drop_devices(&[id]).await.unwrap_err().kind(), "network");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_refusal_keeps_its_own_kind() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        tokio::spawn(async move {
            if let Some(ToConnection::Devices(reply)) = rx.recv().await {
                let _ = reply.send(Err(DeviceGroupError::Failed(
                    "expected devices info".into(),
                )));
            }
        });
        let error = app.linked_devices().await.unwrap_err();
        assert_ne!(error.kind(), "network");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_connection_that_is_gone_is_a_network_error_too() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        drop(rx);
        assert_eq!(app.linked_devices().await.unwrap_err().kind(), "network");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removing_devices_needs_the_mediators_word() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        let error = app.deactivate_multi_device(false).await.unwrap_err();
        assert_eq!(error.kind(), "network");
        assert!(
            app.store.multi_device().unwrap().is_some(),
            "still in the group, to remove the devices later"
        );
        assert_eq!(
            app.take_ui_events(),
            ["deviceRemovalFailed"],
            "heard even if the page that asked is gone"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_failed_removal_of_one_device_is_heard_too() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        let id = format!("{:016x}", 7);
        app.drop_devices(&[id]).await.unwrap_err();
        assert_eq!(app.take_ui_events(), ["deviceRemovalFailed"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn leaving_on_this_device_alone_is_a_choice() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let result = app.deactivate_multi_device(true).await.unwrap();
        assert_eq!(result["active"], false);
        assert!(app.store.multi_device().unwrap().is_none());
        assert!(
            matches!(rx.try_recv(), Ok(ToConnection::Reconnect)),
            "nothing but the way back to the chat server"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removing_all_lets_the_connection_decide_from_its_list() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let store = std::rc::Rc::clone(&app.store);
        let connection = async {
            let Some(ToConnection::DropDevices { ids, reply }) = rx.recv().await else {
                panic!("expected a drop");
            };
            assert!(ids.is_none(), "all there are, not a list from before");
            store.set_multi_device(None).unwrap();
            let _ = reply.send(Ok(DropOutcome {
                remaining: Vec::new(),
                left_group: true,
            }));
        };
        let (result, ()) = tokio::join!(app.deactivate_multi_device(false), connection);
        assert_eq!(result.unwrap()["active"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removing_the_last_device_reports_multi_device_over() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let store = std::rc::Rc::clone(&app.store);
        let connection = async {
            let Some(ToConnection::DropDevices { ids, reply }) = rx.recv().await else {
                panic!("expected a drop");
            };
            assert_eq!(ids, Some(vec![7]));
            store.set_multi_device(None).unwrap();
            let _ = reply.send(Ok(DropOutcome {
                remaining: Vec::new(),
                left_group: true,
            }));
        };
        let id = format!("{:016x}", 7);
        let ids = [id];
        let (result, ()) = tokio::join!(app.drop_devices(&ids), connection);
        let result = result.unwrap();
        assert_eq!(result["active"], false);
        assert_eq!(result["devices"], json!([]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn removing_one_of_two_returns_who_remains() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        app.set_device_alias(&format!("{:016x}", 8), "Laptop")
            .unwrap();
        let connection = async {
            let Some(ToConnection::DropDevices { reply, .. }) = rx.recv().await else {
                panic!("expected a drop");
            };
            let _ = reply.send(Ok(DropOutcome {
                remaining: vec![device(1, true), device(8, false)],
                left_group: false,
            }));
        };
        let id = format!("{:016x}", 7);
        let ids = [id];
        let (result, ()) = tokio::join!(app.drop_devices(&ids), connection);
        let result = result.unwrap();
        assert_eq!(result["active"], true);
        let devices = result["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[1]["alias"], "Laptop");
        assert!(devices[0].get("rawId").is_none(), "hex ids only for the UI");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_page_finding_only_this_device_checks_whether_it_is_alone() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let connection = async {
            let Some(ToConnection::Devices(reply)) = rx.recv().await else {
                panic!("expected the list");
            };
            let _ = reply.send(Ok(vec![device(1, true)]));
        };
        let (result, ()) = tokio::join!(app.linked_devices_checked(), connection);
        result.unwrap();
        assert!(matches!(
            rx.try_recv(),
            Ok(ToConnection::DropDevices { ids: Some(ids), .. }) if ids.is_empty()
        ));
        // With another device, nothing.
        let connection = async {
            let Some(ToConnection::Devices(reply)) = rx.recv().await else {
                panic!("expected the list");
            };
            let _ = reply.send(Ok(vec![device(1, true), device(2, false)]));
        };
        let (result, ()) = tokio::join!(app.linked_devices_checked(), connection);
        result.unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_periodic_check_runs_about_hourly_and_only_with_multi_device() {
        let (_dir, mut app) = app();
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.connection = Some(tx);
        app.check_device_group();
        assert!(rx.try_recv().is_err(), "no device group, nothing to check");
        app.store
            .set_multi_device(Some(&Properties::generate()))
            .unwrap();
        app.last_group_check = None;
        app.check_device_group();
        assert!(matches!(
            rx.try_recv(),
            Ok(ToConnection::DropDevices { ids: Some(ids), .. }) if ids.is_empty()
        ));
        app.check_device_group();
        assert!(rx.try_recv().is_err(), "not again so soon");
    }

    fn before_handover() -> JoinOutcome {
        JoinOutcome::FailedBeforeHandover("not connected".into())
    }

    fn after_handover() -> JoinOutcome {
        JoinOutcome::FailedAfterHandover("connection timed out".into())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_first_linking_failing_before_the_handover_rolls_back() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let error = app
            .finish_linking(before_handover(), false)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "linkFailed");
        assert!(app.store.multi_device().unwrap().is_none());
        assert!(matches!(rx.try_recv(), Ok(ToConnection::Reconnect)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn another_failing_before_the_handover_leaves_the_group_alone() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let error = app
            .finish_linking(before_handover(), true)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "linkFailed");
        assert!(app.store.multi_device().unwrap().is_some());
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn after_the_handover_the_new_device_is_removed_first() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let store = std::rc::Rc::clone(&app.store);
        let connection = async {
            let Some(ToConnection::DropDevices { ids, reply }) = rx.recv().await else {
                panic!("expected a drop");
            };
            assert!(ids.is_none(), "whatever is in the group by now");
            store.set_multi_device(None).unwrap();
            let _ = reply.send(Ok(DropOutcome {
                remaining: Vec::new(),
                left_group: true,
            }));
        };
        let (result, ()) = tokio::join!(app.finish_linking(after_handover(), false), connection);
        assert_eq!(result.unwrap_err().kind(), "linkRemoved");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn after_the_handover_this_device_stays_in_charge_if_removing_fails() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        let error = app
            .finish_linking(after_handover(), false)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "linkUncertain");
        assert!(
            app.store.multi_device().unwrap().is_some(),
            "a new device may be in the group, and only a member can remove it"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn after_the_handover_an_existing_group_keeps_its_devices() {
        let (_dir, mut app) = app();
        let mut rx = in_a_group(&mut app);
        let error = app
            .finish_linking(after_handover(), true)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), "linkUncertain");
        assert!(app.store.multi_device().unwrap().is_some());
        assert!(rx.try_recv().is_err(), "the other devices stay");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_join_that_worked_stays_one_when_the_connection_then_drops() {
        let (_dir, mut app) = app();
        let rx = in_a_group(&mut app);
        tokio::spawn(offline(rx));
        let result = app.finish_linking(JoinOutcome::Joined, false).await;
        assert_eq!(result.unwrap()["active"], true);
        assert!(app.store.multi_device().unwrap().is_some());
    }

    /// A deleted contact is out of the contact list on the other devices too.
    #[test]
    fn deleting_a_contact_is_reflected_as_its_level() {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level = AcquaintanceLevel::Direct;
        app.store.upsert_contact(&contact).unwrap();

        let reply = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle(r#"{"id":1,"cmd":"deleteContact","identity":"BBBBBBBB"}"#));
        assert!(reply.contains(r#""ok":true"#), "{reply}");

        // First the exclusion list, then the contact.
        let (id, scope, _) = app.store.next_sync().unwrap().unwrap();
        assert_eq!(scope, Scope::SettingsSync as i32);
        app.store.sync_done(id).unwrap();
        let synced = next_contact_sync(&app);
        assert_eq!(
            synced.acquaintance_level,
            Some(AcquaintanceLevel::GroupOrDeleted as i32)
        );
    }

    /// However the ID was typed: the command understands it, and so does what tells the others.
    #[test]
    fn an_id_typed_with_spaces_or_in_lower_case_is_still_reflected() {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let (_dir, mut app) = app();
        with_identity(&app);
        let _connection = in_a_group(&mut app);
        let mut contact = crate::contacts::test_contact("BBBBBBBB");
        contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        app.store.upsert_contact(&contact).unwrap();
        let block_on = |app: &mut App, request: &str| {
            let reply = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(app.handle(request));
            assert!(reply.contains(r#""ok":true"#), "{reply}");
        };

        block_on(
            &mut app,
            r#"{"id":1,"cmd":"addContact","identity":" bbbbbbbb "}"#,
        );
        let synced = next_contact_sync(&app);
        assert_eq!(synced.identity, "BBBBBBBB");
        assert_eq!(
            synced.acquaintance_level,
            Some(AcquaintanceLevel::Direct as i32)
        );

        block_on(
            &mut app,
            r#"{"id":2,"cmd":"updateContact","identity":" bbbbbbbb ","firstName":"Ann"}"#,
        );
        assert_eq!(next_contact_sync(&app).first_name.as_deref(), Some("Ann"));
    }

    /// Scanning someone's code: verified on the other devices too.
    #[test]
    fn a_scanned_code_reaches_the_other_devices() {
        use libthreema::protobuf::d2d_sync::contact::VerificationLevel;
        let (_dir, mut app) = app();
        with_identity(&app);
        let _connection = in_a_group(&mut app);
        let contact = crate::contacts::test_contact("BBBBBBBB");
        app.store.upsert_contact(&contact).unwrap();
        let key = contact.public_key.to_string();
        let scan = json!({"id": 1, "cmd": "verifyQrCode", "text": format!("3mid:BBBBBBBB,{key}")});
        let reply = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle(&scan.to_string()));
        assert!(reply.contains(r#""ok":true"#), "{reply}");
        assert_eq!(
            next_contact_sync(&app).verification_level,
            Some(VerificationLevel::FullyVerified as i32)
        );
    }

    /// New here, so to the others too: they ignore an update of someone they don't know.
    #[tokio::test(flavor = "current_thread")]
    async fn someone_new_goes_out_as_a_new_contact() {
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.new_contacts.borrow_mut().push("BBBBBBBB".to_owned());
        // Any command: `handle` tells the others of what it did.
        app.handle(r#"{"id":1,"cmd":"contacts"}"#).await;
        assert_eq!(next_contact_create(&app), "BBBBBBBB");
        assert!(app.store.next_sync().unwrap().is_none());
    }

    /// One command, many contacts: each of those it changed, and only those.
    #[tokio::test(flavor = "current_thread")]
    async fn clearing_individual_receipt_settings_reaches_the_other_devices() {
        use libthreema::protobuf::d2d_sync::contact::read_receipt_policy_override::Override;
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        let mut own_choice = crate::contacts::test_contact("BBBBBBBB");
        own_choice.read_receipt_policy_override = crate::contacts::read_receipt_policy(1);
        assert!(own_choice.read_receipt_policy_override.is_some());
        app.store.upsert_contact(&own_choice).unwrap();
        app.store
            .upsert_contact(&crate::contacts::test_contact("CCCCCCCC"))
            .unwrap();
        let reply = app.handle(r#"{"id":1,"cmd":"clearContactPolicies"}"#).await;
        assert!(reply.contains(r#""cleared":1"#), "{reply}");
        let synced = next_contact_sync(&app);
        assert_eq!(synced.identity, "BBBBBBBB");
        assert!(matches!(
            synced
                .read_receipt_policy_override
                .and_then(|o| o.r#override),
            Some(Override::Default(_))
        ));
        assert!(app.store.next_sync().unwrap().is_none());
    }

    /// A pin is kept on the other devices too, so it goes there with the chat.
    #[test]
    fn deleting_a_pinned_chat_is_reflected_as_its_visibility() {
        use libthreema::protobuf::d2d_sync::ConversationVisibility;
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        app.store
            .upsert_contact(&crate::contacts::test_contact("BBBBBBBB"))
            .unwrap();
        app.store.set_pinned("BBBBBBBB", true).unwrap();

        let reply = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app.handle(r#"{"id":1,"cmd":"deleteConversation","identity":"BBBBBBBB"}"#));
        assert!(reply.contains(r#""ok":true"#), "{reply}");

        let synced = next_contact_sync(&app);
        assert_eq!(
            synced.conversation_visibility,
            Some(ConversationVisibility::Normal as i32)
        );
    }

    /// Android's order: the other devices hear of it, then the row goes.
    #[test]
    fn deleting_a_left_group_is_reflected_and_a_joined_ones_chat_is_not() {
        use libthreema::protobuf::common::GroupIdentity;
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let group = |id: u64, left: bool| {
            let identity = libthreema::common::GroupIdentity {
                creator_identity: "ABCD1234".parse().unwrap(),
                group_id: id,
            };
            Group {
                key: crate::groups::key(&identity),
                identity,
                name: String::new(),
                members: Vec::new(),
                left,
                created_at: 1,
                picture: None,
                picture_version: 0,
            }
        };
        let (left, joined) = (group(1, true), group(2, false));
        app.store.upsert_group(&left).unwrap();
        app.store.upsert_group(&joined).unwrap();

        app.delete_conversation(&joined.key).unwrap();
        assert!(app.store.next_sync().unwrap().is_none());
        assert!(app.store.group(&joined.key).unwrap().is_some());

        app.delete_conversation(&left.key).unwrap();
        let group_sync::Action::Delete(deleted) = next_group_sync(&app) else {
            panic!("not a deletion");
        };
        assert_eq!(
            deleted.group_identity,
            Some(GroupIdentity {
                group_id: 1,
                creator_identity: "ABCD1234".into(),
            })
        );
        assert!(app.store.group(&left.key).unwrap().is_none());
    }

    /// A group the others never got (its creator isn't known here) isn't deleted there.
    #[test]
    fn a_group_the_others_never_got_is_not_reflected_as_deleted() {
        let (_dir, mut app) = app();
        let _connection = in_a_group(&mut app);
        app.store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "ABCD1234".into(),
                client_key: zeroize::Zeroizing::new([3; 32]),
                server_group: "a1".into(),
            })
            .unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 1,
        };
        let group = Group {
            key: crate::groups::key(&identity),
            identity,
            name: String::new(),
            members: Vec::new(),
            left: true,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        app.store.upsert_group(&group).unwrap();
        app.delete_conversation(&group.key).unwrap();
        assert!(app.store.next_sync().unwrap().is_none());
    }
}
