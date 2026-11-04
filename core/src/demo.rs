// SPDX-License-Identifier: AGPL-3.0-only
//! Demo data for the emulator build (feature `demo`, switched on for i486 only; see
//! `scripts/build-core.sh`).
//!
//! The emulator's install is linked to a real ID, and the only contact there that ever answers
//! is ECHOECHO, so a chat list to work against — unread chats, groups, media, a long history —
//! cannot be staged by hand. `demoData` writes one into the store and takes it away again.
//!
//! Everything here is made up and local: contacts `DEMO0001`… with a fixed dummy public key,
//! messages inserted straight into the store, pictures and voice files generated in memory.
//! Writing it sends nothing, fetches no blob and touches no key material, opening a demo chat
//! asks nobody for a picture (`Store::request_picture`), and no device build contains any of it.

use image::codecs::jpeg::JpegEncoder;
use libthreema::protobuf::{d2d_sync, d2d_sync::contact as pc};
use serde_json::{Value, json};

use crate::{
    android_backup::place::Placer,
    app::{ApiError, App},
    app_media::{STATE_FAILED, STATE_LOCAL},
    distribution_lists::DistributionList,
    groups::Group,
    media::{MediaError, MediaPart, MediaStore},
    messages::{INCOMING_READ, INCOMING_UNREAD, ImportedMessage, OutgoingState, now_ms},
    store::Store,
};

/// Media writes here only ever fail on a broken device; the UI just needs to hear about it.
fn wrote(result: Result<(), MediaError>) -> Result<(), ApiError> {
    result.map_err(|error| ApiError::Internal(error.to_string()))
}

/// Every demo contact's ID starts with this; the demo groups and lists are found through them.
const PREFIX: &str = "DEMO";

/// ID, first name, last name, verification level (0 unverified … 2 fully verified).
const CONTACTS: [(&str, &str, &str, i64); 9] = [
    ("DEMO0001", "Ada", "Lindqvist", 2),
    ("DEMO0002", "Bruno", "Weber", 1),
    ("DEMO0003", "Carla", "Mendes", 0),
    ("DEMO0004", "Dieter", "Kraus", 1),
    ("DEMO0005", "Elin", "Haugen", 2),
    ("DEMO0006", "Farid", "Nazari", 0),
    ("DEMO0007", "Greta", "Olsen", 1),
    ("DEMO0008", "Hugo", "Baumann", 0),
    // A stranger: no name, only a message from them (the spam report's banner).
    ("DEMO0009", "", "", 0),
];

const DAY: i64 = 86_400_000;
const HOUR: i64 = 3_600_000;
const MINUTE: i64 = 60_000;

/// One message to insert.
struct Msg {
    at: i64,
    outgoing: bool,
    kind: &'static str,
    body: String,
    sender: Option<String>,
    /// Incoming: [`INCOMING_UNREAD`] or [`INCOMING_READ`]; outgoing: an [`OutgoingState`].
    state: i64,
    starred: bool,
    edited: bool,
}

impl Msg {
    fn incoming(at: i64, kind: &'static str, body: impl Into<String>) -> Self {
        Self {
            at,
            outgoing: false,
            kind,
            body: body.into(),
            sender: None,
            state: INCOMING_READ,
            starred: false,
            edited: false,
        }
    }

    fn outgoing(at: i64, kind: &'static str, body: impl Into<String>) -> Self {
        Self {
            at,
            outgoing: true,
            kind,
            body: body.into(),
            sender: None,
            state: OutgoingState::Read as i64,
            starred: false,
            edited: false,
        }
    }

    fn text_in(at: i64, text: &str) -> Self {
        Self::incoming(at, "text", text)
    }

    fn text_out(at: i64, text: &str) -> Self {
        Self::outgoing(at, "text", text)
    }

    fn unread(mut self) -> Self {
        self.state = INCOMING_UNREAD;
        self
    }

    fn state(mut self, state: OutgoingState) -> Self {
        self.state = state as i64;
        self
    }

