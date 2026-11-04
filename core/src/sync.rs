// SPDX-License-Identifier: AGPL-3.0-only
//! Reflected sync envelopes (multi-device, ADR 0011): contacts, groups, the user profile and
//! settings changed on another device, applied to our store.
//!
//! Upstream: Android's `reflectedd2dsync/*`. Only fields present in the message change
//! (`optional` in md-d2d-sync.proto). Pictures come as blob mirror references
//! (`CoreEvent::SyncPicture`, fetched by `App::apply_synced_picture`).

use libthreema::{
    common::{FeatureMask, ThreemaId, keys::PublicKey},
    model::contact::Contact,
    protobuf::{
        common::GroupIdentity as ProtoGroupIdentity,
        d2d::{
            ContactSync, GroupSync, SettingsSync, UserProfileSync, contact_sync, group_sync,
            settings_sync, user_profile_sync,
        },
        d2d_sync::{
            self, ConversationCategory, ConversationVisibility, ReadReceiptPolicy,
            TypingIndicatorPolicy, contact, group, settings,
        },
    },
};

use libthreema::protobuf::common::{DeltaImage, delta_image};

use crate::{
    chat_notifications::Policy,
    providers::CoreEvent,
    store::{Store, StoreError, keys},
};

/// A picture on the blob mirror, as a sync message describes it.
#[derive(Debug, Clone)]
pub struct SyncedBlob {
    pub id: libthreema::common::BlobId,
    pub key: [u8; 32],
    pub nonce: [u8; 24],
}

/// What a sync message says about a picture: `None` unchanged, `Some(None)` removed,
/// `Some(Some(blob))` a new one.
fn picture(delta: Option<DeltaImage>) -> Option<Option<SyncedBlob>> {
    match delta?.image? {
        delta_image::Image::Removed(_) => Some(None),
        delta_image::Image::Updated(image) => {
            let blob = image.blob?;
            Some(Some(SyncedBlob {
                id: libthreema::common::BlobId(blob.id.as_slice().try_into().ok()?),
                key: blob.key.as_slice().try_into().ok()?,
                // Android's default when absent: the file nonce (…01).
                nonce: blob
                    .nonce
                    .as_slice()
                    .try_into()
                    .unwrap_or(*crate::media::MediaPart::File.nonce()),
            }))
        }
    }
}

/// Visibility and category of a conversation (pinned/archived, private).
fn apply_conversation(
    store: &Store,
    conversation: &str,
    visibility: Option<i32>,
    category: Option<i32>,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    if let Some(visibility) = visibility.and_then(|v| ConversationVisibility::try_from(v).ok()) {
        store.set_pinned(conversation, visibility == ConversationVisibility::Pinned)?;
        store.set_archived(conversation, visibility == ConversationVisibility::Archived)?;
    }
    if let Some(category) = category.and_then(|c| ConversationCategory::try_from(c).ok()) {
        let private = category == ConversationCategory::Protected;
        if store.is_private(conversation)? != private {
            store.set_private(conversation, private)?;
            events.push(CoreEvent::PrivateChanged {
                conversation: conversation.to_owned(),
                private,
            });
        }
    }
    Ok(())
}

fn non_empty(text: Option<String>) -> Option<Option<String>> {
    text.map(|text| Some(text).filter(|t| !t.is_empty()))
}

