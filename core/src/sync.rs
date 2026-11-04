// SPDX-License-Identifier: AGPL-3.0-only
//! Reflected sync envelopes (multi-device, ADR 0011): contacts, groups, the user profile and
//! settings changed on another device, applied to our store.
//!
//! Upstream: Android's `reflectedd2dsync/*`. Only fields present in the message change
//! (`optional` in md-d2d-sync.proto). Pictures come as blob mirror references
//! (`CoreEvent::SyncPicture`, fetched by `App::apply_synced_picture`).

use std::collections::{BTreeMap, HashMap};

use libthreema::{
    common::{FeatureMask, ThreemaId, keys::PublicKey},
    model::contact::Contact,
    protobuf::{
        common::{GroupIdentity as ProtoGroupIdentity, Identities, Unit},
        d2d::{
            ContactSync, GroupSync, SettingsSync, UserProfileSync, contact_sync, envelope::Content,
            group_sync, group_sync::update::MemberStateChange, settings_sync,
            transaction_scope::Scope, user_profile_sync,
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
    contacts::Lookup,
    groups::{GroupChange, MemberChange},
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

/// One line per person a group sync says came or went, as if the change had been made here.
/// Only what fits the lists (md-d2d.proto, `GroupSync.Update`): the other device may have told
/// us of a change we already have.
fn member_change_lines(
    store: &Store,
    key: &str,
    changes: &HashMap<String, i32>,
    old_members: &[String],
    new_members: &[String],
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    let mut changes: Vec<_> = changes.iter().collect();
    changes.sort();
    for (identity, change) in changes {
        let was_in = old_members.contains(identity);
        let is_in = new_members.contains(identity);
        let code = match MemberStateChange::try_from(*change) {
            Ok(MemberStateChange::Added) if !was_in && is_in => "groupMemberAdded",
            Ok(MemberStateChange::Kicked) if was_in && !is_in => "groupMemberRemoved",
            Ok(MemberStateChange::Left) if was_in && !is_in => "groupMemberLeft",
            _ => continue,
        };
        let name = store.contact(identity)?.map_or_else(
            || identity.clone(),
            |contact| crate::contacts::display_name(&contact),
        );
        status_line(store, key, &format!("{code}|{name}"), events)?;
    }
    Ok(())
}

fn status_line(
    store: &Store,
    key: &str,
    code: &str,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    if let Some(message) = store.insert_status(key, code)? {
        events.push(CoreEvent::MessageAdded {
            message,
            send_delivery_receipt: false,
        });
    }
    Ok(())
}

/// How a group sync comes in: `Create` makes the group if need be, `Update` only changes one we
/// have (only some fields, which wouldn't make a group) and may name who came and went.
enum GroupAction<'a> {
    Create,
    Update(&'a HashMap<String, i32>),
}

fn apply_group(
    store: &Store,
    user: &str,
    mut synced: d2d_sync::Group,
    action: &GroupAction<'_>,
    events: &mut Vec<CoreEvent>,
) -> Result<bool, StoreError> {
    let Some((key, identity)) = group_key(synced.group_identity.as_ref()) else {
        return Ok(false);
    };
    let creator = identity.creator_identity.to_string();
    let existing = store.group(&key)?;
    if existing.is_none() && matches!(action, GroupAction::Update(_)) {
        tracing::info!("group sync update for an unknown group, ignored");
        return Ok(false);
    }
    let is_new = existing.is_none();
    let was_left = existing.as_ref().is_some_and(|group| group.left);
    let mut stored = existing.unwrap_or(crate::groups::Group {
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
    let old_members = stored.members.clone();
    let renamed = synced
        .name
        .as_ref()
        .is_some_and(|name| *name != stored.name);
    if let Some(name) = synced.name {
        stored.name = name;
    }
    let members_given = synced.member_identities.is_some();
    if let Some(members) = synced.member_identities {
        stored.members = crate::groups::normalized_members(members.identities, &creator, user);
    }
    let user_state = synced
        .user_state
        .and_then(|s| group::UserState::try_from(s).ok());
    if let Some(state) = user_state {
        stored.left = state != group::UserState::Member;
    }
    store.upsert_group(&stored)?;
    match user_state {
        Some(group::UserState::Kicked) => store.set_group_kicked(&key, true)?,
        Some(group::UserState::Left) => store.set_group_kicked(&key, false)?,
        _ => {}
    }
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
    // The status lines a change made here would have added: reflected group control adds none.
    let user_is_creator = creator == user;
    if is_new {
        if !stored.left {
            let code = if user_is_creator {
                "groupCreated"
            } else {
                "groupJoined"
            };
            status_line(store, &key, code, events)?;
        }
        return Ok(true);
    }
    match user_state {
        Some(group::UserState::Member) if was_left => {
            status_line(store, &key, "groupJoined", events)?;
        }
        Some(group::UserState::Kicked) if !was_left => {
            status_line(store, &key, "groupRemoved", events)?;
        }
        Some(group::UserState::Left) if !was_left => {
            let code = if user_is_creator {
                "groupDisbanded"
            } else {
                "groupLeft"
            };
            status_line(store, &key, code, events)?;
        }
        _ => {}
    }
    if renamed {
        status_line(
            store,
            &key,
            &format!("groupRenamed|{}", stored.name),
            events,
        )?;
    }
    if let GroupAction::Update(changes) = action
        && members_given
        && !was_left
    {
        member_change_lines(store, &key, changes, &old_members, &stored.members, events)?;
    }
    Ok(true)
}

/// The group sync for a change a group message made while this device leads: a new group whole,
/// else only what changed (csp.struct.yml, the _MD_ steps of the group control messages).
fn received_change_sync(
    store: &Store,
    group: &crate::groups::Group,
    change: &GroupChange,
) -> Result<Content, StoreError> {
    let action = if change.created {
        let mut synced = crate::join::group_sync(store, group)?;
        // Not here yet; a picture comes once it has been downloaded.
        synced.profile_picture = None;
        group_sync::Action::Create(group_sync::Create {
            group: Some(synced),
        })
    } else {
        let mut synced = group_only(group);
        if change.name {
            synced.name = Some(group.name.clone());
        }
        let mut member_state_changes = HashMap::new();
        if change.members {
            let identities = crate::join::group_sync_members(store, group)?;
            for (member, state) in &change.member_changes {
                // Someone who has no contact yet comes later (`members_looked_up`).
                if *state != MemberChange::Added || identities.contains(member) {
                    member_state_changes.insert(
                        member.clone(),
                        match state {
                            MemberChange::Added => MemberStateChange::Added,
                            MemberChange::Kicked => MemberStateChange::Kicked,
                            MemberChange::Left => MemberStateChange::Left,
                        } as i32,
                    );
                }
            }
            synced.member_identities = Some(Identities { identities });
        }
        if change.user_state {
            synced.user_state = Some(if !group.left {
                group::UserState::Member
            } else if store.group_kicked(&group.key)? {
                group::UserState::Kicked
            } else {
                group::UserState::Left
            } as i32);
        }
        if change.picture_removed {
            synced.profile_picture = Some(DeltaImage {
                image: Some(delta_image::Image::Removed(Unit {})),
            });
        }
        group_sync::Action::Update(group_sync::Update {
            group: Some(synced),
            member_state_changes,
        })
    };
    Ok(Content::GroupSync(GroupSync {
        action: Some(action),
    }))
}

/// A group sync that only names the group: what changed goes on top.
pub(crate) fn group_only(group: &crate::groups::Group) -> d2d_sync::Group {
    d2d_sync::Group {
        group_identity: Some(ProtoGroupIdentity {
            group_id: group.identity.group_id,
            creator_identity: group.identity.creator_identity.as_str().to_owned(),
        }),
        ..Default::default()
    }
}

/// Tell the other devices of a change a group message made, in the caller's store step. Nothing
/// without a device group (`false`). The other devices only get to see people they have a contact
/// for, so the members still `unknown` here aren't in this entry: they are noted, and the lookup
/// sends their contact and then the list with them in it (`members_looked_up`). So however long
/// the lookup takes, no group sync names anyone before their contact has gone out.
pub(crate) fn queue_received_group_change(
    store: &Store,
    key: &str,
    change: &GroupChange,
    unknown: &[String],
) -> Result<bool, StoreError> {
    if store.multi_device()?.is_none() {
        return Ok(false);
    }
    let Some(group) = store.group(key)? else {
        return Ok(false);
    };
    let content = received_change_sync(store, &group, change)?;
    store.queue_sync_content(Scope::GroupSync, content)?;
    for identity in unknown {
        let added = change.member_changes.get(identity) == Some(&MemberChange::Added);
        store.add_member_lookup(key, identity, added)?;
    }
    Ok(true)
}

/// What the directory said about group members, as one store step: their contacts, and for the
/// lookups the other devices are owed, each contact as a `ContactSync.Create` and then, for each
/// group waiting for it, the list with them in it. Someone the directory doesn't know stops being
/// waited for. Returns whether something was queued. A failed lookup never gets here: what it owed
/// stays noted and is tried again.
pub(crate) fn members_looked_up(
    store: &Store,
    results: Vec<(String, Lookup)>,
) -> Result<bool, StoreError> {
    use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;

    let step = store.conn.unchecked_transaction()?;
    let device_group = store.multi_device()?.is_some();
    let mut queued = false;
    let mut groups: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for (identity, result) in results {
        let waiting = store.take_member_lookups(&identity)?;
        if let Lookup::Found(contact) = result
            && store.contact(&identity)?.is_none()
        {
            let mut contact = *contact;
            contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
            store.upsert_contact(&contact)?;
            if device_group && !waiting.is_empty() {
                store.queue_sync_content(
                    Scope::ContactSync,
                    Content::ContactSync(ContactSync {
                        action: Some(contact_sync::Action::Create(contact_sync::Create {
                            contact: Some(crate::join::contact_sync(store, &contact)?),
                        })),
                    }),
                )?;
                queued = true;
            }
        }
        for (key, added) in waiting {
            groups
                .entry(key)
                .or_default()
                .push((identity.clone(), added));
        }
    }
    if device_group {
        for (key, members) in groups {
            let Some(group) = store.group(&key)? else {
                continue;
            };
            let identities = crate::join::group_sync_members(store, &group)?;
            // Nobody new in the list (the directory didn't know them): nothing to say.
            if !members
                .iter()
                .any(|(member, _)| identities.contains(member))
            {
                continue;
            }
            let member_state_changes = members
                .into_iter()
                .filter(|(member, added)| *added && identities.contains(member))
                .map(|(member, _)| (member, MemberStateChange::Added as i32))
                .collect();
            let mut synced = group_only(&group);
            synced.member_identities = Some(Identities { identities });
            store.queue_sync_content(
                Scope::GroupSync,
                Content::GroupSync(GroupSync {
                    action: Some(group_sync::Action::Update(group_sync::Update {
                        group: Some(synced),
                        member_state_changes,
                    })),
                }),
            )?;
            queued = true;
        }
    }
    step.commit()?;
    Ok(queued)
}

/// Once, after the update that brought this: contacts looked up for a group used to stay on this
/// device, so a linked Desktop doesn't have them, and throws away every member list naming one.
/// All group-level contacts go out; it discards those it has. One store step, and only for a
/// device group: one linked later gets all contacts with the join anyway.
pub(crate) fn reflect_group_contacts_once(store: &Store) -> Result<(), StoreError> {
    use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;

    if store.setting(keys::GROUP_CONTACTS_TO_REFLECT)?.is_none() {
        return Ok(());
    }
    let step = store.conn.unchecked_transaction()?;
    if store.multi_device()?.is_some() {
        for contact in store.contacts()? {
            if contact.acquaintance_level == AcquaintanceLevel::GroupOrDeleted {
                store.queue_sync_content(
                    Scope::ContactSync,
                    Content::ContactSync(ContactSync {
                        action: Some(contact_sync::Action::Create(contact_sync::Create {
                            contact: Some(crate::join::contact_sync(store, &contact)?),
                        })),
                    }),
                )?;
            }
        }
    }
    store.delete_setting(keys::GROUP_CONTACTS_TO_REFLECT)?;
    step.commit()?;
    Ok(())
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
            Some(group) => apply_group(store, user, group, &GroupAction::Create, events)?,
            None => false,
        },
        Some(group_sync::Action::Update(update)) => match update.group {
            Some(group) => {
                let changes = GroupAction::Update(&update.member_state_changes);
                apply_group(store, user, group, &changes, events)?
            }
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

    fn update(store: &Store, group: d2d_sync::Group, changes: &[(&str, MemberStateChange)]) {
        group_sync(
            store,
            USER,
            GroupSync {
                action: Some(group_sync::Action::Update(group_sync::Update {
                    group: Some(group),
                    member_state_changes: changes
                        .iter()
                        .map(|(id, change)| ((*id).to_owned(), *change as i32))
                        .collect(),
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

    fn lines(store: &Store, creator: &str) -> Vec<String> {
        store
            .conversation_messages(&key(creator), 20)
            .unwrap()
            .into_iter()
            .map(|message| message.body)
            .collect()
    }

    #[test]
    fn group_contacts_go_out_once_after_the_update_and_only_to_a_device_group() {
        use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;
        let (_dir, store) = store();
        let mut hidden = crate::contacts::test_contact(BOB);
        hidden.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
        store.upsert_contact(&hidden).unwrap();
        let mut direct = crate::contacts::test_contact(EVE);
        direct.acquaintance_level = AcquaintanceLevel::Direct;
        store.upsert_contact(&direct).unwrap();

        // A new store has the mark from the migration, and no device group: it just goes.
        reflect_group_contacts_once(&store).unwrap();
        assert!(store.next_sync().unwrap().is_none());
        assert!(
            store
                .setting(keys::GROUP_CONTACTS_TO_REFLECT)
                .unwrap()
                .is_none()
        );

        // As after updating with Desktop linked.
        store
            .set_setting(keys::GROUP_CONTACTS_TO_REFLECT, "1")
            .unwrap();
        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        reflect_group_contacts_once(&store).unwrap();
        let (id, scope, _) = store.next_sync().unwrap().unwrap();
        assert_eq!(scope, Scope::ContactSync as i32);
        store.sync_done(id).unwrap();
        assert!(store.next_sync().unwrap().is_none());
        reflect_group_contacts_once(&store).unwrap();
        assert!(store.next_sync().unwrap().is_none());
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
        update(&store, listing(CREATOR, &[BOB, EVE]), &[]);
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, BOB, EVE]
        );
        // Whatever a device sends: no doubles, not us.
        update(&store, listing(CREATOR, &[EVE, USER, CREATOR, EVE]), &[]);
        assert_eq!(
            store.group(&key(CREATOR)).unwrap().unwrap().members,
            vec![CREATOR, EVE]
        );
        // Nothing said about members: nothing changes.
        update(&store, synced(CREATOR), &[]);
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
    fn an_update_makes_no_group() {
        let (_dir, store) = store();
        let mut events = Vec::new();
        let pinned = d2d_sync::Group {
            conversation_visibility: Some(ConversationVisibility::Pinned as i32),
            ..synced(CREATOR)
        };
        group_sync(
            &store,
            USER,
            GroupSync {
                action: Some(group_sync::Action::Update(group_sync::Update {
                    group: Some(pinned),
                    member_state_changes: Default::default(),
                })),
            },
            &mut events,
        )
        .unwrap();
        assert!(store.group(&key(CREATOR)).unwrap().is_none());
        assert!(events.is_empty());
        assert!(store.pinned_chats().unwrap().is_empty());
    }

    fn state(state: group::UserState) -> d2d_sync::Group {
        d2d_sync::Group {
            user_state: Some(state as i32),
            ..synced(CREATOR)
        }
    }

    #[test]
    fn how_we_went_is_kept_and_cleared_on_coming_back() {
        let (_dir, store) = store();
        let group_key = key(CREATOR);
        create(&store, listing(CREATOR, &[BOB]));
        assert!(!store.group_kicked(&group_key).unwrap());

        update(&store, state(group::UserState::Kicked), &[]);
        assert!(store.group(&group_key).unwrap().unwrap().left);
        assert!(store.group_kicked(&group_key).unwrap());

        update(&store, state(group::UserState::Member), &[]);
        assert!(!store.group(&group_key).unwrap().unwrap().left);
        assert!(!store.group_kicked(&group_key).unwrap());

        update(&store, state(group::UserState::Left), &[]);
        assert!(store.group(&group_key).unwrap().unwrap().left);
        assert!(!store.group_kicked(&group_key).unwrap());
    }

    #[test]
    fn changes_from_the_other_device_show_as_lines() {
        let (_dir, store) = store();
        let events = create(&store, listing(CREATOR, &[BOB]));
        assert!(events.iter().any(|event| matches!(
            event,
            CoreEvent::MessageAdded { message, send_delivery_receipt: false }
                if message.body == "groupJoined"
        )));
        update(
            &store,
            listing(CREATOR, &[EVE]),
            &[
                (EVE, MemberStateChange::Added),
                (BOB, MemberStateChange::Kicked),
            ],
        );
        update(
            &store,
            listing(CREATOR, &[]),
            &[(EVE, MemberStateChange::Left)],
        );
        update(
            &store,
            d2d_sync::Group {
                name: Some("Crew".into()),
                ..synced(CREATOR)
            },
            &[],
        );
        // The same name, and changes that don't fit the lists: no lines.
        update(
            &store,
            d2d_sync::Group {
                name: Some("Crew".into()),
                ..listing(CREATOR, &[])
            },
            &[
                (EVE, MemberStateChange::Kicked),
                (BOB, MemberStateChange::Added),
                (CREATOR, MemberStateChange::Left),
            ],
        );
        update(&store, state(group::UserState::Kicked), &[]);
        update(&store, state(group::UserState::Kicked), &[]);
        assert_eq!(
            lines(&store, CREATOR),
            vec![
                "groupJoined",
                format!("groupMemberRemoved|{BOB}").as_str(),
                format!("groupMemberAdded|{EVE}").as_str(),
                format!("groupMemberLeft|{EVE}").as_str(),
                "groupRenamed|Crew",
                "groupRemoved",
            ]
        );
    }

    #[test]
    fn our_own_groups_lines_say_what_we_did() {
        let (_dir, store) = store();
        create(&store, listing(USER, &[BOB]));
        update(
            &store,
            d2d_sync::Group {
                user_state: Some(group::UserState::Left as i32),
                ..synced(USER)
            },
            &[],
        );
        assert_eq!(lines(&store, USER), vec!["groupCreated", "groupDisbanded"]);
    }

    #[test]
    fn members_we_have_no_contact_for_are_looked_up_without_owing_anyone() {
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
        // Another device named them: it tells the others itself, nothing is owed from here.
        assert!(store.outstanding_member_lookups().unwrap().is_empty());
    }
}