    fn from(mut self, sender: &str) -> Self {
        self.sender = Some(sender.to_owned());
        self
    }

    fn starred(mut self) -> Self {
        self.starred = true;
        self
    }

    fn edited(mut self) -> Self {
        self.edited = true;
        self
    }
}

/// A quote of `message_id` in front of `text`, as the chat stores it.
fn quote(message_id: u64, text: &str) -> String {
    format!(
        "> quote #{}\n\n{text}",
        data_encoding::HEXLOWER.encode(&message_id.to_le_bytes())
    )
}

/// A made-up picture: soft bands in a hue picked by `seed`, so every contact and image looks
/// different without shipping any artwork.
fn picture(seed: u32, width: u32, height: u32) -> Vec<u8> {
    let base = [
        (0x4a_u8, 0x6f, 0xa5),
        (0xa5, 0x5a, 0x4a),
        (0x4a, 0xa5, 0x7a),
        (0x8a, 0x4a, 0xa5),
        (0xa5, 0x9a, 0x4a),
        (0x4a, 0x9a, 0xa5),
    ][seed as usize % 6];
    let image = image::RgbImage::from_fn(width, height, |x, y| {
        let wave = (f64::from(x) / f64::from(width) * 3.0
            + f64::from(y) / f64::from(height) * 2.0
            + f64::from(seed))
        .sin()
            * 0.25
            + 0.75;
        let shade = |c: u8| ((f64::from(c) * wave).clamp(0.0, 255.0)) as u8;
        image::Rgb([shade(base.0), shade(base.1), shade(base.2)])
    });
    let mut out = Vec::new();
    let _ = JpegEncoder::new_with_quality(&mut out, 80).encode_image(&image);
    out
}

/// How many favourite GIFs the demo data adds, and their size: about what a phone's GIFs are.
const GIFS: u32 = 40;
const GIF_SIZE: (u16, u16) = (320, 240);
const GIF_FRAMES: u16 = 30;

/// A made-up animated GIF: moving bands in a hue picked by `seed`, different on every frame so a
/// player has real work to do. No GIF encoder among the dependencies, and none is needed: with
/// a clear code often enough, LZW may carry every pixel as a literal 8-bit code. With `hole`, the
/// top left quarter is transparent, so what is behind the picture has to show.
fn gif(seed: u32, hole: bool) -> Vec<u8> {
    const CLEAR: u8 = 128;
    const END: u8 = 129;
    let (width, height) = GIF_SIZE;
    let mut out = b"GIF89a".to_vec();
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    // A global table of 128 colours; a grey ramp tinted by the seed.
    out.extend_from_slice(&[0b1111_0110, 0, 0]);
    let tint = [(3_u32, 2_u32, 1_u32), (1, 3, 2), (2, 1, 3)][seed as usize % 3];
    for i in 0..128_u32 {
        out.extend(
            [tint.0, tint.1, tint.2]
                .iter()
                .map(|t| u8::try_from((i * 2 * (t + 1) / 4).min(255)).unwrap_or(255)),
        );
    }
    // Forever (Netscape's extension).
    out.extend_from_slice(b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00");
    for frame in 0..GIF_FRAMES {
        // 40 ms per frame; with a hole, colour 0 is transparent and each frame replaces the last.
        let flags = if hole { 0b0000_1001 } else { 0 };
        out.extend_from_slice(&[0x21, 0xf9, 4, flags, 4, 0, 0, 0, 0x2c, 0, 0, 0, 0]);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&[0, 7]);
        let mut codes = Vec::with_capacity(usize::from(width) * usize::from(height) * 101 / 100);
        for y in 0..u32::from(height) {
            for x in 0..u32::from(width) {
                // Before the code table could outgrow the 8 bits.
                if (y * u32::from(width) + x) % 100 == 0 {
                    codes.push(CLEAR);
                }
                let wave = x + y + u32::from(frame) * 8 + seed * 13;
                let inside_hole = hole && x < u32::from(width) / 2 && y < u32::from(height) / 2;
                codes.push(if inside_hole {
                    0
                } else {
                    // Never colour 0, which a hole would make transparent here too.
                    u8::try_from((wave ^ (x * y / 64)) % 127 + 1).unwrap_or(1)
                });
            }
        }
        codes.push(END);
        for block in codes.chunks(255) {
            out.push(u8::try_from(block.len()).unwrap_or(255));
            out.extend_from_slice(block);
        }
        out.push(0);
    }
    out.push(0x3b);
    out
}

