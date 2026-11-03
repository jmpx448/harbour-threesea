// SPDX-License-Identifier: AGPL-3.0-only
//! Favourite GIFs: animated images the user keeps to send again (from a chat, or from the
//! device). The GIF and its thumbnail are encrypted in the media store like pictures
//! (`gif:<id>`, `gif-thumb:<id>`); the table only knows sizes and a content hash, so the same
//! GIF is kept once.

use data_encoding::HEXLOWER;
use rusqlite::{OptionalExtension as _, params};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::{
    app::{ApiError, App},
    app_media::OutgoingFile,
    media::MediaPart,
    store::{Store, StoreError},
};

/// Schema (store migration 21).
pub(crate) const MIGRATION: &str = "
    CREATE TABLE gif_favourites (
        id        INTEGER PRIMARY KEY,
        hash      TEXT NOT NULL UNIQUE,
        width     INTEGER,
        height    INTEGER,
        size      INTEGER NOT NULL,
        added_at  INTEGER NOT NULL
    );";

/// One favourite, newest first in [`Store::gif_favourites`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifFavourite {
    pub id: i64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Media store names of a favourite's parts.
#[must_use]
pub fn picture_name(id: i64, part: MediaPart) -> String {
    match part {
        MediaPart::File => format!("gif:{id}"),
        MediaPart::Thumbnail => format!("gif-thumb:{id}"),
    }
}

