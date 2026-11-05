// SPDX-License-Identifier: AGPL-3.0-only
//! Reflected sync envelopes (multi-device, ADR 0011): contacts, groups, the user profile and
//! settings changed on another device, applied to our store.
//!
//! Upstream: Android's `reflectedd2dsync/*`. Only fields present in the message change
//! (`optional` in md-d2d-sync.proto). Pictures come as blob mirror references
//! (`synced_pictures`, then `CoreEvent::SyncPicture`, fetched by `App::apply_synced_picture`).

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
/// Kept with the sync that says it, which comes only once: fetched and stored from there on, and
/// again at a later try (`App::apply_synced_picture`).
fn note_picture(
    store: &Store,
    target: String,
    blob: Option<&SyncedBlob>,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
    store.set_synced_picture(&target, blob)?;
    events.push(CoreEvent::SyncPicture { target });
    Ok(())
}

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
    let name_before = (contact.first_name.clone(), contact.last_name.clone());
    if let Some(name) = non_empty(synced.first_name) {
        contact.first_name = name;
    }
    if let Some(name) = non_empty(synced.last_name) {
        contact.last_name = name;
    }
    let name_changed = name_before != (contact.first_name.clone(), contact.last_name.clone());
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
    } else if name_changed {
        // Android reflects a rename as the name alone. The user wrote it, on the other device: the
        // address book's next match must not write over it.
        contact.sync_state = contact::SyncState::Custom;
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
        // The synced state is the newest; a download still under way is obsolete.
        store.cancel_picture_download(&id)?;
        let target = crate::profile_pictures::contact_picture_name(&id);
        note_picture(store, target, blob.as_ref(), events)?;
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
        events.push(CoreEvent::MessageAdded { message });
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
        note_picture(store, key.clone(), blob.as_ref(), events)?;
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
            status_line(store, &key, "groupNoLongerMember", events)?;
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
    if !crate::join::group_syncable(store, &group)? {
        return Ok(false);
    }
    let content = received_change_sync(store, &group, change)?;
    store.queue_sync_content(Scope::GroupSync, content)?;
    for identity in unknown {
        let added = change.member_changes.get(identity) == Some(&MemberChange::Added);
        store.add_member_lookup(key, identity, added)?;
    }
    Ok(true)
}

/// What [`members_looked_up`] did.
#[derive(Debug, Default)]
pub(crate) struct MembersLookedUp {
    /// Something was queued for the other devices.
    pub queued: bool,
    /// "Added" lines of those the directory knows, for the UI.
    pub lines: Vec<crate::messages::Message>,
}