/// A made-up voice message: `seconds` of a quiet two-tone chirp as Ogg Opus, so the player and
/// the waveform have something real to work with.
fn voice(seconds: f64) -> Option<(Vec<u8>, f64)> {
    let rate = 48_000.0;
    let samples = (rate * seconds) as usize;
    let mut pcm = Vec::with_capacity(samples * 2);
    for n in 0..samples {
        let t = n as f64 / rate;
        let tone = (t * 440.0 * std::f64::consts::TAU).sin() * 0.3
            + (t * 660.0 * std::f64::consts::TAU).sin() * 0.15;
        let fade = (t / 0.05).min(1.0).min((seconds - t) / 0.05).max(0.0);
        #[expect(clippy::cast_possible_truncation, reason = "clamped to i16")]
        let value = (tone * fade * 20_000.0).clamp(-32_000.0, 32_000.0) as i16;
        pcm.extend_from_slice(&value.to_le_bytes());
    }
    crate::voice::encode(&pcm).ok()
}

/// The body of a file message whose parts are on the device already.
fn file_body(
    rendering: &str,
    media_type: &str,
    name: &str,
    size: usize,
    thumbnail: bool,
    extra: Value,
) -> Value {
    let mut body = json!({
        "rendering": rendering,
        "mediaType": media_type,
        "name": name,
        "size": size,
        "thumbnail": thumbnail,
        "thumbnailMediaType": "image/jpeg",
        "fileState": STATE_LOCAL,
        "thumbnailState": if thumbnail { STATE_LOCAL } else { STATE_FAILED },
        "uploadState": STATE_LOCAL,
    });
    if let (Some(body), Some(extra)) = (body.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            body.insert(key.clone(), value.clone());
        }
    }
    body
}

/// Writes the demo chats; `messages` is the length of the long history chat.
struct Seeder<'a> {
    store: &'a Store,
    media: &'a MediaStore,
    own: String,
    now: i64,
    next_message_id: u64,
    messages: usize,
}

