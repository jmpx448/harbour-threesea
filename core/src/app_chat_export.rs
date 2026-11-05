// SPDX-License-Identifier: AGPL-3.0-only
//! Exporting one chat into a ZIP file, like Android's "Export chat"
//! (`ExportConversationServiceImpl`): `messages.txt` with one line per message, and the media
//! that are on the device when they are asked for. Optionally encrypted with a password, the
//! same AES-256 the data backup uses, so the archive can be opened with any ZIP tool.
//!
//! Everything stays on the device: the file goes where the UI says (Downloads) and nothing is
//! sent anywhere. Texts the file shows the user — "Me" and the header — come from the UI, like
//! everywhere else in the core.

use std::io::Write as _;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::{
    app::{ApiError, App},
    app_messages::Conversation,
    contacts,
    media::MediaPart,
    messages::{Message, media_kind, split_quote},
};

/// Messages read from the store at a time (the whole chat is never in memory at once).
const PAGE: u32 = 500;

type Writer = zip::ZipWriter<std::fs::File>;

/// `YYYY-MM-DD HH:MM`, `offset` minutes east of UTC (the device's, as the UI knows it).
fn stamp(at_ms: i64, offset_minutes: i64) -> String {
    // The time comes from the sender, so it can be anything: never overflow on it.
    let local = at_ms.saturating_add(offset_minutes.saturating_mul(60_000));
    let stamp = crate::android_backup::export::time_stamp(local);
    let (date, time) = stamp.split_at(8);
    format!(
        "{}-{}-{} {}:{}",
        &date[..4],
        &date[4..6],
        &date[6..],
        &time[1..3],
        &time[3..5]
    )
}

