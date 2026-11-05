// SPDX-License-Identifier: AGPL-3.0-only
//! The history import job: streams chats out of the archive on a blocking thread, applies
//! them on the core thread in bounded, fully synchronous transactions (no await while one is
//! open: every task shares the store connection), and only marks media local once its bytes are
//! written and authenticated. Cancellation stops between batches; done batches stay (the whole
//! import is idempotent, so running it again resumes). A store that refuses a write ends the job
//! with its error: an archive's bad row is counted, a full disk isn't a bad row.

use std::{cell::Cell, rc::Rc, sync::Arc};

use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{
    Archive, ArchiveError, EntryKind, convert, place::Placer, polls::ArchivePolls, records,
};
use crate::{
    media::{MAX_BLOB_SIZE, MediaPart, MediaStore},
    messages::{ImportedMessage, OutgoingState},
    providers::CoreEvent,
    store::{Store, StoreError},
};

/// Stable ids for rows without a valid `apiid` (store migration 29): keyed by the archive's
/// fingerprint, record family, conversation and Android `uid`, so a resumed or repeated
/// import of the same backup never mints a second id.
pub(crate) const MIGRATION_IMPORT_IDS: &str = "
    CREATE TABLE android_import_ids (
        source        TEXT NOT NULL,
        family        TEXT NOT NULL,
        conversation  TEXT NOT NULL,
        uid           TEXT NOT NULL,
        message_id    INTEGER NOT NULL,
        PRIMARY KEY (source, family, conversation, uid)
    ) WITHOUT ROWID;";

const BATCH: usize = 400;
/// How many left-out chats the summary names; a crafted archive could leave out thousands, and
/// the counts hold them all.
const MAX_LEFT_OUT_NAMED: usize = 100;
/// How much of one table is held in memory (tests stay at what they can write quickly).
const MAX_TABLE_BYTES: usize = if cfg!(test) { 1 << 20 } else { 64 << 20 };

/// A file message whose bytes are applied after its row is committed.
struct MediaJob {
    row: i64,
    uid: String,
    outgoing: bool,
}

impl Store {
    /// The stable synthetic message id for an archive row without a valid `apiid`.
    pub fn android_import_message_id(
        &self,
        source: &str,
        family: &str,
        conversation: &str,
        uid: &str,
    ) -> Result<u64, StoreError> {
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT message_id FROM android_import_ids
                 WHERE source = ?1 AND family = ?2 AND conversation = ?3 AND uid = ?4",
                params![source, family, conversation, uid],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id.cast_unsigned());
        }
        let id: u64 = rand::random();
        self.conn.execute(
            "INSERT OR IGNORE INTO android_import_ids
                 (source, family, conversation, uid, message_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![source, family, conversation, uid, id.cast_signed()],
        )?;
        Ok(id)
    }
}

/// What kind of chat an archive file belongs to; decides media entry names and senders.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Family {
    Contact,
    Group,
    List,
}

impl Family {
    fn name(self) -> &'static str {
        match self {
            Self::Contact => "contact",
            Self::Group => "group",
            Self::List => "list",
        }
    }
}