impl Seeder<'_> {
    fn message_id(&mut self) -> u64 {
        self.next_message_id += 1;
        // Nothing of ours ever goes out, but keep them apart from real ones all the same.
        0xDEA0_0000_0000_0000 | self.next_message_id
    }

    /// Insert a conversation's messages (oldest first); returns their row ids in order.
    fn chat(&mut self, conversation: &str, msgs: &[Msg]) -> Result<Vec<i64>, ApiError> {
        let mut placer = Placer::new(self.store.conversation_positions(conversation)?, msgs.len());
        let mut ids = Vec::with_capacity(msgs.len());
        for msg in msgs {
            let message_id = self.message_id();
            let read = !msg.outgoing && msg.state == INCOMING_READ;
            let imported = ImportedMessage {
                conversation,
                message_id,
                outgoing: msg.outgoing,
                kind: msg.kind,
                body: &msg.body,
                created_at: msg.at,
                updated_at: msg.at,
                state: msg.state,
                edited_at: msg.edited.then_some(msg.at + MINUTE),
                sender: msg.sender.as_deref(),
                starred: msg.starred,
                sent_at: msg.outgoing.then_some(msg.at),
                delivered_at: (msg.outgoing && msg.state >= OutgoingState::Delivered as i64)
                    .then_some(msg.at + MINUTE),
                read_at: if msg.outgoing {
                    (msg.state >= OutgoingState::Read as i64).then_some(msg.at + 2 * MINUTE)
                } else {
                    read.then_some(msg.at + MINUTE)
                },
                received_at: (!msg.outgoing).then_some(msg.at),
                sort_key: placer.place(msg.at),
            };
            if let Some(id) = self.store.insert_imported(&imported)? {
                ids.push(id);
            }
        }
        Ok(ids)
    }

    /// A picture message with its thumbnail, both on the device.
    fn picture_message(
        &mut self,
        conversation: &str,
        at: i64,
        outgoing: bool,
        seed: u32,
        caption: &str,
    ) -> Result<i64, ApiError> {
        let full = picture(seed, 1024, 768);
        let thumbnail = picture(seed, 320, 240);
        let body = file_body(
            "media",
            "image/jpeg",
            "picture.jpg",
            full.len(),
            true,
            json!({ "width": 1024, "height": 768, "caption": caption }),
        );
        let msg = if outgoing {
            Msg::outgoing(at, "file", body.to_string())
        } else {
            Msg::incoming(at, "file", body.to_string())
        };
        let id = *self
            .chat(conversation, &[msg])?
            .first()
            .ok_or_else(|| ApiError::Internal("demo picture not inserted".into()))?;
        wrote(self.media.write(id, MediaPart::File, &full))?;
        wrote(self.media.write(id, MediaPart::Thumbnail, &thumbnail))?;
        Ok(id)
    }

    fn contact_chats(&mut self) -> Result<(), ApiError> {
        let now = self.now;

        // A stranger's first message: the chat offers a spam report.
        self.chat(
            "DEMO0009",
            &[Msg::incoming(
                now - HOUR,
                "text",
                "You have won a prize. Reply to claim it.",
            )],
        )?;

        // Ada: everything read, pinned, a profile picture.
        self.chat(
            "DEMO0001",
            &[
                Msg::text_in(now - 3 * DAY, "Shall we take the ferry on Saturday?"),
                Msg::text_out(
                    now - 3 * DAY + 4 * MINUTE,
                    "Good idea. Which one goes at nine?",
                ),
                Msg::text_in(now - 3 * DAY + 9 * MINUTE, "The one from the north pier."),
                Msg::text_out(now - 2 * DAY, "Tickets are booked 🎟️").starred(),
                Msg::text_in(now - 2 * DAY + HOUR, "Perfect, see you there!"),
                // The chat list's preview, where a picture emoji once landed a line below where
                // it belonged: an emoji right after the text, a second one at the end of the next
                // line, more lines than the preview shows.
                Msg::text_in(
                    now - DAY,
                    "Good morning!🙋🏻\u{200d}♂️\nThe ferry leaves at nine.🛳️\nBring a warm \
                     jacket, it gets windy on deck.",
                ),
            ],
        )?;
        self.store.set_pinned("DEMO0001", true)?;

        // Bruno: one message left unread on purpose, older than Carla's.
        self.chat(
            "DEMO0002",
            &[
                Msg::text_out(now - 2 * DAY, "Did the parts arrive?"),
                Msg::text_in(now - 3 * HOUR, "They did — can you call me later?").unread(),
            ],
        )?;

        // Carla: the newest unread chat (three of them).
        self.chat(
            "DEMO0003",
            &[
                Msg::text_in(now - 4 * DAY, "Hi! Found your number through a friend."),
                Msg::text_out(now - 4 * DAY + 20 * MINUTE, "Hello Carla, welcome!"),
                Msg::text_in(now - HOUR, "Are you coming on Friday?").unread(),
                Msg::text_in(now - HOUR + MINUTE, "We start at seven.").unread(),
                Msg::text_in(now - 50 * MINUTE, "Bring something to drink 🍷").unread(),
            ],
        )?;

        // Dieter: archived.
        self.chat(
            "DEMO0004",
            &[
                Msg::text_in(now - 30 * DAY, "The invoice is in the post."),
                Msg::text_out(now - 30 * DAY + HOUR, "Thanks, got it."),
            ],
        )?;
        self.store.set_archived("DEMO0004", true)?;

        // Greta: notifications off.
        self.chat(
            "DEMO0007",
            &[Msg::text_in(
                now - 6 * DAY,
                "Forwarded you the whole thread, sorry!",
            )],
        )?;
        self.store.set_chat_notifications(
            "DEMO0007",
            Some((crate::chat_notifications::Policy::Never, None)),
        )?;

        // Hugo: a message that failed to go out (the chat list's and the bubble's state).
        // Never a *pending* one: the connection sends everything outgoing in state 0 after
        // every connect, and nobody is behind a demo ID.
        self.chat(
            "DEMO0008",
            &[
                Msg::text_in(now - 5 * HOUR, "Are we still on for tomorrow?"),
                Msg::text_out(now - 4 * HOUR, "Yes — leaving at eight.")
                    .state(OutgoingState::Delivered),
                Msg::text_out(now - 3 * HOUR, "Take the small boat.").state(OutgoingState::Failed),
            ],
        )?;
        Ok(())
    }

    /// Elin: a long history for scrolling, paging and the quick scroll.
    fn long_history(&mut self) -> Result<(), ApiError> {
        let lines = [
            "Did you see the sunrise this morning?",
            "Almost. I was still asleep.",
            "The harbour was completely still.",
            "Send a picture next time!",
            "It is supposed to rain all weekend.",
            "Then we stay in and cook.",
            "I found that recipe again.",
            "The one with the fennel?",
            "That one. It takes two hours.",
            "Worth it though.",
        ];
        let count = self.messages;
        let start = self.now - 90 * DAY;
        let step = (90 * DAY) / i64::try_from(count.max(1)).unwrap_or(1);
        let mut msgs = Vec::with_capacity(count);
        for n in 0..count {
            let at = start + step * i64::try_from(n).unwrap_or(0);
            let text = lines[n % lines.len()];
            msgs.push(if n % 2 == 0 {
                Msg::text_in(at, text)
            } else {
                Msg::text_out(at, text)
            });
        }
        self.chat("DEMO0005", &msgs)?;
        Ok(())
    }

    /// Farid: one of every kind of content.
    fn media_chat(&mut self) -> Result<(), ApiError> {
        let now = self.now;
        let chat = "DEMO0006";
        let first = self.chat(
            chat,
            &[
                Msg::text_in(now - 2 * DAY, "Here is everything from the trip."),
                Msg::text_out(now - 2 * DAY + MINUTE, "Can't wait to see it!"),
            ],
        )?;

        self.picture_message(
            chat,
            now - 2 * DAY + 2 * MINUTE,
            false,
            1,
            "The old lighthouse",
        )?;
        self.picture_message(chat, now - 2 * DAY + 3 * MINUTE, true, 2, "")?;

        // A GIF that plays in its bubble, with a transparent part.
        let animation = gif(1, true);
        let (width, height) = GIF_SIZE;
        let body = file_body(
            "media",
            "image/gif",
            "animation.gif",
            animation.len(),
            true,
            json!({ "width": width, "height": height, "animated": true }),
        );
        if let Some(&id) = self
            .chat(
                chat,
                &[Msg::incoming(
                    now - 2 * DAY + 4 * MINUTE,
                    "file",
                    body.to_string(),
                )],
            )?
            .first()
        {
            wrote(self.media.write(id, MediaPart::File, &animation))?;
            wrote(
                self.media
                    .write(id, MediaPart::Thumbnail, &picture(1, 320, 240)),
            )?;
        }

        // A voice message, played back for real.
        if let Some((ogg, seconds)) = voice(4.0) {
            let body = file_body(
                "media",
                "audio/ogg",
                "voice.ogg",
                ogg.len(),
                false,
                json!({ "duration": seconds }),
            );
            if let Some(&id) = self
                .chat(chat, &[Msg::incoming(now - DAY, "file", body.to_string())])?
                .first()
            {
                wrote(self.media.write(id, MediaPart::File, &ogg))?;
            }
        }

        // A document, and a video whose download failed (the retry state).
        let text = b"Packing list\n\n- boots\n- map\n- thermos\n";
        let body = file_body(
            "file",
            "text/plain",
            "packing-list.txt",
            text.len(),
            false,
            json!({}),
        );
        if let Some(&id) = self
            .chat(
                chat,
                &[Msg::incoming(now - DAY + HOUR, "file", body.to_string())],
            )?
            .first()
        {
            wrote(self.media.write(id, MediaPart::File, text))?;
        }
        let thumbnail = picture(3, 320, 180);
        let mut video = file_body(
            "media",
            "video/mp4",
            "harbour.mp4",
            8_400_000,
            true,
            json!({ "width": 1280, "height": 720, "duration": 12.0 }),
        );
        video["fileState"] = json!(STATE_FAILED);
        if let Some(&id) = self
            .chat(
                chat,
                &[Msg::incoming(
                    now - DAY + 2 * HOUR,
                    "file",
                    video.to_string(),
                )],
            )?
            .first()
        {
            wrote(self.media.write(id, MediaPart::Thumbnail, &thumbnail))?;
        }

        // Location, a quote of it, an edited message and one deleted for everyone.
        let location = json!({
            "lat": 59.9139, "lon": 10.7522, "accuracy": 12.0,
            "name": "North pier", "address": "Havnegata 1",
        });
        let located = self.chat(
            chat,
            &[Msg::incoming(
                now - 5 * HOUR,
                "location",
                location.to_string(),
            )],
        )?;
        let quoted = match located.first() {
            Some(&id) => self.store.message(id)?.map(|m| m.message_id),
            None => None,
        };
        if let Some(quoted) = quoted {
            self.chat(
                chat,
                &[Msg::text_out(
                    now - 4 * HOUR,
                    &quote(quoted, "That's where we met the others."),
                )],
            )?;
        }
        let ids = self.chat(
            chat,
            &[
                Msg::text_in(now - 3 * HOUR, "Lunch at one, not at noon.").edited(),
                Msg::incoming(now - 2 * HOUR, "deleted", ""),
                Msg::text_in(now - 90 * MINUTE, "Anyway — thanks for the weekend! 🙏").unread(),
            ],
        )?;
        // Reactions, ours and theirs.
        if let Some(&id) = ids.first() {
            self.store
                .set_reaction(id, "DEMO0006", "👍", true, now - 2 * HOUR)?;
            self.store.set_reaction(id, "", "🙂", true, now - HOUR)?;
        }
        if let Some(&id) = first.first() {
            self.store.set_reaction(id, "", "❤️", true, now - DAY)?;
        }
        Ok(())
    }

    /// A group with a picture and a mention of us, and the notes group (only ourselves).
    fn groups(&mut self) -> Result<(), ApiError> {
        let now = self.now;
        let group = Group {
            key: format!("g:{}:{:016x}", CONTACTS[0].0, 0xDEA0_0000_0000_0001_u64),
            identity: libthreema::common::GroupIdentity {
                creator_identity: CONTACTS[0].0.parse().map_err(|_| {
                    ApiError::Internal("demo group creator is not a Threema ID".into())
                })?,
                group_id: 0xDEA0_0000_0000_0001,
            },
            name: "Hiking trip".into(),
            members: vec![
                CONTACTS[0].0.to_owned(),
                CONTACTS[6].0.to_owned(),
                CONTACTS[7].0.to_owned(),
            ],
            left: false,
            created_at: now - 20 * DAY,
            picture: None,
            picture_version: 1,
        };
        self.store.upsert_group(&group)?;
        wrote(self.media.write_picture(&group.key, &picture(4, 512, 512)))?;
        let key = group.key.clone();
        let mention = format!("Who is driving? @[{}]", self.own);
        self.chat(
            &key,
            &[
                Msg::incoming(now - 20 * DAY, "status", "group|created").from(CONTACTS[0].0),
                Msg::text_in(now - 2 * DAY, "Route is fixed: the ridge, then the hut.")
                    .from(CONTACTS[0].0),
                Msg::text_in(now - 2 * DAY + 5 * MINUTE, "I can bring the big tent.")
                    .from(CONTACTS[6].0),
                Msg::text_out(now - 2 * DAY + 11 * MINUTE, "I'll take care of breakfast."),
                Msg::text_in(now - 6 * HOUR, &mention)
                    .from(CONTACTS[7].0)
                    .unread(),
                Msg::text_in(now - 5 * HOUR, "Weather looks good 🌤️")
                    .from(CONTACTS[0].0)
                    .unread(),
            ],
        )?;
        self.picture_message(&key, now - 4 * HOUR, false, 5, "Last year's view")?;

        // The notes group: a group of one, which Threema uses as a private notebook.
        let own = self.own.clone();
        let notes = Group {
            key: format!("g:{own}:{:016x}", 0xDEA0_0000_0000_0002_u64),
            identity: libthreema::common::GroupIdentity {
                creator_identity: own
                    .parse()
                    .map_err(|_| ApiError::Internal("own ID is not a Threema ID".into()))?,
                group_id: 0xDEA0_0000_0000_0002,
            },
            name: "Notes".into(),
            members: Vec::new(),
            left: false,
            created_at: now - 40 * DAY,
            picture: None,
            picture_version: 0,
        };
        self.store.upsert_group(&notes)?;
        self.chat(
            &notes.key,
            &[
                Msg::text_out(now - 40 * DAY, "Ferry times: 09:10, 13:40, 18:05"),
                Msg::text_out(now - 7 * DAY, "Spare key is with the neighbour."),
            ],
        )?;
        Ok(())
    }

    fn distribution_list(&mut self) -> Result<(), ApiError> {
        let list = DistributionList {
            key: format!("d:{:016x}", 0xDEA0_0000_0000_0003_u64),
            name: "Boat crew".into(),
            members: vec![
                CONTACTS[0].0.to_owned(),
                CONTACTS[4].0.to_owned(),
                CONTACTS[6].0.to_owned(),
            ],
            created_at: self.now - 10 * DAY,
        };
        self.store.upsert_distribution_list(&list)?;
        let key = list.key.clone();
        self.chat(
            &key,
            &[
                Msg::text_out(self.now - 10 * DAY, "Departure moved to Saturday 08:00."),
                Msg::text_out(self.now - 2 * DAY, "Life jackets are in the shed."),
            ],
        )?;
        Ok(())
    }
}

