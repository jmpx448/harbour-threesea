// SPDX-License-Identifier: AGPL-3.0-only
//! Device linking, second half (ADR 0011): the essential data a new device (Threema Desktop)
//! needs to join the device group, collected from the store.
//!
//! Upstream: Android's `DeviceLinkingDataCollector`. Pictures go along as `common.BlobData`
//! (plain JPEG; the rendezvous path is end-to-end encrypted) with local blob ids; the user's own
//! picture keeps its upload (id, key) so the new device can keep distributing it. Distribution
//! lists are not sent (Android: `MD_SYNC_DISTRIBUTION_LISTS` is off). Used nonces go along
//! hashed (HMAC-SHA256, key: the Threema ID).

use std::collections::HashMap;

use libthreema::protobuf::{
    common::{
        Blob, BlobData, DeltaImage, GroupIdentity as ProtoGroupIdentity, Identities, Image, Unit,
        delta_image, image,
    },
    d2d_join::{
        EdToNd, EssentialData, ed_to_nd,
        essential_data::{AugmentedContact, AugmentedGroup, DeviceGroupData, IdentityData},
    },
    d2d_sync::{
        ConversationCategory, ConversationVisibility, Group, ReadReceiptPolicy, Settings,
        TypingIndicatorPolicy, UserProfile, contact, group, settings, user_profile,
    },
};
use prost::Message as _;
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    chat_notifications::Policy,
    media::MediaStore,
    profile_pictures::{OWN_PICTURE, Share, contact_picture_name},
    store::{Store, StoreError, keys},
};

/// The ED-to-ND messages after `Begin`: blobs first, then the essential data (encoded
/// `d2d_join.EdToNd`).
pub struct JoinData {
    pub blobs: Vec<Zeroizing<Vec<u8>>>,
    pub essential_data: Zeroizing<Vec<u8>>,
}

/// What ChaCha20-Poly1305 appends when the rendezvous link encrypts in place.
const ENCRYPTION_TAG_LENGTH: usize = 16;

/// `d2d_join.EdToNd { begin }`.
#[must_use]
pub fn begin_message() -> Vec<u8> {
    EdToNd {
        content: Some(ed_to_nd::Content::Begin(
            libthreema::protobuf::d2d_join::Begin {},
        )),
    }
    .encode_to_vec()
}

/// Whether `data` is `d2d_join.NdToEd { registered }`.
#[must_use]
pub fn is_registered(data: &[u8]) -> bool {
    use libthreema::protobuf::d2d_join::{NdToEd, nd_to_ed};
    NdToEd::decode(data)
        .is_ok_and(|message| matches!(message.content, Some(nd_to_ed::Content::Registered(_))))
}

/// Collects pictures as blob data with sequential local blob ids.
struct Blobs {
    next: u64,
    data: Vec<Zeroizing<Vec<u8>>>,
}

impl Blobs {
    /// A picture as `DeltaImage` (updated), its data queued; `blob` overrides id/key (own
    /// picture's upload).
    fn add(&mut self, data: Zeroizing<Vec<u8>>, blob: Option<Blob>) -> DeltaImage {
        let blob = blob.unwrap_or_else(|| {
            self.next += 1;
            let mut id = vec![0_u8; 16];
            id[8..].copy_from_slice(&self.next.to_be_bytes());
            Blob {
                id,
                ..Blob::default()
            }
        });
        let message = EdToNd {
            content: Some(ed_to_nd::Content::BlobData(BlobData {
                id: blob.id.clone(),
                data: data.to_vec(),
            })),
        };
        self.data.push(Zeroizing::new(message.encode_to_vec()));
        DeltaImage {
            image: Some(delta_image::Image::Updated(Image {
                r#type: image::Type::Jpeg as i32,
                blob: Some(blob),
            })),
        }
    }
}

/// Raw nonces hashed; hashed ones (imported from an Android backup) as they are.
fn hashed_nonces(identity: &str, nonces: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    nonces
        .into_iter()
        .map(|nonce| crate::messages::hash_nonce(identity, &nonce))
        .collect()
}

fn visibility(conversation: &str, pinned: &[String], archived: &[String]) -> i32 {
    let visibility = if pinned.iter().any(|p| p == conversation) {
        ConversationVisibility::Pinned
    } else if archived.iter().any(|a| a == conversation) {
        ConversationVisibility::Archived
    } else {
        ConversationVisibility::Normal
    };
    visibility as i32
}

fn category(store: &Store, conversation: &str) -> Result<i32, StoreError> {
    Ok(if store.is_private(conversation)? {
        ConversationCategory::Protected as i32
    } else {
        ConversationCategory::Default as i32
    })
}

