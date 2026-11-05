// SPDX-License-Identifier: AGPL-3.0-only
//! Threema Web, pictures and files: profile pictures (`request/avatar`), message thumbnails
//! (`request/thumbnail`, and the small preview inside each message), a message's file
//! (`request/blob`; downloaded from the blob server first if it isn't on the device), and files
//! the browser sends (`create/fileMessage`, through our `sendFile`).

use rmpv::Value;
use serde_json::json;

use crate::{
    app::App,
    app_web_data::receiver_type,
    media::{self, MediaPart},
    messages::Message,
    profile_pictures::{OWN_PICTURE, contact_picture_name},
    providers::CoreEvent,
    web::{
        arp::{self, Incoming, map},
        images,
        saltyrtc::field,
    },
};

type Code = &'static str;

/// A `request/blob` waiting for the file's download.
pub(crate) struct PendingBlob {
    pub session: i64,
    pub message: i64,
    /// The request's acknowledgement ID.
    pub id: Option<String>,
}

fn receiver_args(key: &str) -> Vec<(&'static str, Value)> {
    vec![("type", receiver_type(key).into()), ("id", key.into())]
}

/// How many messages' thumbnail fields are kept: what a chat list and a page of messages show.
const THUMBNAILS_KEPT: usize = 128;

/// The `thumbnail` fields made lately, so that what is sent again and again needn't be made again.
/// Only while a browser is connected and the lock doesn't change: see `web_refresh`. Only for chats
/// that aren't private; every way to a browser leaves those out long before this.
#[derive(Default)]
pub(crate) struct WebThumbnails {
    /// By row and message id: the row of a removed message can be used again. `None`: there is a
    /// thumbnail, and it can't be read.
    fields: std::collections::BTreeMap<(i64, u64), Option<Value>>,
    /// How many were made.
    #[cfg(test)]
    pub(crate) made: u32,
}

impl WebThumbnails {
    fn get(&self, message: &Message) -> Option<Option<Value>> {
        self.fields.get(&(message.id, message.message_id)).cloned()
    }

    fn insert(&mut self, message: &Message, field: Option<Value>) {
        // The oldest first: the rows of chats that have moved on.
        if self.fields.len() >= THUMBNAILS_KEPT {
            self.fields.pop_first();
        }
        self.fields.insert((message.id, message.message_id), field);
        #[cfg(test)]
        {
            self.made += 1;
        }
    }