impl App {
    /// `demoData`: fill the store with made-up chats, or (`remove`) take them away again.
    pub(crate) fn demo_data(
        &mut self,
        remove: bool,
        messages: Option<u32>,
    ) -> Result<Value, ApiError> {
        if remove {
            return self.remove_demo_data();
        }
        let own = self
            .store
            .identity()?
            .map(|identity| identity.threema_id)
            .ok_or(ApiError::NoIdentity)?;
        for (id, first, last, verification) in CONTACTS {
            self.store
                .upsert_contact(&crate::contacts::test_contact(id))?;
            if let Some(mut contact) = self.store.contact(id)? {
                contact.first_name = Some(first.to_owned()).filter(|name| !name.is_empty());
                contact.last_name = Some(last.to_owned()).filter(|name| !name.is_empty());
                contact.nickname = contact
                    .first_name
                    .as_ref()
                    .map(|name| name.to_ascii_lowercase());
                contact.verification_level = match verification {
                    2 => pc::VerificationLevel::FullyVerified,
                    1 => pc::VerificationLevel::ServerVerified,
                    _ => pc::VerificationLevel::Unverified,
                };
                // As a real sender becomes with their first message in a 1:1 chat.
                contact.acquaintance_level = pc::AcquaintanceLevel::Direct;
                // Nothing is behind these IDs, so nothing should ever be sent to them by
                // itself: opening a demo chat must not send a read receipt, writing in one
                // must not send a typing indicator.
                contact.read_receipt_policy_override =
                    Some(d2d_sync::ReadReceiptPolicy::DontSendReadReceipt);
                contact.typing_indicator_policy_override =
                    Some(d2d_sync::TypingIndicatorPolicy::DontSendTypingIndicator);
                self.store.upsert_contact(&contact)?;
            }
        }
        // Two of them have a profile picture.
        for (index, id) in [CONTACTS[0].0, CONTACTS[2].0].iter().enumerate() {
            wrote(self.media.write_picture(
                &crate::profile_pictures::contact_picture_name(id),
                &picture(u32::try_from(index).unwrap_or(0), 512, 512),
            ))?;
            self.store.set_contact_picture(id, Some(1))?;
        }

        let mut seeder = Seeder {
            store: &self.store,
            media: &self.media,
            own,
            now: now_ms(),
            next_message_id: 0,
            messages: messages.unwrap_or(400) as usize,
        };
        seeder.contact_chats()?;
        seeder.long_history()?;
        seeder.media_chat()?;
        seeder.groups()?;
        seeder.distribution_list()?;
        let messages = seeder.next_message_id;
        for seed in 0..GIFS {
            self.demo_gif_favourite(seed)?;
        }

        tracing::info!(messages, "demo data written");
        self.demo_changed();
        Ok(json!({ "contacts": CONTACTS.len(), "messages": messages }))
    }