/// The user profile as sync data, the picture as "removed" (the caller adds one if there is).
pub fn profile_sync(store: &Store) -> Result<UserProfile, StoreError> {
    let share_with = match store.picture_share()? {
        Share::Everyone => user_profile::profile_picture_share_with::Policy::Everyone(Unit {}),
        Share::Nobody => user_profile::profile_picture_share_with::Policy::Nobody(Unit {}),
        Share::Selected => {
            user_profile::profile_picture_share_with::Policy::AllowList(Identities {
                identities: store.picture_recipients()?,
            })
        }
    };
    let mut links = Vec::new();
    if let Some(email) = store.setting(keys::LINKED_EMAIL)? {
        links.push(user_profile::identity_links::IdentityLink {
            r#type: Some(user_profile::identity_links::identity_link::Type::Email(
                email,
            )),
            description: String::new(),
        });
    }
    if let Some(mobile) = store.setting(keys::LINKED_MOBILE)? {
        links.push(user_profile::identity_links::IdentityLink {
            r#type: Some(user_profile::identity_links::identity_link::Type::PhoneNumber(mobile)),
            description: String::new(),
        });
    }
    Ok(UserProfile {
        nickname: Some(store.setting(keys::NICKNAME)?.unwrap_or_default()),
        profile_picture: Some(DeltaImage {
            image: Some(delta_image::Image::Removed(Unit {})),
        }),
        profile_picture_share_with: Some(user_profile::ProfilePictureShareWith {
            policy: Some(share_with),
        }),
        identity_links: Some(user_profile::IdentityLinks { links }),
        work_availability_status: None,
    })
}

/// The shared settings as sync data.
pub fn settings_sync(store: &Store) -> Result<Settings, StoreError> {
    let app: crate::safe::backup::Settings = store
        .setting(keys::APP_SETTINGS)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let flag = |on: bool, yes: i32, no: i32| Some(if on { yes } else { no });
    Ok(Settings {
        contact_sync_policy: flag(
            app.sync_contacts,
            settings::ContactSyncPolicy::Sync as i32,
            settings::ContactSyncPolicy::NotSynced as i32,
        ),
        unknown_contact_policy: flag(
            app.block_unknown,
            settings::UnknownContactPolicy::BlockUnknown as i32,
            settings::UnknownContactPolicy::AllowUnknown as i32,
        ),
        read_receipt_policy: flag(
            app.read_receipts,
            ReadReceiptPolicy::SendReadReceipt as i32,
            ReadReceiptPolicy::DontSendReadReceipt as i32,
        ),
        typing_indicator_policy: flag(
            app.send_typing,
            TypingIndicatorPolicy::SendTypingIndicator as i32,
            TypingIndicatorPolicy::DontSendTypingIndicator as i32,
        ),
        o2o_call_policy: flag(
            app.threema_calls,
            settings::O2oCallPolicy::AllowO2oCall as i32,
            settings::O2oCallPolicy::DenyO2oCall as i32,
        ),
        o2o_call_connection_policy: flag(
            app.relay_threema_calls,
            settings::O2oCallConnectionPolicy::RequireRelayedConnection as i32,
            settings::O2oCallConnectionPolicy::AllowDirectConnection as i32,
        ),
        o2o_call_video_policy: flag(
            store.setting(keys::VIDEO_CALLS)?.as_deref() != Some("0"),
            settings::O2oCallVideoPolicy::AllowVideo as i32,
            settings::O2oCallVideoPolicy::DenyVideo as i32,
        ),
        group_call_policy: flag(
            store.setting(keys::GROUP_CALLS)?.as_deref() != Some("0"),
            settings::GroupCallPolicy::AllowGroupCall as i32,
            settings::GroupCallPolicy::DenyGroupCall as i32,
        ),
        screenshot_policy: flag(
            app.disable_screenshots,
            settings::ScreenshotPolicy::DenyScreenshot as i32,
            settings::ScreenshotPolicy::AllowScreenshot as i32,
        ),
        keyboard_data_collection_policy: flag(
            app.incognito_keyboard,
            settings::KeyboardDataCollectionPolicy::DenyDataCollection as i32,
            settings::KeyboardDataCollectionPolicy::AllowDataCollection as i32,
        ),
        blocked_identities: Some(Identities {
            identities: app.blocked_contacts.clone(),
        }),
        exclude_from_sync_identities: Some(Identities {
            identities: app.sync_excluded_ids.clone(),
        }),
    })
}