fn apply_contact(
    store: &Store,
    mut synced: d2d_sync::Contact,
    events: &mut Vec<CoreEvent>,
) -> Result<bool, StoreError> {
    let Ok(identity) = synced.identity.parse::<ThreemaId>() else {
        return Ok(false);
    };
    let existing = store.contact(identity.as_str())?;
    let mut contact = match existing {
        Some(contact) => contact,
        None => {
            // A new contact needs its key.
            let Some(key) = synced
                .public_key
                .as_deref()
                .and_then(|k| <[u8; 32]>::try_from(k).ok())
            else {
                return Ok(false);
            };
            Contact {
                identity,
                public_key: PublicKey::from(key),
                created_at: synced.created_at.unwrap_or_else(|| {
                    u64::try_from(crate::messages::now_ms()).unwrap_or_default()
                }),
                first_name: None,
                last_name: None,
                nickname: None,
                verification_level: contact::VerificationLevel::Unverified,
                work_verification_level: contact::WorkVerificationLevel::None,
                identity_type: contact::IdentityType::Regular,
                acquaintance_level: contact::AcquaintanceLevel::Direct,
                activity_state: contact::ActivityState::Active,
                feature_mask: FeatureMask(0),
                sync_state: contact::SyncState::Initial,
                work_last_full_sync_at: None,
                work_availability_status: None,
                read_receipt_policy_override: None,
                typing_indicator_policy_override: None,
                notification_trigger_policy_override: None,
                conversation_category: ConversationCategory::Default,
                conversation_visibility: ConversationVisibility::Normal,
            }
        }
    };
    if let Some(name) = non_empty(synced.first_name) {
        contact.first_name = name;
    }
    if let Some(name) = non_empty(synced.last_name) {
        contact.last_name = name;
    }
    if let Some(name) = non_empty(synced.nickname) {
        contact.nickname = name;
    }
    if let Some(level) = synced
        .verification_level
        .and_then(|v| contact::VerificationLevel::try_from(v).ok())
    {
        contact.verification_level = level;
    }
    if let Some(level) = synced
        .acquaintance_level
        .and_then(|v| contact::AcquaintanceLevel::try_from(v).ok())
    {
        contact.acquaintance_level = level;
    }
    if let Some(state) = synced
        .activity_state
        .and_then(|v| contact::ActivityState::try_from(v).ok())
    {
        contact.activity_state = state;
    }
    if let Some(kind) = synced
        .identity_type
        .and_then(|v| contact::IdentityType::try_from(v).ok())
    {
        contact.identity_type = kind;
    }
    if let Some(state) = synced
        .sync_state
        .and_then(|v| contact::SyncState::try_from(v).ok())
    {
        contact.sync_state = state;
    }
    if let Some(mask) = synced.feature_mask {
        contact.feature_mask = FeatureMask(mask);
    }
    if let Some(policy) = synced.read_receipt_policy_override {
        contact.read_receipt_policy_override = match policy.r#override {
            Some(contact::read_receipt_policy_override::Override::Policy(p)) => {
                ReadReceiptPolicy::try_from(p).ok()
            }
            _ => None,
        };
    }
    if let Some(policy) = synced.typing_indicator_policy_override {
        contact.typing_indicator_policy_override = match policy.r#override {
            Some(contact::typing_indicator_policy_override::Override::Policy(p)) => {
                TypingIndicatorPolicy::try_from(p).ok()
            }
            _ => None,
        };
    }
    let id = identity.as_str().to_owned();
    if let Some(blob) = picture(synced.contact_defined_profile_picture.take()) {
        events.push(CoreEvent::SyncPicture {
            target: crate::profile_pictures::contact_picture_name(&id),
            blob,
        });
    }
    if let Some(policy) = synced.notification_trigger_policy_override {
        store.set_chat_notifications(
            &id,
            match policy.r#override {
                Some(contact::notification_trigger_policy_override::Override::Policy(p)) => Some((
                    Policy::Never,
                    p.expires_at.and_then(|at| i64::try_from(at).ok()),
                )),
                _ => None,
            },
        )?;
    }
    if let Some(category) = synced
        .conversation_category
        .and_then(|c| ConversationCategory::try_from(c).ok())
    {
        contact.conversation_category = category;
    }
    if let Some(visibility) = synced
        .conversation_visibility
        .and_then(|v| ConversationVisibility::try_from(v).ok())
    {
        contact.conversation_visibility = visibility;
    }
    store.upsert_contact(&contact)?;
    apply_conversation(
        store,
        &id,
        synced.conversation_visibility,
        synced.conversation_category,
        events,
    )?;
    Ok(true)
}

fn group_key(
    identity: Option<&ProtoGroupIdentity>,
) -> Option<(String, libthreema::common::GroupIdentity)> {
    let identity = identity?;
    let group = libthreema::common::GroupIdentity {
        group_id: identity.group_id,
        creator_identity: identity.creator_identity.parse().ok()?,
    };
    Some((crate::groups::key(&group), group))
}

fn apply_group(
    store: &Store,
    user: &str,
    mut synced: d2d_sync::Group,
    events: &mut Vec<CoreEvent>,
) -> Result<bool, StoreError> {
    let Some((key, identity)) = group_key(synced.group_identity.as_ref()) else {
        return Ok(false);
    };
    let creator = identity.creator_identity.to_string();
    let mut stored = store.group(&key)?.unwrap_or(crate::groups::Group {
        key: key.clone(),
        identity,
        name: String::new(),
        members: crate::groups::normalized_members([], &creator, user),
        left: false,
        created_at: synced
            .created_at
            .and_then(|at| i64::try_from(at).ok())
            .unwrap_or_else(crate::messages::now_ms),
        picture: None,
        picture_version: 0,
    });
    if let Some(name) = synced.name {
        stored.name = name;
    }
    if let Some(members) = synced.member_identities {
        stored.members = crate::groups::normalized_members(members.identities, &creator, user);
    }
    if let Some(state) = synced
        .user_state
        .and_then(|s| group::UserState::try_from(s).ok())
    {
        stored.left = state != group::UserState::Member;
    }
    store.upsert_group(&stored)?;
    // Their contacts come first by the protocol; if one doesn't, messages to them would go nowhere.
    let unknown: Vec<String> = stored
        .members
        .iter()
        .filter(|member| !stored.left && store.contact(member).ok().flatten().is_none())
        .cloned()
        .collect();
    if !unknown.is_empty() {
        events.push(CoreEvent::UnknownMembers(unknown));
    }
    if let Some(blob) = picture(synced.profile_picture.take()) {
        events.push(CoreEvent::SyncPicture {
            target: key.clone(),
            blob,
        });
    }
    if let Some(policy) = synced.notification_trigger_policy_override {
        store.set_chat_notifications(
            &key,
            match policy.r#override {
                Some(group::notification_trigger_policy_override::Override::Policy(p)) => {
                    let policy = match group::notification_trigger_policy_override::policy::NotificationTriggerPolicy::try_from(p.policy) {
                        Ok(group::notification_trigger_policy_override::policy::NotificationTriggerPolicy::Mentioned) => Policy::Mentioned,
                        _ => Policy::Never,
                    };
                    Some((policy, p.expires_at.and_then(|at| i64::try_from(at).ok())))
                }
                _ => None,
            },
        )?;
    }
    apply_conversation(
        store,
        &key,
        synced.conversation_visibility,
        synced.conversation_category,
        events,
    )?;
    Ok(true)
}

