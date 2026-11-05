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
    app::{ApiError, Shared},
    app_messages::Conversation,
    contacts,
    media::MediaPart,
    messages::{Message, split_quote},
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

impl Shared {
    /// `exportChat`: write the chat as a ZIP into `directory`; returns its path and what went
    /// in. `me` and `title_line` are the UI's texts (translated there).
    pub(crate) async fn export_chat(
        &self,
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
        let folder = std::path::PathBuf::from(directory);
        crate::media::off_thread(move || remove_leftovers(&folder))
            .await
            .map_err(|e| ApiError::Internal(e.to_string()))?;
        // Here and not on the blocking thread: dropped before that has run, it would still come.
        let temporary = PartFile::new(path.with_extension("zip.part"));
        let file =
            std::fs::File::create(&temporary.0).map_err(|e| ApiError::Internal(e.to_string()))?;
        let mut writer: Writer = zip::ZipWriter::new(file);

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
            let history = self.chat_history(&key).await?;
            let mut read = 0;
            for message in history.iter().rev().flatten() {
                // Locked meanwhile: a private chat is out of bounds again, also for this.
                self.check_readable(&key)?;
                // A chat without files has nothing else to wait for.
                read += 1;
                if read % PAGE == 0 {
                    tokio::task::yield_now().await;
                }
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
                                let files_here = std::sync::Arc::clone(&self.media);
                                let id = message.id;
                                let data =
                                    crate::media::off_thread(move || files_here.read(id, part))
                                        .await
                                        .and_then(|read| read)
                                        .map_err(|e| ApiError::Internal(e.to_string()))?;
                                if let Some(data) = data {
                                    writer = add(writer, in_file.clone(), data, &password).await?;
                                    files += 1;
                                    line.push_str(&format!("<{in_file}>"));
                                }
                            }
                            // Not included, but say what it was: by its name, like Android.
                            None => line.push_str(&format!("<{name}>")),
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

        let (writer, messages, files) = result?;
        let finish = tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
            let file = writer
                .finish()
                .map_err(|e| ApiError::Internal(e.to_string()))?;
            file.sync_all()
                .map_err(|e| ApiError::Internal(e.to_string()))
        })
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
        finish?;
        // Locked during the last steps: no private chat gets out in one piece.
        self.check_readable(&key)?;
        std::fs::rename(&temporary.0, &path).map_err(|e| ApiError::Internal(e.to_string()))?;
        temporary.moved();
        tracing::info!(messages, files, "chat exported");
        Ok(json!({
            "path": path.to_string_lossy(),
            "name": name,
            "messages": messages,
            "files": files,
        }))
    }

    /// A chat's messages in pages, newest page first, each page oldest first.
    async fn chat_history(&self, key: &str) -> Result<Vec<Vec<Message>>, ApiError> {
        let mut pages = Vec::new();
        let mut page = self.store.conversation_messages(key, PAGE)?;
        while !page.is_empty() {
            let oldest = page[0].id;
            pages.push(page);
            // The store is on this thread; a long chat lets the core loop in now and then.
            tokio::task::yield_now().await;
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

/// The archive while it is being written, with the chat's messages inside: gone with this however
/// the export ends, a dropped task (the ID is being removed) as well.
struct PartFile(std::path::PathBuf);

impl PartFile {
    const fn new(path: std::path::PathBuf) -> Self {
        Self(path)
    }

    /// It is not there any more by that name.
    fn moved(self) {
        std::mem::forget(self);
    }
}

impl Drop for PartFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// What an export left behind when the app was killed during it, with the chat's messages inside:
/// the user doesn't know about the file and nothing else ever takes it away. Only one export runs
/// at a time (`Lane::InTurn`), so none of these is in use.
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

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::Pin, task::Poll};

    use super::*;
    use crate::app::{App, StartConfig};

    fn app(dir: &std::path::Path) -> App {
        let app = App::new(
            &StartConfig {
                data_dir: dir.to_owned(),
                app_version: "0.1.0".into(),
                language: "de".into(),
                country: "CH".into(),
                model: "test".into(),
                os_version: "5".into(),
                pictures_dir: None,
                calls: false,
                log_level: None,
                update_check: false,
            },
            &[3; 32],
        )
        .unwrap();
        app.store
            .upsert_contact(&contacts::test_contact("ABCD1234"))
            .unwrap();
        // A page of messages makes the export yield once its file is there. With nothing to read,
        // one poll can run it to the end if each blocking step is done before it is looked at.
        app.store
            .insert_incoming("ABCD1234", 1, "text", "hello", 1)
            .unwrap();
        app
    }

    fn part_files(directory: &std::path::Path) -> usize {
        std::fs::read_dir(directory)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".zip.part"))
            .count()
    }

    /// Runs the export until its temporary file is there. It stands still when not polled, so
    /// this leaves it there with certainty.
    async fn until_part_file<T>(
        export: &mut Pin<Box<impl Future<Output = T>>>,
        directory: &std::path::Path,
    ) {
        while part_files(directory) == 0 {
            let polled =
                std::future::poll_fn(|cx| Poll::Ready(export.as_mut().poll(cx).is_ready())).await;
            assert!(!polled, "it ended before there was a file");
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    }

    /// Removing the ID drops an export under way: no chat stays behind in the Downloads.
    #[tokio::test(flavor = "current_thread")]
    async fn a_dropped_export_leaves_no_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let directory = out.path().to_str().unwrap().to_owned();
        let mut export = Box::pin(app.export_chat("ABCD1234", false, &directory, None, None, None));
        until_part_file(&mut export, out.path()).await;
        drop(export);
        assert_eq!(part_files(out.path()), 0);
    }

    /// Media left out are named by their file, not by a word of ours in English.
    #[tokio::test(flavor = "current_thread")]
    async fn media_left_out_are_named_by_their_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let body = serde_json::json!({ "name": "holiday.jpg", "mediaType": "image/jpeg" });
        app.store
            .insert_incoming("ABCD1234", 2, "file", &body.to_string(), 2)
            .unwrap();
        let directory = out.path().to_str().unwrap().to_owned();
        let result = app
            .export_chat("ABCD1234", false, &directory, None, None, None)
            .await
            .unwrap();
        let file = std::fs::File::open(result["path"].as_str().unwrap()).unwrap();
        let mut zip = zip::ZipArchive::new(file).unwrap();
        let text = std::io::read_to_string(zip.by_name("messages.txt").unwrap()).unwrap();
        assert!(text.contains("<holiday.jpg>"), "{text}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_export_says_where_it_went_and_leaves_only_that() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        let directory = out.path().to_str().unwrap().to_owned();
        let result = app
            .export_chat("ABCD1234", false, &directory, None, None, None)
            .await
            .unwrap();
        assert_eq!(part_files(out.path()), 0);
        assert!(std::path::Path::new(result["path"].as_str().unwrap()).exists());
    }

    /// Locked as the last steps run: no archive of a private chat, and nothing "saved".
    #[tokio::test(flavor = "current_thread")]
    async fn a_lock_during_the_last_steps_leaves_no_archive() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let app = app(dir.path());
        app.unlocked.set(true);
        app.mark_private("ABCD1234", true).unwrap();
        let directory = out.path().to_str().unwrap().to_owned();
        let mut export = Box::pin(app.export_chat("ABCD1234", false, &directory, None, None, None));
        until_part_file(&mut export, out.path()).await;
        app.unlocked.set(false);
        assert!(export.as_mut().await.is_err());
        drop(export);
        assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
    }
}
