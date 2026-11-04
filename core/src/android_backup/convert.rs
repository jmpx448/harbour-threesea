// SPDX-License-Identifier: AGPL-3.0-only
//! Converting one backup message row into the core's canonical form (kind + body JSON),
//! following `RestoreService.setMessageContent`, `FileDataModelSerializer.kt`,
//! `LegacyMessageBodyTransformer.kt`, `LocationDataModel.kt` and `VoipStatusDataModel.kt`.
//! Media availability (`fileState`…) is filled in by the importer from what the archive
//! actually holds, never from Android's stale `isDownloaded` flag.

use serde_json::{Value, json};
use zeroize::{Zeroize as _, Zeroizing};

use super::records;

/// Blob key material of a file message, for the encrypted `files` table only.
pub struct FileImport {
    pub key: Option<Zeroizing<[u8; 32]>>,
    pub blob_id: Option<[u8; 16]>,
}

/// What one backup row becomes here.
pub enum Converted {
    Message {
        kind: &'static str,
        body: String,
        file: Option<FileImport>,
        /// Counted in the summary when the content is only an approximation (`CONTACT`
        /// becomes plain text, like Android's own restore).
        compat: Option<&'static str>,
    },
    /// Not imported; the category is reported, never silently dropped.
    Unsupported(&'static str),
}

fn text_message(message: &records::Message) -> Converted {
    // Android stores quote bodies without the "> quote #" prefix plus a separate column; our
    // store keeps the prefix inline. A body that already carries one is left alone, so a
    // quote is neither lost nor prefixed twice.
    let body = if message.body.starts_with("> quote #") {
        message.body.clone()
    } else {
        crate::messages::with_quote(message.quoted_api_id, &message.body)
    };
    Converted::Message {
        kind: "text",
        body,
        file: None,
        compat: None,
    }
}

/// `[latitude, longitude, accuracy, address, name]`, optional tails (`LocationDataModel`).
fn location(message: &records::Message) -> Converted {
    let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(&message.body) else {
        return Converted::Unsupported("location");
    };
    let number = |index: usize| parts.get(index).and_then(Value::as_f64);
    let text = |index: usize| {
        parts
            .get(index)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
    };
    let (Some(latitude), Some(longitude)) = (number(0), number(1)) else {
        return Converted::Unsupported("location");
    };
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return Converted::Unsupported("location");
    }
    let mut body = json!({ "latitude": latitude, "longitude": longitude });
    if let Some(accuracy) = number(2).filter(|accuracy| accuracy.is_finite() && *accuracy >= 0.0) {
        body["accuracy"] = accuracy.into();
    }
    if let Some(address) = text(3) {
        body["address"] = address.into();
    }
    if let Some(name) = text(4) {
        body["name"] = name.into();
    }
    Converted::Message {
        kind: "location",
        body: body.to_string(),
        file: None,
        compat: None,
    }
}

fn hex_bytes<const N: usize>(value: Option<&Value>) -> Option<[u8; N]> {
    data_encoding::HEXLOWER_PERMISSIVE
        .decode(value?.as_str()?.as_bytes())
        .ok()?
        .try_into()
        .ok()
}

/// A blob key in hex, leaving no copy of it behind.
fn hex_key(value: Option<&Value>) -> Option<Zeroizing<[u8; 32]>> {
    let decoded = Zeroizing::new(
        data_encoding::HEXLOWER_PERMISSIVE
            .decode(value?.as_str()?.as_bytes())
            .ok()?,
    );
    let mut key = Zeroizing::new([0; 32]);
    if decoded.len() != key.len() {
        return None;
    }
    key.copy_from_slice(&decoded);
    Some(key)
}

/// Wipe a parsed body's strings when it is done with: one of them is the blob's key.
fn wipe_strings(parts: &mut [Value]) {
    for part in parts {
        if let Value::String(text) = part {
            text.zeroize();
        }
    }
}