/// `+02:00` / `-05:30`, for the file's header.
fn offset_text(offset_minutes: i64) -> String {
    let sign = if offset_minutes < 0 { '-' } else { '+' };
    let minutes = offset_minutes.abs();
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

impl App {
    /// `exportChat`: write the chat as a ZIP into `directory`; returns its path and what went
    /// in. `me` and `title_line` are the UI's texts (translated there).
    pub(crate) async fn export_chat(
        &mut self,
        identity: &str,
        media: bool,
        directory: &str,
        password: Option<Zeroizing<String>>,
        me: Option<String>,
        offset_minutes: Option<i64>,
    ) -> Result<Value, ApiError> {
        let (key, conversation) = self.readable_conversation(identity)?;
        self.check_readable(&key)?;
        // Same rule as the data backup; Android asks the same of this archive.
        if password
            .as_ref()
            .is_some_and(|password| !password.is_empty() && password.chars().count() < 8)
        {
            return Err(ApiError::PasswordTooShort);
        }
        let me = me.unwrap_or_else(|| "Me".to_owned());
        let offset = offset_minutes.unwrap_or(0);
        let title = match &conversation {
            Conversation::Contact(contact) => contacts::display_name(contact),
            Conversation::Group(group) => {
                let names = |identity: &str| self.display_name(identity);
                crate::app_groups::title(&group.name, &group.members, &names)
            }
            Conversation::List(list) => list.name.clone(),
        };

        let now = crate::messages::now_ms();
        let name = format!(
            "threesea-chat-{}.zip",
            crate::android_backup::export::time_stamp(now)
        );
        let path = std::path::Path::new(directory).join(&name);
        let temporary = path.with_extension("zip.part");
        let create = temporary.clone();
        let folder = std::path::PathBuf::from(directory);
        let mut writer = tokio::task::spawn_blocking(move || -> Result<Writer, ApiError> {
            remove_leftovers(&folder);
            let file =
                std::fs::File::create(&create).map_err(|e| ApiError::Internal(e.to_string()))?;
            Ok(zip::ZipWriter::new(file))
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))??;

        let result = async {
            let mut text = format!(
                "{title}\nThreesea, {} (UTC{})\n\n",
                stamp(now, offset),
                offset_text(offset)
            );
            let mut messages = 0_usize;
            let mut files = 0_usize;
            // Pages newest first, each one in order: read backwards, write forwards, without
            // ever holding a second copy of a long chat.
            let history = self.chat_history(&key)?;
            for message in history.iter().rev().flatten() {
                // Status lines are the app talking to itself, as on Android.
                if matches!(message.kind.as_str(), "status" | "deleted")
                    || message.kind.starts_with("call")
                {
                    continue;
                }
                // Who wrote it, as Android writes it: "Me" for ours, the name for theirs (in a
                // group the sender's, in a 1:1 chat the chat's).
                let mut line = if message.outgoing {
                    format!("{me}: ")
                } else {
                    format!(
                        "{}: ",
                        self.display_name(message.sender.as_deref().unwrap_or(&key))
                    )
                };
                let body: Value = serde_json::from_str(&message.body).unwrap_or(Value::Null);
                match message.kind.as_str() {
                    "text" => line.push_str(split_quote(&message.body).1),
                    "location" => {
                        let (lat, lon) = (body["latitude"].as_f64(), body["longitude"].as_f64());
                        for field in ["name", "address"] {
                            if let Some(value) = body[field].as_str().filter(|v| !v.is_empty()) {
                                line.push_str(value);
                                line.push(' ');
                            }
                        }
                        if let (Some(lat), Some(lon)) = (lat, lon) {
                            line.push_str(&format!("<geo:{lat},{lon}>"));
                        }
                    }
                    "poll" | crate::polls::KIND_CLOSED => {
                        if let Some(question) = body["description"].as_str() {
                            line.push_str(question);
                        }
                    }
                    "file" => {
                        if let Some(caption) = body["caption"].as_str().filter(|c| !c.is_empty()) {
                            line.push_str(caption);
                            line.push(' ');
                        }
                        let name = crate::app_media::safe_file_name(
                            body["name"].as_str(),
                            body["mediaType"].as_str().unwrap_or_default(),
                        );
                        // The file itself, else its preview — whatever is on the device. A
                        // preview is a JPEG and says so: it is not the video it belongs to.
                        let (part, in_file) = if self.media.has(message.id, MediaPart::File) {
                            (
                                Some(MediaPart::File),
                                format!("{:016x}-{name}", message.message_id),
                            )
                        } else if self.media.has(message.id, MediaPart::Thumbnail) {
                            let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
                            (
                                Some(MediaPart::Thumbnail),
                                format!("{:016x}-{stem}-preview.jpg", message.message_id),
                            )
                        } else {
                            (None, String::new())
                        };
                        match part.filter(|_| media) {
                            Some(part) => {
                                let data = self
                                    .media
                                    .read(message.id, part)
                                    .map_err(|e| ApiError::Internal(e.to_string()))?;
                                if let Some(data) = data {
                                    writer = add(writer, in_file.clone(), data, &password).await?;
                                    files += 1;
                                    line.push_str(&format!("<{in_file}>"));
                                }
                            }
                            // Not included, but say what it was.
                            None => line.push_str(&format!("<{}>", media_kind(&message.body))),
                        }
                    }
                    _ => continue,
                }
                text.push_str(&format!("[{}] {line}\n", stamp(message.created_at, offset)));
                messages += 1;
            }
            let writer = add(
                writer,
                "messages.txt".to_owned(),
                Zeroizing::new(std::mem::take(&mut text).into_bytes()),
                &password,
            )
            .await?;
            Ok::<_, ApiError>((writer, messages, files))
        }
        .await;

        let (writer, messages, files) = match result {
            Ok(done) => done,
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(error);
            }
        };
        let finish = tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
            let file = writer
                .finish()
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            file.sync_all()
                .map_err(|e| ApiError::Internal(e.to_string()))
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        if let Err(error) = finish {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
        std::fs::rename(&temporary, &path).map_err(|e| ApiError::Internal(e.to_string()))?;
        tracing::info!(messages, files, "chat exported");
        Ok(json!({
            "path": path.to_string_lossy(),
            "name": name,
            "messages": messages,
            "files": files,
        }))
    }

    /// A chat's messages in pages, newest page first, each page oldest first.
    fn chat_history(&self, key: &str) -> Result<Vec<Vec<Message>>, ApiError> {
        let mut pages = Vec::new();
        let mut page = self.store.conversation_messages(key, PAGE)?;
        while !page.is_empty() {
            let oldest = page[0].id;
            pages.push(page);
            page = self.store.conversation_messages_before(key, oldest, PAGE)?;
        }
        Ok(pages)
    }

    /// A contact's name as the chat shows it, or the bare ID.
    fn display_name(&self, identity: &str) -> String {
        self.store
            .contact(identity)
            .ok()
            .flatten()
            .as_ref()
            .map_or_else(|| identity.to_owned(), contacts::display_name)
    }
}

/// What an export left behind when the app was killed during it, with the chat's messages inside:
/// the user doesn't know about the file and nothing else ever takes it away. Only one export runs
/// at a time (`export_chat` holds the whole `App`), so none of these is in use.
fn remove_leftovers(directory: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("threesea-chat-") && name.ends_with(".zip.part") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// One entry into the archive, on the blocking pool (the writer and the data round-trip, so a
/// 100 MB video is never held twice).
async fn add(
    mut writer: Writer,
    name: String,
    data: Zeroizing<Vec<u8>>,
    password: &Option<Zeroizing<String>>,
) -> Result<Writer, ApiError> {
    let password = password.clone();
    tokio::task::spawn_blocking(move || -> Result<Writer, ApiError> {
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let options = match &password {
            Some(password) if !password.is_empty() => {
                options.with_aes_encryption(zip::AesMode::Aes256, password)
            }
            _ => options,
        };
        writer
            .start_file(name, options)
            .and_then(|()| writer.write_all(&data).map_err(zip::result::ZipError::Io))
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        Ok(writer)
    })
    .await
    .map_err(|e| ApiError::Internal(e.to_string()))?
}