/// What the directory said about group members, as one store step: their contacts, in a device
/// group each as a `ContactSync.Create`, and for the lookups the other devices are owed, for each
/// group waiting for it, the list with them in it. So is an "added" line for those who came with a
/// change. Someone the directory doesn't know stops being
/// waited for. Returns whether something was queued. A failed lookup never gets here: what it owed
/// stays noted and is tried again.
pub(crate) fn members_looked_up(
    store: &Store,
    results: Vec<(String, Lookup)>,
) -> Result<MembersLookedUp, StoreError> {
    use libthreema::protobuf::d2d_sync::contact::AcquaintanceLevel;

    let step = store.step()?;
    let device_group = store.multi_device()?.is_some();
    let mut queued = false;
    let mut lines = Vec::new();
    let mut groups: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    let answered: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
    store.member_lookups_answered(&answered)?;
    for (identity, result) in results {
        let waiting = store.take_member_lookups(&identity)?;
        let found = matches!(result, Lookup::Found(_));
        if !found && store.contact(&identity)?.is_none() {
            store.reflected_forget_sender(&identity)?;
        }
        if let Lookup::Found(contact) = result
            && store.contact(&identity)?.is_none()
        {
            let mut contact = *contact;
            contact.acquaintance_level = AcquaintanceLevel::GroupOrDeleted;
            store.upsert_contact(&contact)?;
            // Whatever the lookup was for (a member, a creator asked for a group, a retry): a later
            // group sync can name them, and the other devices refuse one naming someone unknown.
            // A device that has them already discards this.
            if device_group {
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
            if found
                && added
                && let Some(contact) = store.contact(&identity)?
                && store
                    .group(&key)?
                    .is_some_and(|group| !group.left && group.members.contains(&identity))
            {
                lines.extend(store.insert_status(
                    &key,
                    &format!(
                        "groupMemberAdded|{}",
                        crate::contacts::display_name(&contact)
                    ),
                )?);
            }
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
            if !crate::join::group_syncable(store, &group)? {
                continue;
            }
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
    Ok(MembersLookedUp { queued, lines })
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
    let step = store.step()?;
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

/// A directory lookup changed `after`: the other devices get just those fields, as a rename made
/// meanwhile on one of them must stand (Android reflects each alone, `ContactUpdateWorker`).
/// Desktop looks up nobody itself: what it knows of who can do what and who was revoked comes from
/// here. Nothing without a device group, or if nothing it keeps changed.
pub(crate) fn queue_directory_change(
    store: &Store,
    before: &libthreema::model::contact::Contact,
    after: &libthreema::model::contact::Contact,
) -> Result<(), StoreError> {
    if store.multi_device()?.is_none() {
        return Ok(());
    }
    let full = after.to_d2d_sync();
    let mut changed = d2d_sync::Contact {
        identity: full.identity,
        ..Default::default()
    };
    if before.feature_mask.0 != after.feature_mask.0 {
        changed.feature_mask = full.feature_mask;
    }
    if before.activity_state != after.activity_state {
        changed.activity_state = full.activity_state;
    }
    if before.identity_type != after.identity_type {
        changed.identity_type = full.identity_type;
    }
    if changed.feature_mask.is_none()
        && changed.activity_state.is_none()
        && changed.identity_type.is_none()
    {
        return Ok(());
    }
    store.queue_sync_content(
        Scope::ContactSync,
        Content::ContactSync(ContactSync {
            action: Some(contact_sync::Action::Update(contact_sync::Update {
                contact: Some(changed),
            })),
        }),
    )
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
                events.push(CoreEvent::GroupDeleted(key));
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

pub fn settings_sync(
    store: &Store,
    sync: SettingsSync,
    events: &mut Vec<CoreEvent>,
) -> Result<(), StoreError> {
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
        // Contact pages say whether someone is blocked.
        if app.blocked_contacts != blocked.identities {
            events.push(CoreEvent::ContactsChanged);
        }
        for identity in &blocked.identities {
            store.reflected_forget_sender(identity)?;
        }
        app.blocked_contacts = blocked.identities;
    }
    if let Some(excluded) = synced.exclude_from_sync_identities {
        app.sync_excluded_ids = excluded.identities;
    }
    let json = serde_json::to_string(&app).unwrap_or_default();
    store.set_setting(keys::APP_SETTINGS, &json)?;
    events.push(CoreEvent::SettingsChanged);
    Ok(())
}

/// A link another device says is confirmed, or says isn't there. One still waiting for its
/// confirmation here stays if there is none: it wasn't known there. (Android forgets it.)
fn apply_identity_link(
    store: &Store,
    link: Option<String>,
    confirmed: &str,
    pending: &str,
) -> Result<(), StoreError> {
    match link.filter(|link| !link.is_empty()) {
        Some(link) => {
            store.set_setting(confirmed, &link)?;
            store.delete_setting(pending)
        }
        None if store.setting(pending)?.is_none() => store.delete_setting(confirmed),
        None => Ok(()),
    }
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
        let target = crate::profile_pictures::OWN_PICTURE.to_owned();
        note_picture(store, target, blob.as_ref(), events)?;
    }
    if profile.nickname.is_some()
        || profile.profile_picture_share_with.is_some()
        || profile.identity_links.is_some()
    {
        events.push(CoreEvent::ProfileChanged);
    }
    if let Some(nickname) = profile.nickname {
        if nickname.is_empty() {
            store.delete_setting(keys::NICKNAME)?;
        } else {
            store.set_setting(keys::NICKNAME, &nickname)?;
        }
        // Answered on another device.
        store.delete_setting(keys::NICKNAME_UNANSWERED)?;
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
    if let Some(links) = profile.identity_links {
        use d2d_sync::user_profile::identity_links::identity_link::Type;
        let (mut email, mut phone) = (None, None);
        for link in links.links {
            match link.r#type {
                Some(Type::Email(address)) if email.is_none() => email = Some(address),
                Some(Type::PhoneNumber(number)) if phone.is_none() => phone = Some(number),
                _ => {}
            }
        }
        apply_identity_link(store, email, keys::LINKED_EMAIL, keys::LINKED_EMAIL_PENDING)?;
        apply_identity_link(
            store,
            phone,
            keys::LINKED_MOBILE,
            keys::LINKED_MOBILE_PENDING,
        )?;
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

    /// A name from another device: the user's, unless that device says it is from an address book.
    #[test]
    fn a_name_from_another_device_stops_following_the_address_book() {
        let (_dir, store) = store();
        let state = |store: &Store| store.contact(BOB).unwrap().unwrap().sync_state;
        let apply = |store: &Store, synced: d2d_sync::Contact| {
            let mut events = Vec::new();
            assert!(apply_contact(store, synced, &mut events).unwrap());
        };
        let mut contact = crate::contacts::test_contact(BOB);
        contact.first_name = Some("Anna".into());
        contact.sync_state = contact::SyncState::Imported;
        store.upsert_contact(&contact).unwrap();
        let of_bob = || d2d_sync::Contact {
            identity: BOB.to_owned(),
            ..Default::default()
        };

        // Something else changes, or the same name comes again: it stays as it was.
        apply(
            &store,
            d2d_sync::Contact {
                nickname: Some("Nick".into()),
                first_name: Some("Anna".into()),
                last_name: Some(String::new()),
                ..of_bob()
            },
        );
        assert_eq!(state(&store), contact::SyncState::Imported);

        // Another device says where its name comes from.
        apply(
            &store,
            d2d_sync::Contact {
                first_name: Some("Anni".into()),
                sync_state: Some(contact::SyncState::Imported as i32),
                ..of_bob()
            },
        );
        assert_eq!(state(&store), contact::SyncState::Imported);

        // Only the name, as Android reflects a rename.
        apply(
            &store,
            d2d_sync::Contact {
                first_name: Some("Annika".into()),
                last_name: Some("Muster".into()),
                ..of_bob()
            },
        );
        let stored = store.contact(BOB).unwrap().unwrap();
        assert_eq!(stored.first_name.as_deref(), Some("Annika"));
        assert_eq!(stored.sync_state, contact::SyncState::Custom);

        // The next match leaves it alone: see `a_name_from_the_address_book_follows_it…`.
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
            CoreEvent::MessageAdded { message }
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
                "groupNoLongerMember",
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

    /// Deleted on another device: the chat is gone from the store here, and the app is told, as
    /// the picture and the media are for it to remove.
    /// What is open on the screen shows it at once.
    /// What a lookup changes reaches the other devices alone: a name changed there stands.
    #[test]
    fn a_lookup_tells_the_other_devices_only_what_it_changed() {
        use libthreema::{common::FeatureMask, protobuf::d2d::Envelope};
        use prost::Message as _;
        let (_dir, store) = store();
        let config = libthreema::common::config::Config::from(
            libthreema::common::config::ConfigEnvironment::Production,
        );
        let id = "ABCD1234";
        let mut known = crate::contacts::test_contact(id);
        known.feature_mask = FeatureMask(0);
        known.first_name = Some("Ann".into());
        store.upsert_contact(&known).unwrap();
        let mut fresh = crate::contacts::test_contact(id);
        fresh.feature_mask = FeatureMask(255);
        let found = |fresh: &libthreema::model::contact::Contact| {
            crate::contacts::Lookup::Found(Box::new(fresh.clone()))
        };

        // Nobody linked: nothing to tell.
        assert!(store.store_lookup(&config, id, found(&fresh)).unwrap());
        assert!(store.next_sync().unwrap().is_none());

        store
            .set_multi_device(Some(&crate::multidevice::Properties::generate()))
            .unwrap();
        // Nothing new: nothing to tell.
        assert!(!store.store_lookup(&config, id, found(&fresh)).unwrap());
        assert!(store.next_sync().unwrap().is_none());

        fresh.feature_mask = FeatureMask(511);
        assert!(store.store_lookup(&config, id, found(&fresh)).unwrap());
        let (entry, scope, content) = store.next_sync().unwrap().unwrap();
        store.sync_done(entry).unwrap();
        assert_eq!(scope, Scope::ContactSync as i32);
        let Some(Content::ContactSync(ContactSync {
            action: Some(contact_sync::Action::Update(update)),
        })) = Envelope::decode(content.as_slice()).unwrap().content
        else {
            panic!("not an update");
        };
        let synced = update.contact.unwrap();
        assert_eq!(synced.identity, id);
        assert_eq!(synced.feature_mask, Some(511));
        assert!(synced.activity_state.is_none());
        assert!(synced.first_name.is_none());
        assert!(synced.nickname.is_none());
        assert!(synced.acquaintance_level.is_none());

        // Revoked: they hear of that too, and nothing else.
        assert!(
            store
                .store_lookup(&config, id, crate::contacts::Lookup::Invalid)
                .unwrap()
        );
        let (entry, _, content) = store.next_sync().unwrap().unwrap();
        store.sync_done(entry).unwrap();
        let Some(Content::ContactSync(ContactSync {
            action: Some(contact_sync::Action::Update(update)),
        })) = Envelope::decode(content.as_slice()).unwrap().content
        else {
            panic!("not an update");
        };
        let synced = update.contact.unwrap();
        assert_eq!(
            synced.activity_state,
            Some(contact::ActivityState::Invalid as i32)
        );
        assert!(synced.feature_mask.is_none());
    }

    /// It comes once, and the fetch can fail or be cut off: what it said stays until it is done.
    #[test]
    fn a_synced_picture_is_noted_with_the_sync() {
        use libthreema::protobuf::common::{Blob, Image, image};
        let (_dir, store) = store();
        let updated = |byte: u8| DeltaImage {
            image: Some(delta_image::Image::Updated(Image {
                r#type: image::Type::Jpeg as i32,
                blob: Some(Blob {
                    id: vec![byte; 16],
                    nonce: vec![byte; 24],
                    key: vec![byte; 32],
                    uploaded_at: 0,
                }),
            })),
        };
        let profile = |picture| UserProfileSync {
            action: Some(user_profile_sync::Action::Update(
                user_profile_sync::Update {
                    user_profile: Some(d2d_sync::UserProfile {
                        profile_picture: Some(picture),
                        ..Default::default()
                    }),
                },
            )),
        };
        let own = crate::profile_pictures::OWN_PICTURE;
        let mut events = Vec::new();
        user_profile_sync(&store, profile(updated(1)), &mut events).unwrap();
        assert!(matches!(&events[..], [CoreEvent::SyncPicture { target }] if target == own));
        let (blob, _) = store.synced_picture(own).unwrap().unwrap();
        assert_eq!(blob.unwrap().key, [1; 32]);

        // Newer: replaces it.
        user_profile_sync(&store, profile(updated(2)), &mut Vec::new()).unwrap();
        let (blob, _) = store.synced_picture(own).unwrap().unwrap();
        assert_eq!(blob.unwrap().key, [2; 32]);

        let removed = DeltaImage {
            image: Some(delta_image::Image::Removed(Unit {})),
        };
        user_profile_sync(&store, profile(removed), &mut Vec::new()).unwrap();
        let (blob, _) = store.synced_picture(own).unwrap().unwrap();
        assert!(blob.is_none());
    }

    #[test]
    fn a_change_from_another_device_tells_the_ui() {
        let (_dir, store) = store();
        let mut events = Vec::new();
        let profile = |nickname: Option<&str>| UserProfileSync {
            action: Some(user_profile_sync::Action::Update(
                user_profile_sync::Update {
                    user_profile: Some(d2d_sync::UserProfile {
                        nickname: nickname.map(str::to_owned),
                        ..Default::default()
                    }),
                },
            )),
        };
        user_profile_sync(&store, profile(Some("Ann")), &mut events).unwrap();
        assert!(matches!(events[..], [CoreEvent::ProfileChanged]));
        events.clear();
        user_profile_sync(&store, profile(None), &mut events).unwrap();
        assert!(events.is_empty());

        let settings = |blocked: Option<Vec<String>>| SettingsSync {
            action: Some(settings_sync::Action::Update(settings_sync::Update {
                settings: Some(d2d_sync::Settings {
                    blocked_identities: blocked.map(|identities| Identities { identities }),
                    ..Default::default()
                }),
            })),
        };
        settings_sync(&store, settings(None), &mut events).unwrap();
        assert!(matches!(events[..], [CoreEvent::SettingsChanged]));
        events.clear();
        // Someone's page says they are blocked.
        settings_sync(&store, settings(Some(vec![BOB.to_owned()])), &mut events).unwrap();
        assert!(matches!(
            events[..],
            [CoreEvent::ContactsChanged, CoreEvent::SettingsChanged]
        ));
        events.clear();
        settings_sync(&store, settings(Some(vec![BOB.to_owned()])), &mut events).unwrap();
        assert!(matches!(events[..], [CoreEvent::SettingsChanged]));
    }

    #[test]
    fn links_from_another_device_are_kept_and_one_waiting_here_stays() {
        use d2d_sync::user_profile::{
            IdentityLinks,
            identity_links::{IdentityLink, identity_link::Type},
        };
        let (_dir, store) = store();
        let apply = |links: Option<Vec<Type>>| {
            let profile = d2d_sync::UserProfile {
                identity_links: links.map(|links| IdentityLinks {
                    links: links
                        .into_iter()
                        .map(|link| IdentityLink {
                            r#type: Some(link),
                            description: String::new(),
                        })
                        .collect(),
                }),
                ..Default::default()
            };
            user_profile_sync(
                &store,
                UserProfileSync {
                    action: Some(user_profile_sync::Action::Update(
                        user_profile_sync::Update {
                            user_profile: Some(profile),
                        },
                    )),
                },
                &mut Vec::new(),
            )
            .unwrap();
        };
        let setting = |key: &str| store.setting(key).unwrap();

        apply(Some(vec![
            Type::Email("ab@example.org".into()),
            Type::PhoneNumber("41791234567".into()),
        ]));
        assert_eq!(
            setting(keys::LINKED_EMAIL).as_deref(),
            Some("ab@example.org")
        );
        assert_eq!(setting(keys::LINKED_MOBILE).as_deref(), Some("41791234567"));

        // Nothing said about links: they stay.
        apply(None);
        assert!(setting(keys::LINKED_EMAIL).is_some());

        // Removed there, and one waiting for its code here stays.
        store
            .set_setting(keys::LINKED_MOBILE_PENDING, "41790000000 v1")
            .unwrap();
        store.delete_setting(keys::LINKED_MOBILE).unwrap();
        apply(Some(vec![]));
        assert!(setting(keys::LINKED_EMAIL).is_none());
        assert!(setting(keys::LINKED_MOBILE_PENDING).is_some());

        // Confirmed there: nothing is waiting any more.
        apply(Some(vec![Type::PhoneNumber("41790000000".into())]));
        assert!(setting(keys::LINKED_MOBILE_PENDING).is_none());
        assert_eq!(setting(keys::LINKED_MOBILE).as_deref(), Some("41790000000"));
    }

    #[test]
    fn a_group_deleted_on_another_device_goes_with_all_that_is_its() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db"), &[1; 32]).unwrap();
        let identity = libthreema::common::GroupIdentity {
            creator_identity: "BBBBBBBB".parse().unwrap(),
            group_id: 7,
        };
        let key = crate::groups::key(&identity);
        store
            .upsert_group(&crate::groups::Group {
                key: key.clone(),
                identity,
                name: "Crew".into(),
                members: vec!["BBBBBBBB".into()],
                left: true,
                created_at: 1,
                picture: None,
                picture_version: 0,
            })
            .unwrap();
        store.set_pinned(&key, true).unwrap();
        let mut events = Vec::new();
        group_sync(
            &store,
            "ABCD1234",
            GroupSync {
                action: Some(group_sync::Action::Delete(group_sync::Delete {
                    group_identity: Some(ProtoGroupIdentity {
                        group_id: 7,
                        creator_identity: "BBBBBBBB".into(),
                    }),
                })),
            },
            &mut events,
        )
        .unwrap();
        assert!(store.group(&key).unwrap().is_none());
        assert!(store.pinned_chats().unwrap().is_empty());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, CoreEvent::GroupDeleted(deleted) if *deleted == key))
        );
    }

    /// What one device sends of a contact, the other stores and would send the same way back.
    fn round_trip_contact(from: &Store, to: &Store, create: bool) -> Vec<CoreEvent> {
        let synced = crate::join::contact_sync(from, &from.contact(BOB).unwrap().unwrap()).unwrap();
        let action = if create {
            contact_sync::Action::Create(contact_sync::Create {
                contact: Some(synced.clone()),
            })
        } else {
            contact_sync::Action::Update(contact_sync::Update {
                contact: Some(synced.clone()),
            })
        };
        let mut events = Vec::new();
        contact_sync(
            to,
            ContactSync {
                action: Some(action),
            },
            &mut events,
        )
        .unwrap();
        let back = crate::join::contact_sync(to, &to.contact(BOB).unwrap().unwrap()).unwrap();
        assert_eq!(back, synced);
        events
    }

    #[test]
    fn a_contact_goes_to_the_other_device_and_comes_back_the_same() {
        let (_one, here) = store();
        let (_two, there) = store();
        let mut bob = crate::contacts::test_contact(BOB);
        bob.first_name = Some("Bob".into());
        bob.last_name = Some("Builder".into());
        bob.nickname = Some("bob".into());
        bob.verification_level = contact::VerificationLevel::FullyVerified;
        bob.acquaintance_level = contact::AcquaintanceLevel::Direct;
        bob.sync_state = contact::SyncState::Custom;
        bob.read_receipt_policy_override = Some(ReadReceiptPolicy::DontSendReadReceipt);
        bob.typing_indicator_policy_override = Some(TypingIndicatorPolicy::DontSendTypingIndicator);
        bob.conversation_category = ConversationCategory::Protected;
        here.upsert_contact(&bob).unwrap();
        here.set_private(BOB, true).unwrap();
        let tomorrow = crate::messages::now_ms() + 86_400_000;
        here.set_chat_notifications(BOB, Some((Policy::Never, Some(tomorrow))))
            .unwrap();

        let events = round_trip_contact(&here, &there, true);
        assert!(there.is_private(BOB).unwrap(), "private there too");
        assert!(
            events
                .iter()
                .any(|event| matches!(event, CoreEvent::ContactsChanged))
        );

        // Everything back to its default, and the chat pinned (a private chat can't be).
        let mut plain = crate::contacts::test_contact(BOB);
        plain.acquaintance_level = contact::AcquaintanceLevel::Direct;
        plain.created_at = bob.created_at;
        here.upsert_contact(&plain).unwrap();
        here.set_private(BOB, false).unwrap();
        here.set_pinned(BOB, true).unwrap();
        here.set_chat_notifications(BOB, None).unwrap();
        round_trip_contact(&here, &there, false);
        assert!(!there.is_private(BOB).unwrap());
        assert_eq!(there.pinned_chats().unwrap(), [BOB]);

        // Archived, which unpins.
        here.set_archived(BOB, true).unwrap();
        round_trip_contact(&here, &there, false);
        assert!(there.pinned_chats().unwrap().is_empty());
        assert_eq!(there.archived_chats().unwrap(), [BOB]);
    }

    /// Absent means unchanged: an update naming one field leaves the verification, the private mark,
    /// the pin and the rest alone.
    #[test]
    fn a_contact_update_changes_only_what_it_names() {
        let (_dir, store) = store();
        let mut bob = crate::contacts::test_contact(BOB);
        bob.first_name = Some("Bob".into());
        bob.verification_level = contact::VerificationLevel::FullyVerified;
        bob.conversation_category = ConversationCategory::Protected;
        bob.read_receipt_policy_override = Some(ReadReceiptPolicy::DontSendReadReceipt);
        store.upsert_contact(&bob).unwrap();
        store.set_private(BOB, true).unwrap();
        store.set_pinned(BOB, true).unwrap();
        let before =
            crate::join::contact_sync(&store, &store.contact(BOB).unwrap().unwrap()).unwrap();

        let mut events = Vec::new();
        contact_sync(
            &store,
            ContactSync {
                action: Some(contact_sync::Action::Update(contact_sync::Update {
                    contact: Some(d2d_sync::Contact {
                        identity: BOB.to_owned(),
                        nickname: Some("bobby".into()),
                        ..Default::default()
                    }),
                })),
            },
            &mut events,
        )
        .unwrap();
        let after =
            crate::join::contact_sync(&store, &store.contact(BOB).unwrap().unwrap()).unwrap();
        assert_eq!(
            after,
            d2d_sync::Contact {
                nickname: Some("bobby".into()),
                ..before
            }
        );
        assert!(store.is_private(BOB).unwrap());
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, CoreEvent::PrivateChanged { .. }))
        );
    }

    #[test]
    fn a_new_contact_needs_its_key() {
        let (_dir, store) = store();
        let create = |public_key: Option<Vec<u8>>| ContactSync {
            action: Some(contact_sync::Action::Create(contact_sync::Create {
                contact: Some(d2d_sync::Contact {
                    identity: BOB.to_owned(),
                    public_key,
                    first_name: Some("Bob".into()),
                    ..Default::default()
                }),
            })),
        };
        for key in [None, Some(vec![3; 31])] {
            let mut events = Vec::new();
            contact_sync(&store, create(key), &mut events).unwrap();
            assert!(store.contact(BOB).unwrap().is_none());
            assert!(events.is_empty());
        }
        let mut events = Vec::new();
        contact_sync(&store, create(Some(vec![3; 32])), &mut events).unwrap();
        let bob = store.contact(BOB).unwrap().unwrap();
        assert!(bob.public_key == PublicKey::from([3; 32]));
        assert_eq!(bob.first_name.as_deref(), Some("Bob"));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, CoreEvent::ContactsChanged))
        );
    }

    fn apply_settings(store: &Store, settings: d2d_sync::Settings) -> Vec<CoreEvent> {
        let mut events = Vec::new();
        settings_sync(
            store,
            SettingsSync {
                action: Some(settings_sync::Action::Update(settings_sync::Update {
                    settings: Some(settings),
                })),
            },
            &mut events,
        )
        .unwrap();
        events
    }

    #[test]
    fn settings_go_to_the_other_device_and_come_back_the_same() {
        let (_one, here) = store();
        let (_two, there) = store();
        let defaults = crate::join::settings_sync(&here).unwrap();
        let changed = crate::safe::backup::Settings {
            sync_contacts: true,
            block_unknown: true,
            read_receipts: false,
            send_typing: false,
            threema_calls: false,
            relay_threema_calls: true,
            disable_screenshots: true,
            incognito_keyboard: true,
            blocked_contacts: vec![EVE.to_owned()],
            sync_excluded_ids: vec![BOB.to_owned()],
            ..Default::default()
        };
        here.set_setting(
            keys::APP_SETTINGS,
            &serde_json::to_string(&changed).unwrap(),
        )
        .unwrap();
        here.set_setting(keys::VIDEO_CALLS, "0").unwrap();
        here.set_setting(keys::GROUP_CALLS, "0").unwrap();
        let synced = crate::join::settings_sync(&here).unwrap();
        assert_ne!(synced, defaults);

        let events = apply_settings(&there, synced.clone());
        assert_eq!(crate::join::settings_sync(&there).unwrap(), synced);
        assert!(there.is_explicitly_blocked(EVE).unwrap());
        assert!(
            events
                .iter()
                .any(|event| matches!(event, CoreEvent::SettingsChanged))
        );

        // And back to the defaults.
        apply_settings(&there, defaults.clone());
        assert_eq!(crate::join::settings_sync(&there).unwrap(), defaults);
        assert!(!there.is_explicitly_blocked(EVE).unwrap());

        // A setting the message leaves out stays as it is.
        apply_settings(&there, synced.clone());
        apply_settings(
            &there,
            d2d_sync::Settings {
                read_receipt_policy: Some(ReadReceiptPolicy::SendReadReceipt as i32),
                ..Default::default()
            },
        );
        assert_eq!(
            crate::join::settings_sync(&there).unwrap(),
            d2d_sync::Settings {
                read_receipt_policy: Some(ReadReceiptPolicy::SendReadReceipt as i32),
                ..synced
            }
        );
    }

    #[test]
    fn the_profile_goes_to_the_other_device_and_comes_back_the_same() {
        use crate::profile_pictures::Share;
        let (_one, here) = store();
        let (_two, there) = store();
        let apply = |store: &Store, profile: d2d_sync::UserProfile| {
            let mut events = Vec::new();
            user_profile_sync(
                store,
                UserProfileSync {
                    action: Some(user_profile_sync::Action::Update(
                        user_profile_sync::Update {
                            user_profile: Some(profile),
                        },
                    )),
                },
                &mut events,
            )
            .unwrap();
        };
        let without_picture = |store: &Store| d2d_sync::UserProfile {
            profile_picture: None,
            ..crate::join::profile_sync(store).unwrap()
        };
        here.set_setting(keys::NICKNAME, "Tester").unwrap();
        here.set_picture_share(Share::Selected).unwrap();
        here.set_picture_recipients(&[BOB.to_owned(), EVE.to_owned()])
            .unwrap();
        there.set_setting(keys::NICKNAME_UNANSWERED, "ask").unwrap();

        apply(&there, without_picture(&here));
        assert_eq!(without_picture(&there), without_picture(&here));
        assert_eq!(there.setting(keys::NICKNAME_UNANSWERED).unwrap(), None);

        // Cleared here, cleared there.
        here.delete_setting(keys::NICKNAME).unwrap();
        here.set_picture_share(Share::Nobody).unwrap();
        apply(&there, without_picture(&here));
        assert_eq!(there.setting(keys::NICKNAME).unwrap(), None);
        assert_eq!(without_picture(&there), without_picture(&here));
    }
}