impl Store {
    pub fn gif_favourites(&self) -> Result<Vec<GifFavourite>, StoreError> {
        let mut statement = self.conn.prepare(
            "SELECT id, width, height FROM gif_favourites ORDER BY added_at DESC, id DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(GifFavourite {
                id: row.get(0)?,
                width: row.get(1)?,
                height: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn gif_favourite(&self, id: i64) -> Result<Option<GifFavourite>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, width, height FROM gif_favourites WHERE id = ?1",
                [id],
                |row| {
                    Ok(GifFavourite {
                        id: row.get(0)?,
                        width: row.get(1)?,
                        height: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    /// Add a favourite with this content hash; `(id, new)`: an existing one moves to the front.
    pub fn add_gif_favourite(
        &self,
        hash: &str,
        width: Option<u32>,
        height: Option<u32>,
        size: usize,
    ) -> Result<(i64, bool), StoreError> {
        let now = crate::messages::now_ms();
        if let Some(id) = self
            .conn
            .query_row(
                "SELECT id FROM gif_favourites WHERE hash = ?1",
                [hash],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
        {
            self.conn.execute(
                "UPDATE gif_favourites SET added_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            return Ok((id, false));
        }
        self.conn.execute(
            "INSERT INTO gif_favourites (hash, width, height, size, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                hash,
                width,
                height,
                i64::try_from(size).unwrap_or(i64::MAX),
                now
            ],
        )?;
        Ok((self.conn.last_insert_rowid(), true))
    }

    pub fn remove_gif_favourite(&self, id: i64) -> Result<(), StoreError> {
        self.conn
            .execute("DELETE FROM gif_favourites WHERE id = ?1", [id])?;
        Ok(())
    }
}

impl App {
    /// `gifFavourites`: `{favourites: [{id, width, height}]}`, newest first.
    pub(crate) fn gif_favourites_json(&self) -> Result<Value, ApiError> {
        let favourites: Vec<Value> = self
            .store
            .gif_favourites()?
            .into_iter()
            .map(|f| json!({ "id": f.id, "width": f.width, "height": f.height }))
            .collect();
        Ok(json!({ "favourites": favourites }))
    }

    /// `addGifFavourite`: from a GIF message (`message`), or from the device (the attached GIF
    /// and thumbnail, or `path`, with `width`/`height`). Returns `{id}` and the list.
    pub(crate) fn add_gif_favourite(
        &mut self,
        message: Option<i64>,
        path: Option<&str>,
        mut width: Option<u32>,
        mut height: Option<u32>,
    ) -> Result<Value, ApiError> {
        let mut attachments = std::mem::take(&mut self.attachments).into_iter();
        let mut data = attachments.next().unwrap_or_default();
        let mut thumbnail = attachments.next().filter(|t| !t.is_empty());
        if let Some(message) = message {
            let stored = self.store.message(message)?;
            if let Some(stored) = &stored {
                self.check_readable(&stored.conversation)?;
            }
            let stored = stored
                .filter(|m| m.kind == "file")
                .ok_or_else(|| ApiError::Internal("no such file".into()))?;
            let body: Value = serde_json::from_str(&stored.body).unwrap_or_default();
            if !body["mediaType"]
                .as_str()
                .is_some_and(|t| t.eq_ignore_ascii_case("image/gif"))
            {
                return Err(ApiError::NotGif);
            }
            let dimension = |field: &str| body[field].as_u64().and_then(|v| u32::try_from(v).ok());
            width = dimension("width");
            height = dimension("height");
            data = self
                .media
                .read(message, MediaPart::File)
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .ok_or(ApiError::FileNotDownloaded)?;
            thumbnail = self
                .media
                .read(message, MediaPart::Thumbnail)
                .map_err(|e| ApiError::Internal(e.to_string()))?;
        } else if data.is_empty() {
            let path = path.ok_or_else(|| ApiError::Internal("no file".into()))?;
            let size = std::fs::metadata(path)
                .map_err(|e| ApiError::Internal(e.to_string()))?
                .len();
            if size > crate::media::MAX_BLOB_SIZE as u64 {
                return Err(ApiError::FileTooLarge);
            }
            data = zeroize::Zeroizing::new(
                std::fs::read(path).map_err(|e| ApiError::Internal(e.to_string()))?,
            );
        }
        if !data.starts_with(b"GIF8") {
            return Err(ApiError::NotGif);
        }
        let hash = HEXLOWER.encode(&Sha256::digest(data.as_slice()));
        let (id, new) = self
            .store
            .add_gif_favourite(&hash, width, height, data.len())?;
        if new {
            let written = (|| {
                self.media
                    .write_picture(&picture_name(id, MediaPart::File), &data)?;
                if let Some(thumbnail) = &thumbnail {
                    self.media
                        .write_picture(&picture_name(id, MediaPart::Thumbnail), thumbnail)?;
                }
                Ok::<_, crate::media::MediaError>(())
            })();
            if let Err(error) = written {
                self.remove_gif_files(id);
                self.store.remove_gif_favourite(id)?;
                return Err(ApiError::Internal(error.to_string()));
            }
        }
        let mut result = self.gif_favourites_json()?;
        result["id"] = json!(id);
        Ok(result)
    }

    /// `removeGifFavourite`: returns the list.
    pub(crate) fn remove_gif_favourite(&mut self, id: i64) -> Result<Value, ApiError> {
        self.store.remove_gif_favourite(id)?;
        self.remove_gif_files(id);
        self.gif_favourites_json()
    }

    fn remove_gif_files(&self, id: i64) {
        for part in [MediaPart::File, MediaPart::Thumbnail] {
            self.media.remove_picture(&picture_name(id, part));
        }
    }

    /// `sendGifFavourite`: send it to `identity` like a GIF from the device.
    pub(crate) fn send_gif_favourite(
        &mut self,
        identity: String,
        id: i64,
    ) -> Result<Value, ApiError> {
        let favourite = self
            .store
            .gif_favourite(id)?
            .ok_or_else(|| ApiError::Internal("no such favourite".into()))?;
        let read = |part| {
            self.media
                .read_picture(&picture_name(id, part))
                .map_err(|e| ApiError::Internal(e.to_string()))
        };
        let data =
            read(MediaPart::File)?.ok_or_else(|| ApiError::Internal("favourite missing".into()))?;
        let thumbnail = read(MediaPart::Thumbnail)?.unwrap_or_default();
        self.attachments = vec![data, thumbnail];
        let result = self.send_file(OutgoingFile {
            identity,
            path: None,
            name: Some("favourite.gif".into()),
            media_type: "image/gif".into(),
            caption: None,
            rendering: "media".into(),
            width: favourite.width,
            height: favourite.height,
            duration: None,
            encode: None,
        });
        self.attachments.clear();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn favourites_are_kept_once_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("db"), &[7; 32]).unwrap();
        let (a, new_a) = store
            .add_gif_favourite("aa", Some(10), Some(20), 5)
            .unwrap();
        let (b, new_b) = store.add_gif_favourite("bb", None, None, 6).unwrap();
        assert!(new_a && new_b);
        assert_eq!(
            store
                .gif_favourites()
                .unwrap()
                .iter()
                .map(|f| f.id)
                .collect::<Vec<_>>(),
            [b, a]
        );
        // The same GIF again: no copy, back to the front.
        std::thread::sleep(std::time::Duration::from_millis(2));
        assert_eq!(
            store.add_gif_favourite("aa", None, None, 5).unwrap(),
            (a, false)
        );
        assert_eq!(store.gif_favourites().unwrap()[0].id, a);
        assert_eq!(store.gif_favourite(a).unwrap().unwrap().width, Some(10));
        store.remove_gif_favourite(a).unwrap();
        assert_eq!(store.gif_favourites().unwrap().len(), 1);
        assert_eq!(store.gif_favourite(a).unwrap(), None);
    }
}