    /// Keeps demo GIF `seed` as a favourite (or finds it, the same GIF is kept once): its id.
    fn demo_gif_favourite(&mut self, seed: u32) -> Result<i64, ApiError> {
        self.attachments = vec![
            zeroize::Zeroizing::new(gif(seed, false)),
            zeroize::Zeroizing::new(picture(seed, 320, 240)),
        ];
        let (width, height) = GIF_SIZE;
        let added =
            self.add_gif_favourite(None, None, Some(u32::from(width)), Some(u32::from(height)))?;
        added["id"]
            .as_i64()
            .ok_or_else(|| ApiError::Internal("no favourite".into()))
    }

    /// Every demo chat and contact, and the media that went with them.
    fn remove_demo_data(&mut self) -> Result<Value, ApiError> {
        for seed in 0..GIFS {
            let id = self.demo_gif_favourite(seed)?;
            self.remove_gif_favourite(id)?;
        }
        let mut chats: Vec<String> = CONTACTS.iter().map(|c| c.0.to_owned()).collect();
        for group in self.store.groups()? {
            if group.key.contains(PREFIX) || group.name == "Notes" && group.members.is_empty() {
                chats.push(group.key.clone());
                self.media.remove_picture(&group.key);
                self.store.delete_group(&group.key)?;
            }
        }
        for list in self.store.distribution_lists()? {
            if list.members.iter().any(|m| m.starts_with(PREFIX)) {
                chats.push(list.key.clone());
                self.store.delete_distribution_list(&list.key)?;
            }
        }
        for chat in &chats {
            self.store.delete_conversation(chat)?;
        }
        for (id, ..) in CONTACTS {
            self.media
                .remove_picture(&crate::profile_pictures::contact_picture_name(id));
            self.store.set_contact_picture(id, None)?;
            self.store.delete_contact(id)?;
        }
        let store = std::rc::Rc::clone(&self.store);
        self.media
            .remove_orphans(|id| store.message_exists(id).unwrap_or(true));
        let chats = chats.len();
        tracing::info!(chats, "demo data removed");
        self.demo_changed();
        Ok(json!({ "chats": chats }))
    }

    /// Tell the UI to read everything again (the chat list listens for the groups).
    fn demo_changed(&self) {
        let _ = self
            .media_events
            .send(crate::providers::CoreEvent::ContactsChanged);
        let _ = self
            .media_events
            .send(crate::providers::CoreEvent::GroupsChanged);
    }
}