/// What the archive holds beside its chats, looked up while they are imported.
#[derive(Default)]
struct ArchiveIndex {
    /// Media and thumbnail entries, by family and uid.
    media: std::collections::HashMap<(&'static str, String), String>,
    thumbnails: std::collections::HashMap<(&'static str, String), String>,
    /// People whose key in the archive differs from the one stored here. The archive may be old
    /// or wrong about them, so nothing it says in their name goes into their chat, groups or
    /// messages here.
    other_key: std::collections::HashSet<String>,
}

/// One chat file of the archive, resolved to a local conversation.
struct ChatPlan {
    entry: String,
    conversation: String,
    family: Family,
}

/// A chat the import left out, for the result page. Why: `notHere`, `tooLarge`, `damaged`,
/// `otherKey`.
#[derive(serde::Serialize)]
pub struct LeftOutChat {
    pub conversation: String,
    pub name: String,
    pub reason: &'static str,
}

#[derive(Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub chats: usize,
    /// Chat files whose conversation is not on this device (metadata import pending).
    pub skipped_chats: usize,
    /// Chat files beyond what is read into memory: left out, the rest of the archive goes on.
    pub too_large_chats: usize,
    /// Chat files that fail authentication or aren't text: left out as a whole, the rest goes on.
    pub damaged_chats: usize,
    /// Which chats those are, as far as they have a name. A private chat is never named here: the
    /// page shows while the private chats may be hidden, and only the core knows them.
    pub left_out_chats: Vec<LeftOutChat>,
    /// Chats of contacts whose key in the archive differs from the one stored here: left out.
    pub key_conflict_chats: usize,
    /// Group messages and reactions in the name of such a contact: left out.
    pub key_conflict_rows: usize,
    pub imported: usize,
    pub existing: usize,
    /// Same message id from a different group sender: reported, neither record touched.
    pub collisions: usize,
    /// Outgoing rows that never made it out on Android: kept as failed, never re-queued.
    pub unsent: usize,
    pub invalid_rows: usize,
    pub unsupported: std::collections::BTreeMap<&'static str, usize>,
    pub compat: std::collections::BTreeMap<&'static str, usize>,
    pub media_files: usize,
    pub thumbnails: usize,
    /// File messages whose bytes are not in the archive (metadata-only backup): the message
    /// stays, availability reflects the missing bytes.
    pub media_missing: usize,
    pub media_failed: usize,
    pub reactions: usize,
    pub reactions_unmatched: usize,
    /// Poll messages imported with their poll (also counted in `imported`).
    pub polls: usize,
    /// Rows whose `uid` another chat of the same kind in this archive already holds. Android's
    /// uids are unique, so something copied a chat file: a hint, see [`CrossChecks`].
    pub uid_conflicts: usize,
    /// Group chats whose incoming rows mostly come from people outside the group: a hint too.
    pub foreign_group_chats: usize,
    pub cancelled: bool,
}

/// Fewest incoming rows for a verdict on who wrote a group chat: a short chat may simply be
/// the words of someone who left.
const MIN_ROWS_FOR_SENDER_CHECK: usize = 10;

/// Hints that an archive's files were moved around. Neither the entry names nor the set of
/// entries are authenticated (docs/android-backup-design.md), and these only notice the crude
/// cases: no hint means nothing, and a hint doesn't stop the import. Uids are kept as hashes, so
/// the memory stays small however many rows pass by.
#[derive(Default)]
struct CrossChecks {
    hasher: std::hash::RandomState,
    owners: std::collections::HashMap<u64, usize>,
}

impl CrossChecks {
    /// How many of a chat's rows carry a uid another chat of the family already holds.
    fn claimed_elsewhere(
        &mut self,
        chat: usize,
        family: Family,
        messages: &[records::Message],
    ) -> usize {
        use std::hash::BuildHasher as _;
        messages
            .iter()
            .filter(|message| {
                let uid = self.hasher.hash_one((family.name(), &message.uid));
                *self.owners.entry(uid).or_insert(chat) != chat
            })
            .count()
    }
}

/// Whether most incoming rows come from people outside the group as it stands. People who left
/// count as outside, which is why only a majority is worth a hint. The user's own rows are not
/// incoming, even where Android writes the user's ID into them (polls), and the member list
/// doesn't hold the user. A set, because the core thread runs this and a file decides both sizes.
fn mostly_from_non_members(
    members: &[String],
    creator: &str,
    messages: &[records::Message],
) -> bool {
    let members: std::collections::HashSet<&str> = members.iter().map(String::as_str).collect();
    let (mut incoming, mut foreign) = (0, 0);
    for sender in messages
        .iter()
        .filter(|message| !message.outgoing)
        .filter_map(|message| message.sender.as_deref())
    {
        incoming += 1;
        if sender != creator && !members.contains(sender) {
            foreign += 1;
        }
    }
    incoming >= MIN_ROWS_FOR_SENDER_CHECK && foreign * 2 > incoming
}

/// What the blocking thread brings back from opening the archive.
struct OpenedArchive {
    archive: Archive,
    source: String,
    contacts: Vec<records::Contact>,
    groups: Vec<records::Group>,
    polls: ArchivePolls,
    invalid_polls: usize,
}

/// Everything the job owns (nothing borrows the [`crate::app::App`]).
pub struct HistoryImport {
    pub store: Rc<Store>,
    pub media: Arc<MediaStore>,
    pub events: tokio::sync::mpsc::UnboundedSender<CoreEvent>,
    pub cancel: Rc<Cell<bool>>,
    pub me: String,
    pub own_public_key: String,
    pub path: std::path::PathBuf,
    pub password: Zeroizing<String>,
}

impl HistoryImport {
    fn emit(&self, value: Value) {
        let _ = self.events.send(CoreEvent::AndroidBackup(value));
    }