/// The FILE body array (`FileDataModelSerializer`):
/// `[blobId, encryptionKey, mimeType, fileSize, fileName, renderingType, isDownloaded,
///   caption, thumbnailMimeType, metadata]`, older bodies with missing tails.
fn file(message: &records::Message, parts: &[Value]) -> Converted {
    let text = |index: usize| {
        parts
            .get(index)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let media_type = text(2).unwrap_or_else(|| "application/octet-stream".into());
    let metadata = parts.get(9).cloned().unwrap_or_else(|| json!({}));
    let meta_number = |key: &str| {
        metadata[key]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    let caption = text(7).or_else(|| message.caption.clone());
    let rendering = match parts.get(5).and_then(Value::as_i64) {
        Some(2) => "sticker",
        Some(1) => "media",
        _ => "file",
    };
    let body = json!({
        "rendering": rendering,
        "mediaType": media_type,
        "name": text(4),
        "size": parts.get(3).and_then(Value::as_u64),
        "caption": caption,
        "width": meta_number("w"),
        "height": meta_number("h"),
        "duration": meta_number("d"),
        "animated": metadata["a"].as_bool().filter(|animated| *animated),
        "thumbnailMediaType": text(8).unwrap_or_else(|| "image/jpeg".into()),
        // Availability fields (fileState, thumbnail…, uploadState) are set by the importer
        // from the archive's actual entries.
    });
    Converted::Message {
        kind: "file",
        body: body.to_string(),
        file: Some(FileImport {
            key: hex_key(parts.get(1)),
            blob_id: hex_bytes(parts.first()),
        }),
        compat: None,
    }
}

/// Deprecated IMAGE/VIDEO/VOICEMESSAGE bodies become FILE arrays, exactly like
/// `LegacyMessageBodyTransformer` (the legacy blob keys are kept but their blobs used a
/// different scheme; availability comes from the archived bytes).
fn legacy_file(message: &records::Message, kind: &str) -> Converted {
    let Ok(Value::Array(mut old)) = serde_json::from_str::<Value>(&message.body) else {
        return Converted::Unsupported("media");
    };
    let array = match kind {
        // [isDownloaded, encryptionKey, blobId, nonce]
        "IMAGE" => json!([
            old.get(2),
            old.get(1),
            "image/jpeg",
            0,
            Value::Null,
            1,
            old.first(),
            Value::Null,
            Value::Null,
            {}
        ]),
        // [duration, isDownloaded, encryptionKey, blobId, (fileSize)]
        "VIDEO" => json!([
            old.get(3), old.get(2), "video/mpeg", old.get(4).cloned().unwrap_or(0.into()),
            Value::Null, 1, old.get(1), Value::Null, Value::Null, {"d": old.first()}
        ]),
        // [duration, isDownloaded, encryptionKey, blobId]
        _ => json!([
            old.get(3), old.get(2), "audio/aac", 0, Value::Null, 1, old.get(1),
            Value::Null, Value::Null, {"d": old.first()}
        ]),
    };
    wipe_strings(&mut old);
    let Value::Array(mut parts) = array else {
        return Converted::Unsupported("media");
    };
    let converted = file(message, &parts);
    wipe_strings(&mut parts);
    converted
}

/// `VOIP_STATUS` bodies are `[1, {status, duration, …}]` (`StatusDataModel`); ours is the
/// status code Android shows the same way: `call|result|outgoing|seconds`.
fn call_status(message: &records::Message) -> Converted {
    let Ok(Value::Array(parts)) = serde_json::from_str::<Value>(&message.body) else {
        return Converted::Unsupported("status");
    };
    let Some(params) = parts.get(1).and_then(Value::as_object) else {
        return Converted::Unsupported("status");
    };
    let result = match params.get("status").and_then(Value::as_i64) {
        Some(1) => "missed",
        Some(2) => "finished",
        Some(3) => "rejected",
        Some(4) => "aborted",
        _ => return Converted::Unsupported("status"),
    };
    let duration = params
        .get("duration")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    Converted::Message {
        kind: "status",
        body: format!("call|{result}|{}|{duration}", u8::from(message.outgoing)),
        file: None,
        compat: None,
    }
}

/// One backup row into our canonical form. A local deletion wins over everything: the row
/// stays as the tombstone it is.
pub fn convert(message: &records::Message) -> Converted {
    if message.deleted_at.is_some() {
        return Converted::Message {
            kind: "deleted",
            body: String::new(),
            file: None,
            compat: None,
        };
    }
    if message.kind == "VOIP_STATUS" {
        return call_status(message);
    }
    // System lines (group membership, old call settings, group-call banners): no faithful
    // mapping here yet; counted per category instead of silently dropped.
    if message.is_status {
        return Converted::Unsupported("status");
    }
    match message.kind.as_str() {
        "TEXT" => text_message(message),
        "LOCATION" => location(message),
        "FILE" => match serde_json::from_str::<Value>(&message.body) {
            Ok(Value::Array(mut parts)) => {
                let converted = file(message, &parts);
                wipe_strings(&mut parts);
                converted
            }
            _ => Converted::Unsupported("media"),
        },
        "IMAGE" | "VIDEO" | "VOICEMESSAGE" => legacy_file(message, &message.kind),
        // Like Android's own restore: contact shares become plain text, counted.
        "CONTACT" => Converted::Message {
            kind: "text",
            body: message.body.clone(),
            file: None,
            compat: Some("contactAsText"),
        },
        "BALLOT" => Converted::Unsupported("poll"),
        "STATUS" | "GROUP_STATUS" | "GROUP_CALL_STATUS" => Converted::Unsupported("status"),
        _ => Converted::Unsupported("other"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, body: &str) -> records::Message {
        records::Message {
            uid: "u1".into(),
            api_id: Some(7),
            outgoing: false,
            read: true,
            state: "READ".into(),
            sender: None,
            kind: kind.into(),
            body: body.into(),
            is_status: false,
            caption: None,
            quoted_api_id: None,
            at: 1000,
            modified_at: None,
            delivered_at: None,
            read_at: None,
            edited_at: None,
            deleted_at: None,
            starred: false,
            group_states: Vec::new(),
        }
    }

    #[test]
    fn quotes_are_not_doubled() {
        let mut plain = row("TEXT", "hello");
        plain.quoted_api_id = Some(0x0807_0605_0403_0201);
        let Converted::Message { body, .. } = convert(&plain) else {
            panic!()
        };
        assert_eq!(body, "> quote #0102030405060708\n\nhello");
        // A body that already carries the prefix stays as it is.
        let prefixed = row("TEXT", "> quote #0102030405060708\n\nhello");
        let Converted::Message { body, .. } = convert(&prefixed) else {
            panic!()
        };
        assert_eq!(body, "> quote #0102030405060708\n\nhello");
    }

    #[test]
    fn locations_validate() {
        let Converted::Message { kind, body, .. } =
            convert(&row("LOCATION", "[47.37,8.54,12.5,\"Street 1\",\"Home\"]"))
        else {
            panic!()
        };
        assert_eq!(kind, "location");
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["latitude"], 47.37);
        assert_eq!(value["address"], "Street 1");
        assert_eq!(value["name"], "Home");
        assert!(crate::messages::location_message(&body).is_some());
        // Short tail and bad values.
        assert!(matches!(
            convert(&row("LOCATION", "[47.37,8.54]")),
            Converted::Message { .. }
        ));
        assert!(matches!(
            convert(&row("LOCATION", "[91.0,8.54]")),
            Converted::Unsupported("location")
        ));
    }

    #[test]
    fn file_bodies_map() {
        let body = format!(
            "[\"{}\",\"{}\",\"image/jpeg\",4711,\"photo.jpg\",1,true,\"a caption\",null,{{\"w\":800,\"h\":600}}]",
            "11".repeat(16),
            "22".repeat(32)
        );
        let Converted::Message {
            kind, body, file, ..
        } = convert(&row("FILE", &body))
        else {
            panic!()
        };
        assert_eq!(kind, "file");
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["mediaType"], "image/jpeg");
        assert_eq!(value["rendering"], "media");
        assert_eq!(value["name"], "photo.jpg");
        assert_eq!(value["size"], 4711);
        assert_eq!(value["caption"], "a caption");
        assert_eq!(value["width"], 800.0);
        let file = file.unwrap();
        assert_eq!(file.blob_id, Some([0x11; 16]));
        assert_eq!(file.key.as_deref(), Some(&[0x22; 32]));
    }

    #[test]
    fn a_blob_key_of_the_wrong_length_is_none() {
        let body = format!("[\"{}\",\"{}\"]", "11".repeat(16), "22".repeat(31));
        let Converted::Message { file, .. } = convert(&row("FILE", &body)) else {
            panic!()
        };
        assert!(file.unwrap().key.is_none());
    }

    #[test]
    fn wiping_a_body_clears_its_strings() {
        let mut parts = vec![json!("secret"), json!(1), json!({"kept": true})];
        wipe_strings(&mut parts);
        assert_eq!(parts[0], "");
        assert_eq!(parts[1], 1);
    }

    #[test]
    fn legacy_media_transform() {
        // IMAGE: [isDownloaded, encryptionKey, blobId, nonce]
        let body = format!(
            "[true,\"{}\",\"{}\",\"00\"]",
            "22".repeat(32),
            "11".repeat(16)
        );
        let Converted::Message { body, file, .. } = convert(&row("IMAGE", &body)) else {
            panic!()
        };
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["mediaType"], "image/jpeg");
        assert_eq!(value["rendering"], "media");
        assert_eq!(file.unwrap().blob_id, Some([0x11; 16]));
        // VOICEMESSAGE: [duration, isDownloaded, encryptionKey, blobId]
        let body = format!("[42,true,\"{}\",\"{}\"]", "22".repeat(32), "11".repeat(16));
        let Converted::Message { body, .. } = convert(&row("VOICEMESSAGE", &body)) else {
            panic!()
        };
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["mediaType"], "audio/aac");
        assert_eq!(value["duration"], 42.0);
    }

    #[test]
    fn call_and_unsupported_rows() {
        let mut call = row("VOIP_STATUS", "[1,{\"status\":2,\"duration\":65}]");
        call.outgoing = true;
        let Converted::Message { kind, body, .. } = convert(&call) else {
            panic!()
        };
        assert_eq!((kind, body.as_str()), ("status", "call|finished|1|65"));
        assert!(matches!(
            convert(&row("BALLOT", "x")),
            Converted::Unsupported("poll")
        ));
        let mut status = row("TEXT", "whatever");
        status.is_status = true;
        assert!(matches!(convert(&status), Converted::Unsupported("status")));
        let mut deleted = row("TEXT", "gone");
        deleted.deleted_at = Some(5);
        assert!(matches!(
            convert(&deleted),
            Converted::Message {
                kind: "deleted",
                ..
            }
        ));
    }
}
