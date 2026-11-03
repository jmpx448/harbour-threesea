// SPDX-License-Identifier: AGPL-3.0-only
//! Multi-device commands (ADR 0011): state, linked devices, removing devices, turning it on
//! (from linking) and off.

use serde_json::{Value, json};
use tokio::sync::oneshot;

use crate::{
    app::{ApiError, App},
    connection::ToConnection,
    multidevice::{Properties, SETTING_FS_TERMINATE},
};

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
                        let encrypted =
                            media::download(&https, &blob.id, media::BlobScope::Local).await?;
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
        let devices = answer
            .await
            .map_err(|_| ApiError::Internal("connection gone".into()))?
            .map_err(ApiError::Internal)?;
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
        Ok(json!({ "devices": devices, "label": self.device_label()? }))
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

    /// `dropDevices`: remove devices (hex ids) from the group.
    pub(crate) async fn drop_devices(&mut self, ids: &[String]) -> Result<Value, ApiError> {
        let ids = ids
            .iter()
            .map(|id| u64::from_str_radix(id, 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ApiError::Internal("bad device id".into()))?;
        let (reply, answer) = oneshot::channel();
        self.tell_connection(ToConnection::DropDevices { ids, reply });
        answer
            .await
            .map_err(|_| ApiError::Internal("connection gone".into()))?
            .map_err(ApiError::Internal)?;
        self.linked_devices().await
    }

    /// `deactivateMultiDevice`: remove every other device, then this one (Android's
    /// `DeactivateMultiDeviceTask`); the connection goes back to the chat server. When the
    /// mediator cannot be reached, this device still leaves the group locally and reconnects
    /// directly — the phone must never stay trapped behind an unreachable mediator (user
    /// report 2026-10-28: offline until the ID was set up anew). Other devices' slots can
    /// then only be dropped by those devices.
    pub(crate) async fn deactivate_multi_device(&mut self) -> Result<Value, ApiError> {
        let Some(properties) = self.store.multi_device()? else {
            return self.multi_device_json();
        };
        let own = properties.mediator_device_id;
        let dropped = async {
            let devices = self.linked_devices().await?;
            let mut ids: Vec<String> = devices["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|device| device["current"] != true)
                .filter_map(|device| device["id"].as_str().map(str::to_owned))
                .collect();
            // Ourselves last: that ends the connection through the mediator.
            ids.push(format!("{own:016x}"));
            let ids = ids
                .iter()
                .map(|id| u64::from_str_radix(id, 16))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| ApiError::Internal("bad device id".into()))?;
            let (reply, answer) = oneshot::channel();
            self.tell_connection(ToConnection::DropDevices { ids, reply });
            tokio::time::timeout(std::time::Duration::from_secs(30), answer)
                .await
                .map_err(|_| ApiError::Internal("timeout".into()))?
                .map_err(|_| ApiError::Internal("connection gone".into()))?
                .map_err(ApiError::Internal)
        }
        .await;
        if let Err(error) = dropped {
            tracing::warn!(
                kind = error.kind(),
                "dropping the device slots failed; leaving the device group locally"
            );
        }
        self.leave_device_group_locally().await;
        self.multi_device_json()
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
    /// If the join fails and multi-device was activated just for it (no other device ever
    /// joined), the activation is rolled back: without that, a failed first linking left the
    /// phone alone behind the mediator — offline for good when the mediator connection was
    /// the problem (user report 2026-10-28).
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
        // Above the connection's own bounds (3 minutes for the new device's registration), so
        // this never gives up while the transfer can still succeed.
        let result = match tokio::time::timeout(std::time::Duration::from_secs(200), answer).await {
            Err(_) => Err(ApiError::LinkFailed("timeout".into())),
            Ok(Err(_)) => Err(ApiError::LinkFailed("connection gone".into())),
            Ok(Ok(Err(error))) => Err(ApiError::LinkFailed(error)),
            Ok(Ok(Ok(()))) => Ok(()),
        };
        if let Err(error) = result {
            if !was_active {
                tracing::warn!("linking failed; leaving the new device group again");
                self.leave_device_group_locally().await;
            }
            return Err(error);
        }
        self.linked_devices().await
    }

    /// After a command that changed something other devices keep too: reflect it.
    pub(crate) async fn reflect_change(&self, change: SyncChange, result: &Value) {
        use libthreema::protobuf::d2d::{
            ContactSync, GroupSync, SettingsSync, UserProfileSync, contact_sync, envelope::Content,
            group_sync, settings_sync, transaction_scope::Scope, user_profile_sync,
        };
        if !matches!(self.store.multi_device(), Ok(Some(_))) {
            return;
        }
        let built = (|| -> Result<Vec<(Scope, Content)>, crate::store::StoreError> {
            Ok(match change {
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
                SyncChange::Group { key, created } => {
                    let key = key.or_else(|| result["key"].as_str().map(str::to_owned));
                    let Some(group) = key.and_then(|key| self.store.group(&key).ok().flatten())
                    else {
                        return Ok(Vec::new());
                    };
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
                            member_state_changes: std::collections::HashMap::new(),
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
                            .map_or_else(Vec::new, |group| {
                                crate::join::group_sync(&self.store, &group)
                                    .map(|mut synced| {
                                        synced.profile_picture = None;
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
                    let mut synced = crate::join::group_sync(&self.store, &group)?;
                    synced.name = None;
                    synced.member_identities = None;
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
                match self.upload_sync_picture(&target).await {
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
                use prost::Message as _;
                for (scope, content) in items {
                    #[expect(deprecated, reason = "Padding is still part of the envelope")]
                    let envelope = libthreema::protobuf::d2d::Envelope {
                        padding: Vec::new(),
                        device_id: 0,
                        protocol_version: 0,
                        content: Some(content),
                    };
                    if let Err(error) = self
                        .store
                        .queue_sync(scope as i32, &envelope.encode_to_vec())
                    {
                        tracing::warn!(%error, "sync not queued");
                    }
                }
                self.tell_connection(ToConnection::ReflectSync);
            }
            Err(error) => tracing::warn!(%error, "sync not reflected"),
        }
    }
}

impl App {
    /// The picture stored as `target`, encrypted and uploaded to the mirror (local scope) as a
    /// sync `DeltaImage`; `None` if there is none (the "removed" placeholder stays).
    async fn upload_sync_picture(
        &self,
        target: &str,
    ) -> Result<Option<libthreema::protobuf::common::DeltaImage>, crate::media::MediaError> {
        use libthreema::protobuf::common::{Blob, DeltaImage, Image, delta_image, image};
        let Some(data) = self.media.read_picture(target)? else {
            return Ok(None);
        };
        let key: [u8; 32] = rand::random();
        let nonce = *crate::media::MediaPart::File.nonce();
        let encrypted = crate::media::encrypt_blob(&key, crate::media::MediaPart::File, &data);
        let blob_id = crate::media::upload_scoped(
            &self.https,
            &encrypted,
            false,
            crate::media::BlobScope::Local,
        )
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
}

/// What a command changed that other devices keep too (multi-device).
pub(crate) enum SyncChange {
    Contacts(Vec<String>),
    /// A group (`None`: the new group's key from the command's result).
    Group {
        key: Option<String>,
        created: bool,
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