pub fn contact_sync(
    store: &Store,
    sync: ContactSync,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    let contact = match sync.action {
        Some(contact_sync::Action::Create(create)) => create.contact,
        Some(contact_sync::Action::Update(update)) => update.contact,
        None => None,
    };
    if let Some(contact) = contact
        && apply_contact(store, contact, events)?
    {
        events.push(CoreEvent::ContactsChanged);
    }
    Ok(())
}

pub fn group_sync(
    store: &Store,
    user: &str,
    sync: GroupSync,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    let changed = match sync.action {
        Some(group_sync::Action::Create(create)) => match create.group {
            Some(group) => apply_group(store, user, group, events)?,
            None => false,
        },
        Some(group_sync::Action::Update(update)) => match update.group {
            Some(group) => apply_group(store, user, group, events)?,
            None => false,
        },
        Some(group_sync::Action::Delete(delete)) => match group_key(delete.group_identity.as_ref())
        {
            Some((key, _)) => {
                store.delete_group(&key)?;
                true
            }
            None => false,
        },
        None => false,
    };
    if changed {
        events.push(CoreEvent::GroupsChanged);
    }
    Ok(())
}

pub fn settings_sync(store: &Store, sync: SettingsSync) -> Result<(), StoreError> {
    let Some(settings_sync::Action::Update(update)) = sync.action else {
        return Ok(());
    };
    let Some(synced) = update.settings else {
        return Ok(());
    };
    let mut app: crate::safe::backup::Settings = store
        .setting(keys::APP_SETTINGS)?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    if let Some(policy) = synced.contact_sync_policy {
        app.sync_contacts = policy == settings::ContactSyncPolicy::Sync as i32;
    }
    if let Some(policy) = synced.unknown_contact_policy {
        app.block_unknown = policy == settings::UnknownContactPolicy::BlockUnknown as i32;
    }
    if let Some(policy) = synced.read_receipt_policy {
        app.read_receipts = policy == ReadReceiptPolicy::SendReadReceipt as i32;
    }
    if let Some(policy) = synced.typing_indicator_policy {
        app.send_typing = policy == TypingIndicatorPolicy::SendTypingIndicator as i32;
    }
    if let Some(policy) = synced.o2o_call_policy {
        app.threema_calls = policy == settings::O2oCallPolicy::AllowO2oCall as i32;
    }
    if let Some(policy) = synced.o2o_call_connection_policy {
        app.relay_threema_calls =
            policy == settings::O2oCallConnectionPolicy::RequireRelayedConnection as i32;
    }
    if let Some(policy) = synced.o2o_call_video_policy {
        let allow = policy == settings::O2oCallVideoPolicy::AllowVideo as i32;
        store.set_setting(keys::VIDEO_CALLS, if allow { "1" } else { "0" })?;
    }
    if let Some(policy) = synced.group_call_policy {
        let allow = policy == settings::GroupCallPolicy::AllowGroupCall as i32;
        store.set_setting(keys::GROUP_CALLS, if allow { "1" } else { "0" })?;
    }
    if let Some(policy) = synced.screenshot_policy {
        app.disable_screenshots = policy == settings::ScreenshotPolicy::DenyScreenshot as i32;
    }
    if let Some(policy) = synced.keyboard_data_collection_policy {
        app.incognito_keyboard =
            policy == settings::KeyboardDataCollectionPolicy::DenyDataCollection as i32;
    }
    if let Some(blocked) = synced.blocked_identities {
        app.blocked_contacts = blocked.identities;
    }
    if let Some(excluded) = synced.exclude_from_sync_identities {
        app.sync_excluded_ids = excluded.identities;
    }
    let json = serde_json::to_string(&app).unwrap_or_default();
    store.set_setting(keys::APP_SETTINGS, &json)
}

