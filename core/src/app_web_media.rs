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
    media::MediaPart,
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
    /// thumbnail is on the device.
    pub(crate) fn web_thumbnail_field(&self, message: i64) -> Option<Value> {
        let thumbnail = self.web_media(message, MediaPart::Thumbnail)?;
        let (width, height, preview) = images::thumbnail_info(&thumbnail)?;
        Some(map(vec![
            ("width", width.into()),
            ("height", height.into()),
            ("preview", Value::Binary(preview)),
        ]))
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

    fn web_avatar(&self, incoming: &Incoming<'_>) -> Result<Value, Code> {
        let key = self
            .web_receiver_key(incoming.arg("id"))
            .map_err(|_| "invalidIdentity")?;
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
        let name = match receiver_type(&key) {
            "contact" => Some(crate::profile_pictures::contact_picture_name(&key)),
            "group" => Some(key.clone()),
            _ => None,
        };
        let picture = name
            .and_then(|name| self.media.read_picture(&name).ok().flatten())
            .and_then(|picture| images::scaled_jpeg(&picture, max, if high { 75 } else { 90 }));
        let mut args = receiver_args(&key);
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

    fn web_blob_reply(&self, key: &str, id: i64) -> Option<Value> {
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
                ("name", body["name"].as_str().unwrap_or("file").into()),
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

    /// `create/fileMessage`: send the browser's file like one picked on the phone (images as
    /// media with our thumbnail, unless sent as a file).
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
        let image = if as_file || !media_type.starts_with("image/") {
            None
        } else {
            images::sent_image(bytes)
        };
        let mut attachments = vec![zeroize::Zeroizing::new(bytes.to_vec())];
        if let Some((_, _, thumbnail)) = &image {
            attachments.push(zeroize::Zeroizing::new(thumbnail.clone()));
        }
        let command = json!({
            "id": 0,
            "cmd": "sendFile",
            "identity": key,
            "name": text("name").unwrap_or("file"),
            "mediaType": media_type,
            "caption": text("caption"),
            "rendering": if image.is_some() { "media" } else { "file" },
            "width": image.as_ref().map(|(w, ..)| *w),
            "height": image.as_ref().map(|(_, h, _)| *h),
        });
        let reply: serde_json::Value =
            serde_json::from_str(&self.handle_with(&command.to_string(), attachments).await)
                .unwrap_or_default();
        if reply["ok"] != true {
            return Err(match reply["error"]["kind"].as_str() {
                Some("fileTooLarge") => "fileTooLarge",
                Some("contactBlocked") => "blocked",
                Some("contactRevoked") => "invalidIdentity",
                _ => "internalError",
            });
        }
        let id = reply["result"]["id"].as_i64().unwrap_or(0);
        if let Ok(Some(message)) = self.store.message(id) {
            let _ = self.media_events.send(CoreEvent::MessageAdded {
                message,
                send_delivery_receipt: false,
            });
        }
        Ok(arp::message(
            "create",
            "fileMessage",
            Some(map(receiver_args(&key))),
            Some(map(vec![("messageId", id.to_string().into())])),
        ))
    }
}