    pub async fn run(self) {
        let summary = self.run_inner().await;
        match summary {
            Ok(summary) => {
                tracing::info!(
                    chats = summary.chats,
                    skipped = summary.skipped_chats,
                    too_large = summary.too_large_chats,
                    damaged = summary.damaged_chats,
                    key_conflicts = summary.key_conflict_chats + summary.key_conflict_rows,
                    invalid_rows = summary.invalid_rows,
                    imported = summary.imported,
                    existing = summary.existing,
                    media = summary.media_files,
                    cancelled = summary.cancelled,
                    "Android history import finished"
                );
                // The lists rebuild from scratch: an import is a bulk change, not a stream of
                // messageAdded events.
                self.emit(json!({ "event": "chatsChanged" }));
                self.emit(json!({ "event": "contactsChanged" }));
                self.emit(json!({
                    "event": "androidBackupDone",
                    "ok": true,
                    "summary": serde_json::to_value(&summary).unwrap_or(Value::Null),
                }));
            }
            Err(error) => {
                tracing::warn!(kind = error.kind(), "Android history import failed");
                self.emit(json!({
                    "event": "androidBackupDone",
                    "ok": false,
                    "kind": error.kind(),
                }));
            }
        }
        self.cancel.set(true);
    }

    async fn run_inner(&self) -> Result<Summary, crate::app::ApiError> {
        use crate::app::ApiError;
        let path = self.path.clone();
        let password = self.password.clone();
        let own_id = self.me.clone();
        let own_key = self.own_public_key.clone();
        // Open, gate, fingerprint and read the small tables on the blocking thread; the archive
        // moves back and forth between the threads from here on. Tables are read and parsed
        // there too: the core thread keeps serving commands and the connection.
        let opened = tokio::task::spawn_blocking(move || -> Result<OpenedArchive, ApiError> {
            let mut archive = Archive::open(&path, &password)?;
            let _version = super::read_version(&mut archive)?;
            let encrypted = super::read_identity_backup(&mut archive)?;
            let (_, identity) =
                libthreema::id_backup::decrypt_identity_backup(&password, &encrypted)
                    .map_err(|_| ApiError::BackupDecrypt)?;
            if identity.threema_id.to_string() != own_id
                || identity.client_key.public_key().to_string() != own_key
            {
                return Err(ApiError::BackupOtherIdentity);
            }
            let source = fingerprint_file(&path)?;
            let contacts = optional_table(&mut archive, "contacts.csv")?
                .map(|table| records::contacts(&table).0)
                .unwrap_or_default();
            let groups = optional_table(&mut archive, "groups.csv")?
                .map(|table| records::groups(&table).0)
                .unwrap_or_default();
            let (polls, invalid_polls) = read_polls(&mut archive)?;
            Ok(OpenedArchive {
                archive,
                source,
                contacts,
                groups,
                polls,
                invalid_polls,
            })
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;
        let OpenedArchive {
            mut archive,
            source,
            contacts,
            groups,
            polls,
            invalid_polls,
        } = opened;

        let mut summary = Summary {
            invalid_rows: invalid_polls,
            ..Summary::default()
        };

        // Resolve every chat file to a local conversation; media/thumbnail presence by uid.
        let (plans, index) = self.plan_chats(&archive, contacts, &groups, &mut summary)?;
        let total = plans.len();
        let mut checks = CrossChecks::default();

        for (done, plan) in plans.into_iter().enumerate() {
            if self.cancel.get() {
                summary.cancelled = true;
                break;
            }
            self.emit(json!({
                "event": "androidBackupProgress",
                "step": "messages",
                "done": done,
                "total": total,
                "imported": summary.imported,
            }));
            let entry = plan.entry.clone();
            let (returned, parsed) = tokio::task::spawn_blocking(move || {
                let parsed = archive.read_table(&entry, MAX_TABLE_BYTES).map(|table| {
                    let (mut messages, invalid) = records::messages(&table);
                    // Oldest first, deterministic ties: placement needs ascending times.
                    messages.sort_by(|a, b| a.at.cmp(&b.at).then_with(|| a.uid.cmp(&b.uid)));
                    (messages, invalid)
                });
                (archive, parsed)
            })
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
            archive = returned;
            let (messages, invalid) = match parsed {
                Ok(parsed) => parsed,
                // Every run would stop at this chat, with the later ones and all reactions behind
                // it: it is left out and reported, the memory bound stays.
                Err(ArchiveError::TooLarge) => {
                    summary.too_large_chats += 1;
                    self.name_left_out(&plan.conversation, None, "tooLarge", &mut summary);
                    continue;
                }
                // The entry is read to its end first, so none of it went in. The same place
                // would end every run: an altered or damaged file costs this chat, and says so.
                Err(
                    ArchiveError::Corrupt(_)
                    | ArchiveError::Invalid(_)
                    | ArchiveError::WrongPassword,
                ) => {
                    summary.damaged_chats += 1;
                    self.name_left_out(&plan.conversation, None, "damaged", &mut summary);
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            summary.invalid_rows += invalid;
            summary.uid_conflicts += checks.claimed_elsewhere(done, plan.family, &messages);
            if plan.family == Family::Group
                && let Ok(Some(group)) = self.store.group(&plan.conversation)
                && mostly_from_non_members(
                    &group.members,
                    &group.identity.creator_identity.to_string(),
                    &messages,
                )
            {
                summary.foreign_group_chats += 1;
            }

            archive = self
                .import_chat(
                    archive,
                    &plan,
                    &source,
                    messages,
                    &index,
                    &polls,
                    &mut summary,
                )
                .await?;
            summary.chats += 1;
        }

        if !summary.cancelled {
            archive = self.import_reactions(archive, &index, &mut summary).await?;
        }
        drop(archive);
        // Partially written media without a committed row are removed like other orphans.
        let store = Rc::clone(&self.store);
        self.media
            .remove_orphans(move |id| store.message_exists(id).unwrap_or(true));
        Ok(summary)
    }

    /// Note which chat was left out: by its name here, else the one the archive gives it.
    fn name_left_out(
        &self,
        conversation: &str,
        archive_name: Option<&str>,
        reason: &'static str,
        summary: &mut Summary,
    ) {
        if summary.left_out_chats.len() >= MAX_LEFT_OUT_NAMED
            || self.store.is_private(conversation).unwrap_or(true)
        {
            return;
        }
        let stored = if crate::groups::parse_key(conversation).is_some() {
            self.store
                .group(conversation)
                .ok()
                .flatten()
                .map(|group| group.name)
        } else if conversation.starts_with("d:") {
            self.store
                .distribution_list(conversation)
                .ok()
                .flatten()
                .map(|list| list.name)
        } else {
            let contact = self.store.contact(conversation).ok().flatten();
            Some(contact.map_or_else(
                || conversation.to_owned(),
                |contact| crate::contacts::display_name(&contact),
            ))
        };
        let name = stored.or_else(|| archive_name.map(str::to_owned));
        if let Some(name) = name.filter(|name| !name.is_empty()) {
            summary.left_out_chats.push(LeftOutChat {
                conversation: conversation.to_owned(),
                name,
                reason,
            });
        }
    }

    /// Chat files resolved to local conversations, plus which uids have media/thumbnails.
    fn plan_chats(
        &self,
        archive: &Archive,
        contacts: Vec<records::Contact>,
        groups: &[records::Group],
        summary: &mut Summary,
    ) -> Result<(Vec<ChatPlan>, ArchiveIndex), StoreError> {
        let stored_keys: std::collections::HashMap<String, Vec<u8>> = self
            .store
            .contacts()?
            .iter()
            .map(|contact| {
                (
                    contact.identity.to_string(),
                    crate::contacts::hex_bytes(&contact.public_key.to_string()),
                )
            })
            .collect();
        let mut index = ArchiveIndex::default();
        // Alias maps from the metadata CSVs (the metadata import must have run: chats whose
        // conversation is missing are counted and skipped, nothing is created here).
        let mut contact_alias = std::collections::HashMap::new();
        for contact in contacts {
            if stored_keys
                .get(&contact.identity)
                .is_some_and(|key| key.as_slice() != contact.public_key)
            {
                index.other_key.insert(contact.identity.clone());
            }
            let alias = contact
                .alias
                .clone()
                .unwrap_or_else(|| contact.identity.clone());
            contact_alias.insert(alias, contact.identity.clone());
            contact_alias.insert(contact.identity.clone(), contact.identity);
        }
        let mut group_alias = std::collections::HashMap::new();
        let mut group_names = std::collections::HashMap::new();
        for group in groups {
            if let Ok(creator) = group.creator.parse() {
                let key = crate::groups::key(&libthreema::common::GroupIdentity {
                    creator_identity: creator,
                    group_id: group.group_id,
                });
                group_names.insert(key.clone(), group.name.as_str());
                for alias in group.aliases() {
                    group_alias.insert(alias, key.clone());
                }
            }
        }
        let mut plans = Vec::new();
        for entry in archive.entries() {
            let (family, conversation) = match &entry.kind {
                EntryKind::ContactMessages(alias) => {
                    (Family::Contact, contact_alias.get(alias).cloned())
                }
                EntryKind::GroupMessages(alias) => (Family::Group, group_alias.get(alias).cloned()),
                EntryKind::ListMessages(alias) => (
                    Family::List,
                    alias
                        .parse::<i64>()
                        .ok()
                        .map(|id| format!("d:{:016x}", id.cast_unsigned())),
                ),
                EntryKind::ContactMedia(uid) => {
                    index
                        .media
                        .insert(("contact", uid.clone()), entry.name.clone());
                    continue;
                }
                EntryKind::GroupMedia(uid) => {
                    index
                        .media
                        .insert(("group", uid.clone()), entry.name.clone());
                    continue;
                }
                EntryKind::ListMedia(uid) => {
                    index
                        .media
                        .insert(("list", uid.clone()), entry.name.clone());
                    continue;
                }
                EntryKind::ContactThumbnail(uid) => {
                    index
                        .thumbnails
                        .insert(("contact", uid.clone()), entry.name.clone());
                    continue;
                }
                EntryKind::GroupThumbnail(uid) => {
                    index
                        .thumbnails
                        .insert(("group", uid.clone()), entry.name.clone());
                    continue;
                }
                EntryKind::ListThumbnail(uid) => {
                    index
                        .thumbnails
                        .insert(("list", uid.clone()), entry.name.clone());
                    continue;
                }
                _ => continue,
            };
            let exists = |key: &str| match family {
                Family::Contact => self.store.contact(key).ok().flatten().is_some(),
                Family::Group => self.store.group(key).ok().flatten().is_some(),
                Family::List => self.store.distribution_list(key).ok().flatten().is_some(),
            };
            match conversation {
                Some(key) if family == Family::Contact && index.other_key.contains(&key) => {
                    summary.key_conflict_chats += 1;
                    self.name_left_out(&key, None, "otherKey", summary);
                }
                Some(key) if exists(&key) => plans.push(ChatPlan {
                    entry: entry.name.clone(),
                    conversation: key,
                    family,
                }),
                conversation => {
                    summary.skipped_chats += 1;
                    if let Some(key) = conversation {
                        let name = group_names.get(&key).copied();
                        self.name_left_out(&key, name, "notHere", summary);
                    }
                }
            }
        }
        Ok((plans, index))
    }

    /// Insert one chat's rows in bounded transactions, then its media.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    async fn import_chat(
        &self,
        mut archive: Archive,
        plan: &ChatPlan,
        source: &str,
        messages: Vec<records::Message>,
        index: &ArchiveIndex,
        polls: &ArchivePolls,
        summary: &mut Summary,
    ) -> Result<Archive, crate::app::ApiError> {
        let key = &plan.conversation;
        let mut placer = Placer::new(self.store.conversation_positions(key)?, messages.len());
        let mut media_jobs: Vec<MediaJob> = Vec::new();

        for chunk in messages.chunks(BATCH) {
            if self.cancel.get() {
                summary.cancelled = true;
                break;
            }
            // Fully synchronous within the step: the connection is shared with live work, so no
            // await may interleave while a transaction is open. An error undoes the batch.
            let step = self.store.step()?;
            for message in chunk {
                self.import_message(
                    plan,
                    source,
                    message,
                    &mut placer,
                    index,
                    polls,
                    &mut media_jobs,
                    summary,
                )?;
            }
            step.commit()?;
            tokio::task::yield_now().await;
        }

        // Media: write bytes first (authenticated by reading the entry to its end), then
        // advance the states — never the other way round.
        for (done, job) in media_jobs.iter().enumerate() {
            if self.cancel.get() {
                summary.cancelled = true;
                break;
            }
            if done % 10 == 0 {
                self.emit(json!({
                    "event": "androidBackupProgress",
                    "step": "media",
                    "done": done,
                    "total": media_jobs.len(),
                    "imported": summary.imported,
                    "mediaFiles": summary.media_files,
                }));
            }
            for (part, entries) in [
                (MediaPart::File, &index.media),
                (MediaPart::Thumbnail, &index.thumbnails),
            ] {
                let Some(entry) = entries.get(&(plan.family.name(), job.uid.clone())) else {
                    continue;
                };
                let name = entry.clone();
                let media = Arc::clone(&self.media);
                let row = job.row;
                let (returned, written) =
                    tokio::task::spawn_blocking(move || -> (Archive, Result<(), ArchiveError>) {
                        let result = archive.read(&name, MAX_BLOB_SIZE).and_then(|data| {
                            media
                                .write(row, part, &data)
                                .map_err(|e| ArchiveError::Invalid(e.to_string()))
                        });
                        (archive, result)
                    })
                    .await
                    .map_err(|e| crate::app::ApiError::Internal(e.to_string()))?;
                archive = returned;
                match written {
                    Ok(()) => {
                        self.store
                            .set_media_state(job.row, part, crate::app_media::STATE_LOCAL)?;
                        if job.outgoing {
                            self.store
                                .set_upload_state(job.row, crate::app_media::STATE_LOCAL)?;
                        }
                        if part == MediaPart::File {
                            summary.media_files += 1;
                        } else {
                            summary.thumbnails += 1;
                        }
                    }
                    Err(ArchiveError::TooLarge) => summary.media_failed += 1,
                    Err(ArchiveError::Corrupt(_) | ArchiveError::Invalid(_)) => {
                        summary.media_failed += 1;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(archive)
    }

    /// One row: convert, place, insert (or repair a row from an earlier run).
    #[allow(clippy::too_many_arguments)]
    fn import_message(
        &self,
        plan: &ChatPlan,
        source: &str,
        message: &records::Message,
        placer: &mut Placer,
        index: &ArchiveIndex,
        polls: &ArchivePolls,
        media_jobs: &mut Vec<MediaJob>,
        summary: &mut Summary,
    ) -> Result<(), StoreError> {
        let key = &plan.conversation;
        let mut converted = convert::convert(message);
        // Polls: the message row only points at `ballot.csv`; the state comes from there.
        let mut poll = None;
        if message.kind == "BALLOT"
            && matches!(converted, convert::Converted::Unsupported("poll"))
            && let Some((kind, state)) = polls.message(&message.body)
        {
            poll = super::polls::key(state);
            converted = convert::Converted::Message {
                kind,
                body: state.to_string(),
                file: None,
                compat: None,
            };
        }
        let (kind, mut body, file, compat) = match converted {
            convert::Converted::Message {
                kind,
                body,
                file,
                compat,
            } => (kind, body, file, compat),
            convert::Converted::Unsupported(category) => {
                *summary.unsupported.entry(category).or_default() += 1;
                return Ok(());
            }
        };
        if let Some(compat) = compat {
            *summary.compat.entry(compat).or_default() += 1;
        }
        let message_id = match message.api_id {
            Some(id) => id,
            None => self.store.android_import_message_id(
                source,
                plan.family.name(),
                key,
                &message.uid,
            )?,
        };
        // Availability of file content reflects the archive, not Android's stale flag.
        let has_media = index
            .media
            .contains_key(&(plan.family.name(), message.uid.clone()));
        let has_thumbnail = index
            .thumbnails
            .contains_key(&(plan.family.name(), message.uid.clone()));
        if kind == "file" {
            let mut value: Value = serde_json::from_str(&body).unwrap_or_default();
            // Failed until the media job has written the bytes; not "on the server", which would
            // send every old file of a backup without media to the blob server by itself. A tap
            // still tries, with the blob key kept from the backup.
            value["fileState"] = crate::app_media::STATE_FAILED.into();
            value["thumbnail"] = has_thumbnail.into();
            value["thumbnailState"] = crate::app_media::STATE_FAILED.into();
            if message.outgoing {
                value["uploadState"] = crate::app_media::STATE_LOCAL.into();
            }
            body = value.to_string();
        }
        let sender = match plan.family {
            Family::Group if !message.outgoing => match &message.sender {
                Some(sender) if index.other_key.contains(sender) => {
                    summary.key_conflict_rows += 1;
                    return Ok(());
                }
                Some(sender) => Some(sender.clone()),
                None => {
                    summary.invalid_rows += 1;
                    return Ok(());
                }
            },
            _ => None,
        };
        let state = if message.outgoing {
            if message.unsent() {
                summary.unsent += 1;
                OutgoingState::Failed as i64
            } else {
                match message.state.as_str() {
                    "SENT" => OutgoingState::Sent as i64,
                    "DELIVERED" => OutgoingState::Delivered as i64,
                    "USERACK" | "USERDEC" => {
                        if message.read_at.is_some() {
                            OutgoingState::Read as i64
                        } else {
                            OutgoingState::Delivered as i64
                        }
                    }
                    "READ" | "CONSUMED" => OutgoingState::Read as i64,
                    // Empty on Android's status rows, or a name this format didn't know yet:
                    // nothing says the recipient got it, only that it left.
                    _ => OutgoingState::Sent as i64,
                }
            }
        } else if message.read {
            crate::messages::INCOMING_READ
        } else {
            crate::messages::INCOMING_UNREAD
        };
        let sort_key = placer.place(message.at);
        let inserted = self.store.insert_imported(&ImportedMessage {
            conversation: key,
            message_id,
            outgoing: message.outgoing,
            kind,
            body: &body,
            created_at: message.at,
            updated_at: message.modified_at.unwrap_or(message.at),
            state,
            edited_at: message.deleted_at.or(message.edited_at),
            sender: sender.as_deref(),
            starred: message.starred,
            // Imported outgoing rows carry no server-accept time.
            sent_at: None,
            delivered_at: message.delivered_at,
            read_at: message.read_at,
            received_at: None,
            sort_key,
        })?;
        let row = match inserted {
            Some(row) => row,
            None => {
                // Already here: a live row or an earlier run. The existing row wins; a group
                // row from a different sender is a real collision and reported.
                match self
                    .store
                    .message_by_id(key, message.outgoing, message_id)?
                {
                    Some(existing) => {
                        if sender.is_some() && existing.sender != sender {
                            summary.collisions += 1;
                        } else {
                            summary.existing += 1;
                            // Repair pass: an earlier run may have left the row without what
                            // came after it. Keys and bytes already stored stay untouched (and
                            // uncounted), so importing again moves no media. A row that isn't a
                            // file (any more) gets nothing: a deleted message stays deleted.
                            if kind == "file" && existing.kind == "file" {
                                if self.store.file_keys(existing.id)?.is_none() {
                                    self.keep_file_keys(existing.id, file.as_ref())?;
                                }
                                let stored: Value =
                                    serde_json::from_str(&existing.body).unwrap_or_default();
                                let local = |part: &str| {
                                    stored[part].as_i64() == Some(crate::app_media::STATE_LOCAL)
                                };
                                summary.media_missing +=
                                    usize::from(!has_media && !local("fileState"));
                                if (has_media && !local("fileState"))
                                    || (has_thumbnail && !local("thumbnailState"))
                                {
                                    media_jobs.push(MediaJob {
                                        row: existing.id,
                                        uid: message.uid.clone(),
                                        outgoing: message.outgoing,
                                    });
                                }
                            }
                        }
                    }
                    // Not added, and not here: nothing the store can name, but it isn't imported.
                    None => summary.invalid_rows += 1,
                }
                return Ok(());
            }
        };
        summary.imported += 1;
        if let Some((creator, poll_id)) = &poll {
            summary.polls += 1;
            // Only the poll itself is the target of later votes and its closing.
            if kind == "poll" {
                self.store.register_poll(key, creator, *poll_id, row)?;
            }
        }
        if kind == "file" {
            // With the row, in its transaction: a stop or a kill before the media come must not
            // leave a file nothing can fetch any more.
            self.keep_file_keys(row, file.as_ref())?;
            summary.media_missing += usize::from(!has_media);
            if has_media || has_thumbnail {
                media_jobs.push(MediaJob {
                    row,
                    uid: message.uid.clone(),
                    outgoing: message.outgoing,
                });
            }
        }
        if kind != "deleted" {
            self.apply_thumbs(key, row, message, index, summary)?;
        }
        Ok(())
    }

    /// The blob key a backup row brings, if it brings one.
    fn keep_file_keys(
        &self,
        row: i64,
        file: Option<&convert::FileImport>,
    ) -> Result<(), StoreError> {
        let Some(file) = file else {
            return Ok(());
        };
        let Some(key) = &file.key else {
            return Ok(());
        };
        self.store.set_file_keys(
            row,
            &crate::media::FileKeys {
                key: key.clone(),
                blob_id: file.blob_id.map(libthreema::common::BlobId),
                thumbnail_blob_id: None,
            },
        )
    }

    /// Android's thumbs: 1:1 USERACK/USERDEC and the per-member `g_msg_states`, as reactions
    /// (`RestoreService.createReactionForStateName`).
    fn apply_thumbs(
        &self,
        _key: &str,
        row: i64,
        message: &records::Message,
        index: &ArchiveIndex,
        summary: &mut Summary,
    ) -> Result<(), StoreError> {
        let at = message.modified_at.unwrap_or(message.at);
        let emoji_for = |state: &str| match state {
            "USERACK" => Some("\u{1F44D}"),
            "USERDEC" => Some("\u{1F44E}"),
            _ => None,
        };
        if let Some(emoji) = emoji_for(&message.state) {
            // 1:1: on an outgoing message the partner reacted; on an incoming one we did.
            let sender = if message.outgoing {
                _key.to_owned()
            } else {
                String::new()
            };
            if crate::groups::parse_key(_key).is_none() {
                self.store.set_reaction(row, &sender, emoji, true, at)?;
            }
        }
        for (member, state) in &message.group_states {
            if let Some(emoji) = emoji_for(state) {
                if index.other_key.contains(member) {
                    summary.key_conflict_rows += 1;
                    continue;
                }
                let sender = if *member == self.me {
                    ""
                } else {
                    member.as_str()
                };
                self.store.set_reaction(row, sender, emoji, true, at)?;
            }
        }
        Ok(())
    }

    /// `contact_reactions.csv` and `group_reactions.csv` (format 26+).
    async fn import_reactions(
        &self,
        mut archive: Archive,
        index: &ArchiveIndex,
        summary: &mut Summary,
    ) -> Result<Archive, crate::app::ApiError> {
        for (entry, group) in [
            ("contact_reactions.csv", false),
            ("group_reactions.csv", true),
        ] {
            if !archive.has_entry(entry) {
                continue;
            }
            let name = entry.to_owned();
            let (returned, reactions, invalid) = tokio::task::spawn_blocking(
                move || -> Result<(Archive, Vec<records::Reaction>, usize), ArchiveError> {
                    let table = archive.read_table(&name, MAX_TABLE_BYTES)?;
                    let (reactions, invalid) = records::reactions(&table, group);
                    Ok((archive, reactions, invalid))
                },
            )
            .await
            .map_err(|e| crate::app::ApiError::Internal(e.to_string()))??;
            archive = returned;
            summary.invalid_rows += invalid;
            for chunk in reactions.chunks(BATCH) {
                if self.cancel.get() {
                    summary.cancelled = true;
                    return Ok(archive);
                }
                for reaction in chunk {
                    if index.other_key.contains(&reaction.sender)
                        || reaction
                            .contact
                            .as_ref()
                            .is_some_and(|contact| index.other_key.contains(contact))
                    {
                        summary.key_conflict_rows += 1;
                        continue;
                    }
                    let conversation = match (&reaction.contact, &reaction.group) {
                        (Some(contact), _) => contact.clone(),
                        (_, Some((creator, id))) => match creator.parse() {
                            Ok(creator_identity) => {
                                crate::groups::key(&libthreema::common::GroupIdentity {
                                    creator_identity,
                                    group_id: *id,
                                })
                            }
                            Err(_) => {
                                summary.reactions_unmatched += 1;
                                continue;
                            }
                        },
                        _ => continue,
                    };
                    let mut row = None;
                    for outgoing in [false, true] {
                        if let Some(message) = self.store.message_by_id(
                            &conversation,
                            outgoing,
                            reaction.api_message_id,
                        )? {
                            if message.kind != "deleted" {
                                row = Some(message.id);
                            }
                            break;
                        }
                    }
                    match row {
                        Some(row) => {
                            let sender = if reaction.sender == self.me {
                                ""
                            } else {
                                reaction.sender.as_str()
                            };
                            // No answer: not an emoji, so nothing was stored.
                            if self
                                .store
                                .set_reaction(
                                    row,
                                    sender,
                                    &reaction.emoji,
                                    true,
                                    reaction.reacted_at,
                                )?
                                .is_some()
                            {
                                summary.reactions += 1;
                            } else {
                                summary.reactions_unmatched += 1;
                            }
                        }
                        None => summary.reactions_unmatched += 1,
                    }
                }
                tokio::task::yield_now().await;
            }
        }
        Ok(archive)
    }
}

/// A table the format allows to be absent.
fn optional_table(
    archive: &mut Archive,
    name: &str,
) -> Result<Option<super::csv::Table>, ArchiveError> {
    if !archive.has_entry(name) {
        return Ok(None);
    }
    archive.read_table(name, MAX_TABLE_BYTES).map(Some)
}

/// `ballot*.csv`, if present, and how many of their rows were unreadable.
fn read_polls(archive: &mut Archive) -> Result<(ArchivePolls, usize), ArchiveError> {
    let Some(ballots) = optional_table(archive, "ballot.csv")? else {
        return Ok((ArchivePolls::default(), 0));
    };
    let choices = optional_table(archive, "ballot_choice.csv")?;
    let votes = optional_table(archive, "ballot_vote.csv")?;
    Ok(ArchivePolls::read(
        &ballots,
        choices.as_ref(),
        votes.as_ref(),
    ))
}

/// SHA-256 of the archive file, streamed: the stable fingerprint synthetic ids are keyed by.
fn fingerprint_file(path: &std::path::Path) -> Result<String, ArchiveError> {
    use sha2::Digest as _;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = sha2::Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(data_encoding::HEXLOWER.encode(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::{mostly_from_non_members, records};
    use crate::android_backup::csv::{Table, write_row};

    #[test]
    fn the_sender_check_stays_linear() {
        // As many members as rows, nobody matching: comparing each pair took many minutes.
        const SIZE: usize = 300_000;
        let members: Vec<String> = (0..SIZE).map(|i| format!("M{i:07}")).collect();
        let mut text = write_row(&["uid", "identity", "isoutbox", "type", "body", "posted_at"]);
        for i in 0..SIZE {
            let uid = i.to_string();
            let sender = format!("S{i:07}");
            text.push_str(&write_row(&[&uid, &sender, "0", "TEXT", "", "1"]));
        }
        let (messages, invalid) = records::messages(&Table::parse(&text).unwrap());
        assert_eq!((messages.len(), invalid), (SIZE, 0));
        let start = std::time::Instant::now();
        assert!(mostly_from_non_members(&members, "ABCD1234", &messages));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(20),
            "{:?}",
            start.elapsed()
        );
    }
}