/// A contact as sync data (conversation state included, pictures left out).
pub fn contact_sync(
    store: &Store,
    stored: &libthreema::model::contact::Contact,
) -> Result<libthreema::protobuf::d2d_sync::Contact, StoreError> {
    let id = stored.identity.as_str().to_owned();
    let pinned = store.pinned_chats()?;
    let archived = store.archived_chats()?;
    let mut synced = stored.to_d2d_sync();
    // Always present, empty when cleared: absent would mean "unchanged" (Android sends them so).
    synced.first_name = Some(stored.first_name.clone().unwrap_or_default());
    synced.last_name = Some(stored.last_name.clone().unwrap_or_default());
    synced.nickname = Some(stored.nickname.clone().unwrap_or_default());
    synced.conversation_category = Some(category(store, &id)?);
    synced.conversation_visibility = Some(visibility(&id, &pinned, &archived));
    synced.notification_trigger_policy_override =
        Some(contact::NotificationTriggerPolicyOverride {
            r#override: Some(match store.chat_notifications(&id)? {
                Some((_, until)) => {
                    contact::notification_trigger_policy_override::Override::Policy(
                        contact::notification_trigger_policy_override::Policy {
                            policy: contact::notification_trigger_policy_override::policy::NotificationTriggerPolicy::Never as i32,
                            expires_at: until.and_then(|u| u64::try_from(u).ok()),
                        },
                    )
                }
                None => contact::notification_trigger_policy_override::Override::Default(
                    Unit {},
                ),
            }),
        });
    Ok(synced)
}

/// A group as sync data (conversation state included, picture as "removed").
pub fn group_sync(store: &Store, stored: &crate::groups::Group) -> Result<Group, StoreError> {
    let pinned = store.pinned_chats()?;
    let archived = store.archived_chats()?;
    let notification = match store.chat_notifications(&stored.key)? {
        Some((policy, until)) => group::notification_trigger_policy_override::Override::Policy(
            group::notification_trigger_policy_override::Policy {
                policy: match policy {
                    Policy::Mentioned => group::notification_trigger_policy_override::policy::NotificationTriggerPolicy::Mentioned as i32,
                    Policy::Never => group::notification_trigger_policy_override::policy::NotificationTriggerPolicy::Never as i32,
                },
                expires_at: until.and_then(|u| u64::try_from(u).ok()),
            },
        ),
        None => group::notification_trigger_policy_override::Override::Default(Unit {}),
    };
    #[expect(deprecated, reason = "Must be provided until phased out")]
    Ok(Group {
        group_identity: Some(ProtoGroupIdentity {
            group_id: stored.identity.group_id,
            creator_identity: stored.identity.creator_identity.as_str().to_owned(),
        }),
        name: Some(stored.name.clone()),
        created_at: Some(u64::try_from(stored.created_at).unwrap_or_default()),
        user_state: Some(if stored.left {
            group::UserState::Left as i32
        } else {
            group::UserState::Member as i32
        }),
        profile_picture: Some(DeltaImage {
            image: Some(delta_image::Image::Removed(Unit {})),
        }),
        member_identities: Some(Identities {
            identities: group_sync_members(store, stored)?,
        }),
        notification_trigger_policy_override: Some(group::NotificationTriggerPolicyOverride {
            r#override: Some(notification),
        }),
        deprecated_notification_sound_policy_override: Some(
            group::DeprecatedNotificationSoundPolicyOverride {
                r#override: Some(
                    group::deprecated_notification_sound_policy_override::Override::Default(
                        Unit {},
                    ),
                ),
            },
        ),
        conversation_category: Some(category(store, &stored.key)?),
        conversation_visibility: Some(visibility(&stored.key, &pinned, &archived)),
    })
}