    pub(crate) fn clear(&mut self) {
        self.fields.clear();
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// A message's time for a file name: `20261105-143000`, in UTC (the core knows no time zone).
fn file_name_time(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let (days, time) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Days since 1970-01-01 to a date (Howard Hinnant's civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        time / 3600,
        time / 60 % 60,
        time % 60
    )
}

/// The name a browser gets for a message's file: its own, safe to save, with an extension from the
/// media type if it has none; without a name (the protocol doesn't ask for one) one from the
/// message's time and ID, like Android.
pub(crate) fn web_file_name(message: &Message, body: &serde_json::Value) -> String {
    let media_type = body["mediaType"].as_str().unwrap_or_default();
    let generated;
    let name = match body["name"].as_str().filter(|name| !name.trim().is_empty()) {
        Some(name) => name,
        None => {
            generated = format!(
                "threesea-{}-{:016x}",
                file_name_time(message.created_at),
                message.message_id
            );
            &generated
        }
    };
    crate::app_media::safe_file_name(Some(name), media_type)
}

/// The browser has one general text for a file that wasn't sent. This reason takes more, and it
/// has no way to translate what we tell it, as with the status lines. It goes after the reply: the
/// dialog the reply brings closes any that is open.
fn not_sent_alert(what: &str) -> Value {
    arp::message(
        "update",
        "alert",
        Some(map(vec![
            ("type", "warning".into()),
            ("source", "device".into()),
        ])),
        Some(map(vec![(
            "message",
            format!(
                "This {what} wasn't sent, because its metadata, which can hold where and when it \
                 was taken, can't be removed. Send it as a file to send it as it is."
            )
            .into(),
        )])),
    )
}

impl App {
    /// A message's file part, if on the device.
    fn web_media(&self, message: i64, part: MediaPart) -> Option<Vec<u8>> {
        self.media
            .read(message, part)
            .ok()
            .flatten()
            .map(|data| data.to_vec())
    }

    /// The `thumbnail` field of a file message (size within 350 px, 50 px preview), if its
    /// thumbnail is on the device. A message's thumbnail doesn't change, and it takes decoding,
    /// scaling and encoding a picture on this thread: once while it is wanted.
    pub(crate) fn web_thumbnail_field(&self, message: &Message) -> Option<Value> {
        let cached = self.web.thumbnails.borrow().get(message);
        if let Some(field) = cached {
            return field;
        }
        let thumbnail = self.web_media(message.id, MediaPart::Thumbnail)?;
        let field = images::thumbnail_info(&thumbnail).map(|(width, height, preview)| {
            map(vec![
                ("width", width.into()),
                ("height", height.into()),
                ("preview", Value::Binary(preview)),
            ])
        });
        self.web
            .thumbnails
            .borrow_mut()
            .insert(message, field.clone());
        field
    }

    /// Pictures and files; `None` if `incoming` isn't about them, `Some(Ok(None))` if the reply
    /// comes later (a file being downloaded).
    pub(crate) async fn web_media_action(
        &mut self,
        session: i64,
        incoming: &Incoming<'_>,
    ) -> Option<Result<Option<Value>, Code>> {
        Some(match (incoming.kind, incoming.sub_type) {
            ("request", "avatar") => self.web_avatar(incoming).map(Some),
            ("request", "thumbnail") => self.web_thumbnail(incoming).map(Some),
            ("request", "blob") => self.web_blob(session, incoming),
            ("create", "fileMessage") => self.web_send_file(incoming).await.map(Some),
            _ => return None,
        })
    }

    /// Our own picture in the size Threema Web's profile shows, if we have one.
    pub(crate) fn web_own_picture(&self) -> Option<Vec<u8>> {
        let picture = self.media.read_picture(OWN_PICTURE).ok().flatten()?;
        images::scaled_jpeg(&picture, images::AVATAR_HIGH_MAX, 75)
    }

    /// The file of a receiver's picture, for what the browser may see of it: not a private
    /// contact's, and contacts' only while the user wants them shown.
    pub(crate) fn web_picture_name(key: &str, shown: bool) -> Option<String> {
        match receiver_type(key) {
            "contact" if shown => Some(contact_picture_name(key)),
            "group" => Some(key.to_owned()),
            _ => None,
        }
    }

    pub(crate) fn web_avatar(&self, incoming: &Incoming<'_>) -> Result<Value, Code> {
        // We are no contact, and can't be private.
        let own = self
            .user()
            .ok()
            .filter(|user| incoming.arg("id") == Some(user.as_str()));
        let key = match &own {
            Some(user) => user.clone(),
            None => self
                .web_receiver_key(incoming.arg("id"))
                .map_err(|_| "invalidIdentity")?,
        };
        let high = incoming
            .args
            .and_then(|args| field(args, "highResolution"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let max = if high {
            images::AVATAR_HIGH_MAX
        } else {
            incoming
                .args
                .and_then(|args| field(args, "maxSize"))
                .and_then(Value::as_u64)
                .and_then(|size| u32::try_from(size).ok())
                .filter(|size| (16..=images::AVATAR_HIGH_MAX).contains(size))
                .unwrap_or(images::AVATAR_LOW_MAX)
        };
        let name = match &own {
            Some(_) => Some(OWN_PICTURE.to_owned()),
            None => Self::web_picture_name(&key, self.store.pictures_shown().unwrap_or(true)),
        };
        let picture = name
            .and_then(|name| self.media.read_picture(&name).ok().flatten())
            .and_then(|picture| images::scaled_jpeg(&picture, max, if high { 75 } else { 90 }));
        let mut args = match (&own, incoming.arg("type")) {
            // Threema Web's type for us is its own.
            (Some(_), Some(kind)) => vec![("type", kind.into()), ("id", key.as_str().into())],
            _ => receiver_args(&key),
        };
        args.push(("highResolution", high.into()));
        Ok(arp::message(
            "response",
            "avatar",
            Some(map(args)),
            picture.map(Value::Binary),
        ))
    }

    /// The chat and the message (local ID) a request is about.
    fn web_message_target(&self, incoming: &Incoming<'_>) -> Result<(String, i64), Code> {
        let key = self
            .web_receiver_key(incoming.arg("id"))
            .map_err(|_| "invalidIdentity")?;
        let id = incoming
            .arg("messageId")
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|id| {
                self.store
                    .message(*id)
                    .ok()
                    .flatten()
                    .is_some_and(|message| message.conversation == key && message.kind == "file")
            })
            .ok_or("invalidMessage")?;
        Ok((key, id))
    }

    fn web_message_args(key: &str, id: i64) -> Value {
        let mut args = receiver_args(key);
        args.push(("messageId", id.to_string().into()));
        map(args)
    }

    fn web_thumbnail(&self, incoming: &Incoming<'_>) -> Result<Value, Code> {
        let (key, id) = self.web_message_target(incoming)?;
        let thumbnail = self
            .web_media(id, MediaPart::Thumbnail)
            .and_then(|data| images::scaled_jpeg(&data, images::THUMBNAIL_MAX, 75))
            .ok_or("invalidMessage")?;
        Ok(arp::message(
            "response",
            "thumbnail",
            Some(Self::web_message_args(&key, id)),
            Some(Value::Binary(thumbnail)),
        ))
    }

    /// `response/blob` now, or `None` after starting the download (answered then).
    fn web_blob(&mut self, session: i64, incoming: &Incoming<'_>) -> Result<Option<Value>, Code> {
        let (key, id) = self.web_message_target(incoming)?;
        if let Some(reply) = self.web_blob_reply(&key, id) {
            return Ok(Some(reply));
        }
        self.download_file(id).map_err(|_| "blobDownloadFailed")?;
        self.web.pending_blobs.push(PendingBlob {
            session,
            message: id,
            id: incoming.id.map(str::to_owned),
        });
        Ok(None)
    }

    pub(crate) fn web_blob_reply(&self, key: &str, id: i64) -> Option<Value> {
        let data = self.web_media(id, MediaPart::File)?;
        let message = self.store.message(id).ok().flatten()?;
        let body: serde_json::Value = serde_json::from_str(&message.body).unwrap_or_default();
        Some(arp::message(
            "response",
            "blob",
            Some(Self::web_message_args(key, id)),
            Some(map(vec![
                ("blob", Value::Binary(data)),
                (
                    "type",
                    body["mediaType"]
                        .as_str()
                        .unwrap_or("application/octet-stream")
                        .into(),
                ),
                ("name", web_file_name(&message, &body).into()),
            ])),
        ))
    }

    /// Downloads finished or failed: answer the browsers waiting for them.
    pub(crate) fn web_blobs_changed(&mut self) {
        let pending = std::mem::take(&mut self.web.pending_blobs);
        for blob in pending {
            let state = self
                .store
                .message(blob.message)
                .ok()
                .flatten()
                .and_then(|message| serde_json::from_str::<serde_json::Value>(&message.body).ok())
                .and_then(|body| body["fileState"].as_i64());
            let Some(message) = self.store.message(blob.message).ok().flatten() else {
                continue;
            };
            let key = message.conversation;
            // Asked for before the chat became private: not delivered any more (audit
            // 2026-10-27, S03; unknown counts as private).
            let private = self.store.is_private(&key).unwrap_or(true);
            let reply = match state {
                _ if private => Err("blobDownloadFailed"),
                Some(crate::app_media::STATE_LOCAL) => self
                    .web_blob_reply(&key, blob.message)
                    .ok_or("blobDecryptFailed"),
                Some(crate::app_media::STATE_FAILED) => Err("blobDownloadFailed"),
                _ => {
                    self.web.pending_blobs.push(blob);
                    continue;
                }
            };
            let reply = match (reply, blob.id.as_deref()) {
                (Ok(reply), Some(id)) => arp::with_ack(reply, id, None),
                (Ok(reply), None) => reply,
                (Err(code), Some(id)) => arp::with_ack(
                    arp::message("update", "confirm", None, None),
                    id,
                    Some(code),
                ),
                (Err(_), None) => continue,
            };
            self.send_web(blob.session, reply);
        }
    }

    /// `create/fileMessage`: send the browser's file like one picked on the phone: unless sent as
    /// a file, what Android sends as media goes as media. JPEG and PNG are made anew with our
    /// thumbnail; the core can't make one of the others, which the protocol leaves optional. All
    /// leave without their metadata (ADR 0025) but a GIF: there is nobody to ask here, so what
    /// can't be cleaned isn't sent, and an alert says why.
    async fn web_send_file(&mut self, incoming: &Incoming<'_>) -> Result<Value, Code> {
        let key = self
            .web_receiver_key(incoming.arg("id"))
            .map_err(|_| "invalidIdentity")?;
        let data = incoming.data.ok_or("badRequest")?;
        let text = |name: &str| field(data, name).and_then(Value::as_str);
        let bytes = field(data, "data")
            .and_then(Value::as_slice)
            .ok_or("badRequest")?;
        if bytes.len() > 100 * 1024 * 1024 {
            return Err("fileTooLarge");
        }
        let media_type = text("fileType")
            .unwrap_or("application/octet-stream")
            .to_ascii_lowercase();
        let as_file = field(data, "sendAsFile")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut attachments = vec![zeroize::Zeroizing::new(bytes.to_vec())];
        let mut media_type = media_type;
        let mut media = !as_file && media_type.starts_with("video/");
        let mut size = None;
        if !as_file && media_type.starts_with("image/") {
            let mut picture = attachments.remove(0);
            let claimed = media_type.clone();
            let max = Some(self.image_size).filter(|side| *side > 0);
            let memory = media::reserve_memory_within(
                images::decoding_memory(&picture),
                media::COMMAND_MEMORY_WAIT,
            )
            .await;
            let (picture, prepared) = media::off_thread(move || {
                let _memory = memory;
                let prepared = images::sent_picture(&mut picture, &claimed, max);
                (picture, prepared)
            })
            .await
            .map_err(|_| "internalError")?;
            attachments.push(picture);
            match prepared {
                Ok(Some(prepared)) => {
                    media = true;
                    media_type = prepared.media_type.to_owned();
                    size = prepared.size;
                    attachments.extend(prepared.thumbnail.map(zeroize::Zeroizing::new));
                }
                Ok(None) => {}
                Err(images::PictureKeepsMetadata) => {
                    self.web.alert_after_reply = Some(not_sent_alert("picture"));
                    return Err("internalError");
                }
            }
        }
        let command = json!({
            "id": 0,
            "cmd": "sendFile",
            "identity": key,
            "name": text("name").unwrap_or("file"),
            "mediaType": media_type,
            "caption": text("caption"),
            "rendering": if media { "media" } else { "file" },
            "width": size.map(|(width, _)| width),
            "height": size.map(|(_, height)| height),
        });
        let reply: serde_json::Value =
            serde_json::from_str(&self.handle_with(&command.to_string(), attachments).await)
                .unwrap_or_default();
        if reply["ok"] != true {
            return Err(match reply["error"]["kind"].as_str() {
                Some("fileTooLarge") => "fileTooLarge",
                Some("contactBlocked") => "blocked",
                Some("contactRevoked") => "invalidIdentity",
                Some("messageTooLong") => "valueTooLong",
                Some("videoKeepsMetadata") => {
                    self.web.alert_after_reply = Some(not_sent_alert("video"));
                    "internalError"
                }
                _ => "internalError",
            });
        }
        let id = reply["result"]["id"].as_i64().unwrap_or(0);
        if let Ok(Some(message)) = self.store.message(id) {
            let _ = self.media_events.send(CoreEvent::MessageAdded { message });
        }
        Ok(arp::message(
            "create",
            "fileMessage",
            Some(map(receiver_args(&key))),
            Some(map(vec![("messageId", id.to_string().into())])),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::file_name_time;

    #[test]
    fn a_files_time_reads_as_a_date() {
        assert_eq!(file_name_time(0), "19700101-000000");
        assert_eq!(file_name_time(951_786_061_999), "20000229-010101");
        assert_eq!(file_name_time(1_767_225_599_000), "20251231-235959");
    }
}