pub fn user_profile_sync(
    store: &Store,
    sync: UserProfileSync,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    let Some(user_profile_sync::Action::Update(update)) = sync.action else {
        return Ok(());
    };
    let Some(mut profile) = update.user_profile else {
        return Ok(());
    };
    if let Some(blob) = picture(profile.profile_picture.take()) {
        events.push(CoreEvent::SyncPicture {
            target: crate::profile_pictures::OWN_PICTURE.to_owned(),
            blob,
        });
    }
    if let Some(nickname) = profile.nickname {
        if nickname.is_empty() {
            store.delete_setting(keys::NICKNAME)?;
        } else {
            store.set_setting(keys::NICKNAME, &nickname)?;
        }
    }
    if let Some(share) = profile.profile_picture_share_with.and_then(|s| s.policy) {
        use crate::profile_pictures::Share;
        use d2d_sync::user_profile::profile_picture_share_with::Policy as SharePolicy;
        match share {
            SharePolicy::Nobody(_) => store.set_picture_share(Share::Nobody)?,
            SharePolicy::Everyone(_) => store.set_picture_share(Share::Everyone)?,
            SharePolicy::AllowList(list) => {
                store.set_picture_share(Share::Selected)?;
                store.set_picture_recipients(&list.identities)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use libthreema::protobuf::common::Identities;

    use super::*;

    const USER: &str = "USER0001";
    const CREATOR: &str = "CREATOR1";
    const BOB: &str = "00000BOB";
    const EVE: &str = "00000EVE";

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        (dir, store)
    }

    fn synced(creator: &str) -> d2d_sync::Group {
        d2d_sync::Group {
            group_identity: Some(ProtoGroupIdentity {
                group_id: 7,
                creator_identity: creator.to_owned(),
            }),
            ..Default::default()
        }
    }

    fn listing(creator: &str, members: &[&str]) -> d2d_sync::Group {
        d2d_sync::Group {
            member_identities: Some(Identities {
                identities: members.iter().map(ToString::to_string).collect(),
            }),
            ..synced(creator)
        }
    }

    fn create(store: &Store, group: d2d_sync::Group) -> Vec<CoreEvent> {
        let mut events = Vec::new();
        group_sync(
            store,
            USER,
            GroupSync {
                action: Some(group_sync::Action::Create(group_sync::Create {
                    group: Some(group),
                })),
            },
            &mut events,
        )
        .unwrap();
        events
    }

    fn update(store: &Store, group: d2d_sync::Group) {
        group_sync(
            store,
            USER,
            GroupSync {
                action: Some(group_sync::Action::Update(group_sync::Update {
                    group: Some(group),
                    member_state_changes: Default::default(),
                })),
            },
            &mut Vec::new(),
        )
        .unwrap();
    }

    fn key(creator: &str) -> String {
        let (key, _) = group_key(synced(creator).group_identity.as_ref()).unwrap();
        key
    }

    #[test]
    fn the_creator_stays_a_member_of_someone_elses_group() {
        let (_dir, store) = store();
        // The protocol's list has neither the creator nor us; what we keep has the creator.
        create(&store, listing(CREATOR, &[BOB]));
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, BOB]
        );
        update(&store, listing(CREATOR, &[BOB, EVE]));
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, BOB, EVE]
        );
        // Whatever a device sends: no doubles, not us.
        update(&store, listing(CREATOR, &[EVE, USER, CREATOR, EVE]));
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, EVE]
        );
        // Nothing said about members: nothing changes.
        update(&store, synced(CREATOR));
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, EVE]
        );
    }

    #[test]
    fn our_own_group_keeps_just_the_others() {
        let (_dir, store) = store();
        create(&store, listing(USER, &[BOB]));
        assert_eq!(store.group(&key(USER)).unwrap().unwrap().members, vec![BOB]);
    }

    #[test]
    fn a_new_group_without_a_list_still_has_its_creator() {
        let (_dir, store) = store();
        create(&store, synced(CREATOR));
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR]
        );
    }

    #[test]
    fn members_we_have_no_contact_for_are_looked_up() {
        let (_dir, store) = store();
        store
            .upsert_contact(&crate::contacts::test_contact(BOB))
            .unwrap();
        let events = create(&store, listing(CREATOR, &[BOB, EVE]));
        let asked = events.iter().find_map(|event| match event {
            CoreEvent::UnknownMembers(identities) => Some(identities.clone()),
            _ => None,
        });
        assert_eq!(asked, Some(vec![CREATOR.to_owned(), EVE.to_owned()]));
    }
}