/// A group's member list for the other devices: without the creator and the user
/// (md-d2d-sync.proto), and without anyone whose lookup hasn't given us a contact yet, as the
/// others only get to know the people we have a contact for.
pub(crate) fn group_sync_members(
    store: &Store,
    stored: &crate::groups::Group,
) -> Result<Vec<String>, StoreError> {
    let user = store
        .identity()?
        .ok_or_else(|| StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;
    let mut members = Vec::new();
    for member in stored.sync_members(&user.threema_id) {
        if store.contact(&member)?.is_some() {
            members.push(member);
        }
    }
    Ok(members)
}

/// Collect everything for the new device. `dgk`: the device group key.
pub fn collect(store: &Store, media: &MediaStore, dgk: &[u8; 32]) -> Result<JoinData, StoreError> {
    let identity = store
        .identity()?
        .ok_or_else(|| StoreError::Sqlite(rusqlite::Error::QueryReturnedNoRows))?;
    let cookie = store
        .setting(keys::DEVICE_COOKIE)?
        .and_then(|hex| data_encoding::HEXLOWER.decode(hex.as_bytes()).ok())
        .unwrap_or_default();
    let mut blobs = Blobs {
        next: 0,
        data: Vec::new(),
    };

    // Profile.
    let mut user_profile = profile_sync(store)?;
    if let Some(data) = media.read_picture(OWN_PICTURE).ok().flatten() {
        let upload = store.picture_upload().ok().flatten().map(|upload| Blob {
            id: upload.blob_id.0.to_vec(),
            // Android: CONTACT_PHOTO_NONCE (the file nonce: 1 at the end).
            nonce: crate::media::MediaPart::File.nonce().to_vec(),
            key: upload.key.to_vec(),
            uploaded_at: u64::try_from(upload.uploaded_at).unwrap_or_default(),
        });
        user_profile.profile_picture = Some(blobs.add(data, upload));
    }
    let settings = settings_sync(store)?;

    // When each conversation last changed (its newest message).
    let last_update: HashMap<String, u64> = store
        .conversations()?
        .into_iter()
        .map(|(message, _)| {
            (
                message.conversation,
                u64::try_from(message.created_at).unwrap_or_default(),
            )
        })
        .collect();

    // Contacts.
    let mut contacts = Vec::new();
    for stored in store.contacts()? {
        let id = stored.identity.as_str().to_owned();
        let mut synced = contact_sync(store, &stored)?;
        if let Some(picture) = media
            .read_picture(&contact_picture_name(&id))
            .ok()
            .flatten()
        {
            synced.contact_defined_profile_picture = Some(blobs.add(picture, None));
        }
        contacts.push(AugmentedContact {
            contact: Some(synced),
            last_update_at: last_update.get(&id).copied(),
        });
    }

    // Groups.
    let mut groups = Vec::new();
    for stored in store.groups()? {
        let mut group = group_sync(store, &stored)?;
        if let Some(data) = media.read_picture(&stored.key).ok().flatten() {
            group.profile_picture = Some(blobs.add(data, None));
        }
        groups.push(AugmentedGroup {
            group: Some(group),
            last_update_at: last_update
                .get(&stored.key)
                .copied()
                .unwrap_or_else(|| u64::try_from(stored.created_at).unwrap_or_default()),
        });
    }

    // Before the secrets are copied into the message: nothing leaves early after that.
    let csp_hashed_nonces = hashed_nonces(&identity.threema_id, store.all_nonces()?);
    let d2d_hashed_nonces = hashed_nonces(&identity.threema_id, store.all_d2d_nonces()?);
    let essential = EssentialData {
        identity_data: Some(IdentityData {
            identity: identity.threema_id.clone(),
            ck: identity.client_key.to_vec(),
            csp_device_cookie: cookie,
            csp_server_group: identity.server_group.clone(),
        }),
        work_credentials: None,
        device_group_data: Some(DeviceGroupData { dgk: dgk.to_vec() }),
        user_profile: Some(user_profile),
        settings: Some(settings),
        mdm_parameters: None,
        contacts,
        groups,
        distribution_lists: Vec::new(),
        csp_hashed_nonces,
        d2d_hashed_nonces,
    };
    let mut message = EdToNd {
        content: Some(ed_to_nd::Content::EssentialData(essential)),
    };
    // Room for the tag from the start: the sending encrypts in place, so the buffer never moves
    // and leaves no plain copy behind.
    let mut essential_data = Zeroizing::new(Vec::with_capacity(
        message.encoded_len() + ENCRYPTION_TAG_LENGTH,
    ));
    // Can't fail: a Vec always has the room.
    let _ = message.encode(&mut *essential_data);
    wipe_secrets(&mut message);
    Ok(JoinData {
        blobs: blobs.data,
        essential_data,
    })
}

/// The ID's private key and the device group key are in plain `Vec`s of the message, only until
/// it is encoded.
fn wipe_secrets(message: &mut EdToNd) {
    if let Some(ed_to_nd::Content::EssentialData(essential)) = &mut message.content {
        if let Some(identity) = &mut essential.identity_data {
            identity.ck.zeroize();
        }
        if let Some(group) = &mut essential.device_group_data {
            group.dgk.zeroize();
        }
    }
}

#[cfg(test)]
mod tests {
    use libthreema::common::{GroupIdentity, ThreemaId};

    use super::*;

    fn with_group(left: bool) -> (tempfile::TempDir, Store, crate::groups::Group) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db"), &[2; 32]).unwrap();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "TESTTEST".into(),
                client_key: Zeroizing::new([9; 32]),
                server_group: "01".into(),
            })
            .unwrap();
        let identity = GroupIdentity {
            creator_identity: ThreemaId::try_from("CREATOR1").unwrap(),
            group_id: 7,
        };
        let group = crate::groups::Group {
            key: crate::groups::key(&identity),
            identity,
            name: "Crew".into(),
            members: vec![
                "CREATOR1".into(),
                "ABCD1234".into(),
                "TESTTEST".into(),
                "UNKNOWN1".into(),
            ],
            left,
            created_at: 1,
            picture: None,
            picture_version: 0,
        };
        store.upsert_group(&group).unwrap();
        for id in ["CREATOR1", "ABCD1234"] {
            store
                .upsert_contact(&crate::contacts::test_contact(id))
                .unwrap();
        }
        (dir, store, group)
    }

    #[test]
    fn a_group_goes_out_without_the_creator_the_user_and_people_without_a_contact() {
        let (_dir, store, group) = with_group(false);
        let synced = group_sync(&store, &group).unwrap();
        assert_eq!(
            synced.member_identities.unwrap().identities,
            vec!["ABCD1234"]
        );
        assert_eq!(synced.user_state, Some(group::UserState::Member as i32));
    }

    #[test]
    fn secrets_are_wiped_and_the_buffer_has_room_for_the_tag() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db"), &[2; 32]).unwrap();
        let media = MediaStore::open(dir.path(), &[2; 32]).unwrap();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "TESTTEST".into(),
                client_key: Zeroizing::new([9; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        let data = collect(&store, &media, &[5; 32]).unwrap();
        assert!(
            data.essential_data.capacity() - data.essential_data.len() >= ENCRYPTION_TAG_LENGTH
        );

        let mut message = EdToNd::decode(data.essential_data.as_slice()).unwrap();
        wipe_secrets(&mut message);
        let Some(ed_to_nd::Content::EssentialData(essential)) = message.content else {
            panic!("not essential data");
        };
        assert!(essential.identity_data.unwrap().ck.is_empty());
        assert!(essential.device_group_data.unwrap().dgk.is_empty());
    }

    #[test]
    fn essential_data_from_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db"), &[2; 32]).unwrap();
        let media = MediaStore::open(dir.path(), &[2; 32]).unwrap();
        store
            .set_identity(&crate::store::StoredIdentity {
                threema_id: "TESTTEST".into(),
                client_key: Zeroizing::new([9; 32]),
                server_group: "a".into(),
            })
            .unwrap();
        store.set_setting(keys::NICKNAME, "Tester").unwrap();
        media.write_picture(OWN_PICTURE, b"jpeg").unwrap();
        store.add_nonces(&[&[1_u8; 24][..]]).unwrap();

        let data = collect(&store, &media, &[5; 32]).unwrap();
        assert_eq!(data.blobs.len(), 1, "the own picture");
        let EdToNd {
            content: Some(ed_to_nd::Content::EssentialData(essential)),
        } = EdToNd::decode(data.essential_data.as_slice()).unwrap()
        else {
            panic!("not essential data");
        };
        let identity = essential.identity_data.unwrap();
        assert_eq!(identity.identity, "TESTTEST");
        assert_eq!(identity.ck, vec![9; 32]);
        assert_eq!(essential.device_group_data.unwrap().dgk, vec![5; 32]);
        let profile = essential.user_profile.unwrap();
        assert_eq!(profile.nickname.as_deref(), Some("Tester"));
        let Some(delta_image::Image::Updated(image)) = profile.profile_picture.unwrap().image
        else {
            panic!("no picture");
        };
        let EdToNd {
            content: Some(ed_to_nd::Content::BlobData(blob)),
        } = EdToNd::decode(data.blobs[0].as_slice()).unwrap()
        else {
            panic!("not blob data");
        };
        assert_eq!(blob.id, image.blob.unwrap().id);
        assert_eq!(blob.data, b"jpeg");
        assert_eq!(essential.csp_hashed_nonces.len(), 1);
        assert_ne!(essential.csp_hashed_nonces[0], vec![1_u8; 24], "hashed");
        assert!(is_registered(
            &libthreema::protobuf::d2d_join::NdToEd {
                content: Some(
                    libthreema::protobuf::d2d_join::nd_to_ed::Content::Registered(
                        libthreema::protobuf::d2d_join::Registered {}
                    )
                ),
            }
            .encode_to_vec()
        ));
        // (`Begin` and `Registered` are both field 1: the same bytes, told apart by direction.)
        assert!(!is_registered(&[]));
        assert!(!is_registered(&data.blobs[0]));
    }
}
