//! Trait and implementations to encode/decode CSP E2E messages.
use core::str::{FromStr as _, Split, Utf8Error};

use data_encoding::{HEXLOWER, HEXLOWER_PERMISSIVE};
use duplicate::duplicate_item;
use prost::Message as _;
use rand::Rng as _;
use tracing::warn;

use crate::{
    common::{BlobId, GroupIdentity, MessageId, ThreemaId},
    model::message::{
        CallAnswerMessage, CallHangupMessage, CallIceCandidateMessage, CallOfferMessage, CallRingingMessage,
        ContactMessageBody, DeleteMessage, DeleteProfilePictureMessage, DeliveryReceiptMessage,
        DeliveryReceiptType, DistributionListMessageBody, EditMessage, FileMessage, FileMetadata,
        FileRenderingType, GroupCallStartMessage, GroupLeaveMessage, GroupMessageBody, GroupNameMessage,
        GroupSetupMessage, GroupSyncRequestMessage, IncomingGroupMessageBody, IncomingMessageBody,
        LegacyReactionMessage, LegacyReactionType, LocationMessage, OutgoingContactMessageBody,
        OutgoingDistributionListMessageBody, OutgoingGroupMessageBody, OutgoingMessageBody, PollSetupMessage,
        PollVoteMessage, ReactionAction, ReactionMessage, RequestProfilePictureMessage,
        SetProfilePictureMessage, TextMessage, TypingIndicatorMessage, WebSessionResumeMessage,
    },
    protobuf::{self, common::CspE2eMessageType},
    utils::{
        bytes::{ByteReader, ByteReaderError, ByteWriter, ByteWriterError, SliceByteReader},
        number::CheckedExactDiv as _,
    },
};

/// The minimum amount of padding to add to messages.
const MESSAGE_DATA_PADDING_LENGTH_MIN: u8 = 32;

/// An error occurred while decoding a message.
#[derive(Clone, Debug, thiserror::Error)]
pub enum CspMessageDecodeError {
    /// Decoding the message failed.
    #[error("Decoding message failed: {0}")]
    DecodingFailed(#[from] ByteReaderError),

    /// Invalid UTF-8 contained within the message.
    #[error("Invalid UTF-8: {0}")]
    InvalidString(#[from] Utf8Error),

    /// Invalid message.
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
}

/// Decoder for CSP messages.
pub(crate) trait CspMessageDecoder: Sized {
    /// Decode a CSP E2E message from the protocol type and the raw data. `sender_identity` is the
    /// creator of groups addressed by a `group-creator-container`.
    fn decode(
        sender_identity: ThreemaId,
        message_type: CspE2eMessageType,
        message_data: &[u8],
    ) -> Result<Self, CspMessageDecodeError>;
}

/// Encoder for CSP messages.
pub(crate) trait CspMessageEncoder {
    /// Encode a message as a CSP E2E message into a `writer`.
    fn encode_into<TWriter: ByteWriter>(&self, writer: &mut TWriter) -> Result<(), ByteWriterError>;
}

trait InternalCspMessageDecoder: Sized {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError>;
}

trait InternalCspMessageEncoder: Sized {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError>;
}

/// A protocol compatible `delivery-receipt` message, mapping either to [`DeliveryReceiptMessage`] or
/// [`LegacyReactionMessage`].
enum ProtocolDeliveryReceiptMessage {
    DeliveryReceipt(DeliveryReceiptMessage),
    LegacyReaction(LegacyReactionMessage),
}
impl ProtocolDeliveryReceiptMessage {
    #[inline]
    fn decode_message_ids(reader: &mut impl ByteReader) -> Result<Vec<MessageId>, CspMessageDecodeError> {
        let n_messages_ids = reader.remaining().checked_exact_div_(MessageId::LENGTH).ok_or(
            CspMessageDecodeError::InvalidMessage(
                "Misaligned message ids in delivery receipt / legacy reaction".to_owned(),
            ),
        )?;
        let mut messages_ids: Vec<MessageId> = Vec::with_capacity(n_messages_ids);
        for _ in 0..n_messages_ids {
            messages_ids.push(MessageId::from(reader.read_fixed::<{ MessageId::LENGTH }>()?));
        }
        Ok(messages_ids)
    }

    #[inline]
    fn encode_into(
        r#type: u8,
        message_ids: &[MessageId],
        writer: &mut impl ByteWriter,
    ) -> Result<(), ByteWriterError> {
        writer.write_u8(r#type)?;
        for message_id in message_ids {
            writer.write(&message_id.to_bytes())?;
        }
        Ok(())
    }
}
impl InternalCspMessageDecoder for ProtocolDeliveryReceiptMessage {
    #[inline]
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let receipt_type = reader.read_u8()?;

        // Attempt to decode as a delivery receipt, fall back to a legacy reaction.
        if let Some(receipt_type) = DeliveryReceiptType::from_repr(receipt_type) {
            return Ok(ProtocolDeliveryReceiptMessage::DeliveryReceipt(
                DeliveryReceiptMessage {
                    receipt_type,
                    message_ids: Self::decode_message_ids(reader)?,
                },
            ));
        }
        if let Some(reaction_type) = LegacyReactionType::from_repr(receipt_type) {
            return Ok(ProtocolDeliveryReceiptMessage::LegacyReaction(
                LegacyReactionMessage {
                    reaction_type,
                    message_ids: Self::decode_message_ids(reader)?,
                },
            ));
        }

        Err(CspMessageDecodeError::InvalidMessage(format!(
            "Unknown delivery receipt type: {receipt_type:02x}"
        )))
    }
}

impl InternalCspMessageEncoder for DeliveryReceiptMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        ProtocolDeliveryReceiptMessage::encode_into(self.receipt_type as u8, &self.message_ids, writer)
    }
}

impl InternalCspMessageEncoder for LegacyReactionMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        ProtocolDeliveryReceiptMessage::encode_into(self.reaction_type as u8, &self.message_ids, writer)
    }
}

impl InternalCspMessageDecoder for TextMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let text = str::from_utf8(reader.read_remaining())?;
        Ok(Self {
            text: text.to_owned(),
        })
    }
}
impl InternalCspMessageEncoder for TextMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(self.text.as_bytes())
    }
}

impl InternalCspMessageDecoder for LocationMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let raw = str::from_utf8(reader.read_remaining())?;
        let mut lines = raw.split('\n');

        // Extract latitude, longitude and optional accuracy from this beautifully formatted soup of strings
        let (latitude, longitude, accuracy_m) = {
            let decode_next_f64 = |name: &'static str, values: &mut Split<'_, char>| {
                f64::from_str(values.next().ok_or_else(|| {
                    CspMessageDecodeError::InvalidMessage(format!("Missing {name} in location message"))
                })?)
                .map_err(|error| {
                    CspMessageDecodeError::InvalidMessage(format!(
                        "Invalid {name} in location message: {error}"
                    ))
                })
            };

            let mut values = lines
                .next()
                .expect("lines must contain at least one line")
                .split(',');
            let latitude = decode_next_f64("latitude", &mut values)?;
            let longitude = decode_next_f64("longitude", &mut values)?;
            let accuracy_m = decode_next_f64("accuracy", &mut values).ok();

            (latitude, longitude, accuracy_m)
        };

        // Extract name and address, which could have been stirred either way /chef-kiss.
        let (name, address) = {
            let line1 = lines.next();
            let line2 = lines.next();
            match (line1, line2) {
                (None, _) => (None, None),
                (Some(address), None) => (None, Some(address)),
                (Some(name), Some(address)) => (Some(name.to_owned()), Some(address)),
            }
        };

        // Unescape all the escaped new-lines from the address to remove some of the spiciness.
        let address = address.map(|address| address.replace("\\n", "\n"));

        // Done serving this exquisite menu.
        Ok(LocationMessage {
            latitude,
            longitude,
            accuracy_m,
            name,
            address,
        })
    }
}
impl InternalCspMessageEncoder for LocationMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        // Write latitude, longitude and optional accuracy.
        writer.write(self.latitude.to_string().as_bytes())?;
        writer.write(",".as_bytes())?;
        writer.write(self.longitude.to_string().as_bytes())?;
        if let Some(accuracy_m) = self.accuracy_m {
            writer.write(",".as_bytes())?;
            writer.write(accuracy_m.to_string().as_bytes())?;
        }

        // Write name and address (if any).
        match (
            &self.name,
            self.address.as_ref().map(|address| address.replace('\n', "\\n")),
        ) {
            (None, None) => {},
            (None, Some(address)) => {
                writer.write("\n".as_bytes())?;
                writer.write(address.as_bytes())?;
            },
            (Some(_), None) => {
                warn!("Discarding name in location message without an associated address");
            },
            (Some(name), Some(address)) => {
                writer.write("\n".as_bytes())?;
                writer.write(name.as_bytes())?;
                writer.write("\n".as_bytes())?;
                writer.write(address.as_bytes())?;
            },
        }
        Ok(())
    }
}

impl InternalCspMessageDecoder for WebSessionResumeMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        Ok(Self {
            push_payload: reader.read_remaining().to_vec(),
        })
    }
}
impl InternalCspMessageEncoder for WebSessionResumeMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(&self.push_payload)
    }
}

#[duplicate_item(
    message_type;
    [ CallOfferMessage ];
    [ CallAnswerMessage ];
    [ CallIceCandidateMessage ];
    [ CallHangupMessage ];
    [ CallRingingMessage ];
)]
impl InternalCspMessageDecoder for message_type {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        Ok(Self {
            json: reader.read_remaining().to_vec(),
        })
    }
}
#[duplicate_item(
    message_type;
    [ CallOfferMessage ];
    [ CallAnswerMessage ];
    [ CallIceCandidateMessage ];
    [ CallHangupMessage ];
    [ CallRingingMessage ];
)]
impl InternalCspMessageEncoder for message_type {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(&self.json)
    }
}

impl InternalCspMessageDecoder for TypingIndicatorMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let is_typing = match reader.read_u8()? {
            0 => false,
            1 => true,
            value => {
                return Err(CspMessageDecodeError::InvalidMessage(format!(
                    "Invalid is-typing value {value}"
                )));
            },
        };
        Ok(Self { is_typing })
    }
}
impl InternalCspMessageEncoder for TypingIndicatorMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write_u8(u8::from(self.is_typing))
    }
}

/// Maximum byte length of a reaction's emoji.
const REACTION_EMOJI_LENGTH_MAX: usize = 64;

impl InternalCspMessageDecoder for ReactionMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let reaction = protobuf::csp_e2e::Reaction::decode(reader.read_remaining())
            .map_err(|error| CspMessageDecodeError::InvalidMessage(format!("Invalid reaction: {error}")))?;
        let emoji = |bytes: Vec<u8>| -> Result<String, CspMessageDecodeError> {
            if bytes.len() > REACTION_EMOJI_LENGTH_MAX {
                return Err(CspMessageDecodeError::InvalidMessage(
                    "Reaction emoji too long".to_owned(),
                ));
            }
            let emoji = String::from_utf8(bytes)
                .map_err(|error| CspMessageDecodeError::InvalidMessage(error.to_string()))?;
            if emoji.is_empty() {
                return Err(CspMessageDecodeError::InvalidMessage(
                    "Empty reaction emoji".to_owned(),
                ));
            }
            Ok(emoji)
        };
        let action = match reaction.action {
            Some(protobuf::csp_e2e::reaction::Action::Apply(bytes)) => ReactionAction::Apply(emoji(bytes)?),
            Some(protobuf::csp_e2e::reaction::Action::Withdraw(bytes)) => {
                ReactionAction::Withdraw(emoji(bytes)?)
            },
            None => {
                return Err(CspMessageDecodeError::InvalidMessage(
                    "Reaction without action".to_owned(),
                ));
            },
        };
        Ok(Self {
            message_id: MessageId(reaction.message_id),
            action,
        })
    }
}
impl InternalCspMessageEncoder for ReactionMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        let action = match &self.action {
            ReactionAction::Apply(emoji) => {
                protobuf::csp_e2e::reaction::Action::Apply(emoji.as_bytes().to_vec())
            },
            ReactionAction::Withdraw(emoji) => {
                protobuf::csp_e2e::reaction::Action::Withdraw(emoji.as_bytes().to_vec())
            },
        };
        writer.write(
            &protobuf::csp_e2e::Reaction {
                message_id: self.message_id.0,
                action: Some(action),
            }
            .encode_to_vec(),
        )
    }
}

impl InternalCspMessageDecoder for EditMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let edit = protobuf::csp_e2e::EditMessage::decode(reader.read_remaining()).map_err(|error| {
            CspMessageDecodeError::InvalidMessage(format!("Invalid edit message: {error}"))
        })?;
        Ok(Self {
            message_id: MessageId(edit.message_id),
            text: edit.text,
        })
    }
}
impl InternalCspMessageEncoder for EditMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(
            &protobuf::csp_e2e::EditMessage {
                message_id: self.message_id.0,
                text: self.text.clone(),
            }
            .encode_to_vec(),
        )
    }
}

impl InternalCspMessageDecoder for GroupCallStartMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let start = protobuf::csp_e2e::GroupCallStart::decode(reader.read_remaining()).map_err(|error| {
            CspMessageDecodeError::InvalidMessage(format!("Invalid group call start message: {error}"))
        })?;
        let gck = start.gck.try_into().map_err(|_| {
            CspMessageDecodeError::InvalidMessage(
                "Invalid group call start message: GCK not 32 bytes".to_owned(),
            )
        })?;
        Ok(Self {
            protocol_version: start.protocol_version,
            gck,
            sfu_base_url: start.sfu_base_url,
        })
    }
}
impl InternalCspMessageEncoder for GroupCallStartMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(
            &protobuf::csp_e2e::GroupCallStart {
                protocol_version: self.protocol_version,
                gck: self.gck.to_vec(),
                sfu_base_url: self.sfu_base_url.clone(),
            }
            .encode_to_vec(),
        )
    }
}

impl InternalCspMessageDecoder for DeleteMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let delete = protobuf::csp_e2e::DeleteMessage::decode(reader.read_remaining()).map_err(|error| {
            CspMessageDecodeError::InvalidMessage(format!("Invalid delete message: {error}"))
        })?;
        Ok(Self {
            message_id: MessageId(delete.message_id),
        })
    }
}
impl InternalCspMessageEncoder for DeleteMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(
            &protobuf::csp_e2e::DeleteMessage {
                message_id: self.message_id.0,
            }
            .encode_to_vec(),
        )
    }
}

/// A field that may be anything: of another type it counts as absent, so that one sloppy value
/// doesn't lose the message (Android keeps metadata untyped and coerces the rest).
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}

/// A rendering type: an unassigned number means a plain file (the spec), not a number means none
/// was given.
fn lenient_rendering_type<'de, D>(deserializer: D) -> Result<Option<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
    Ok(value.as_f64().map(|number| {
        if (1.0_f64..2.0_f64).contains(&number) {
            1
        } else if (2.0_f64..3.0_f64).contains(&number) {
            2
        } else {
            0
        }
    }))
}

/// The size in bytes, also when written as a float or in a string, which Android's `getInt` takes
/// too. Only shown, so a negative one is just 0.
fn lenient_size<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
    if let Some(size) = value.as_u64() {
        return Ok(size);
    }
    let number = value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.trim().parse::<f64>().ok()));
    match number {
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "Floats convert saturating, and a size this large is nonsense either way"
        )]
        Some(size) => Ok(size as u64),
        None => Err(serde::de::Error::custom("size is not a number")),
    }
}

/// The media type: Android's `getString` takes any value, so one that isn't a text doesn't lose the
/// message. Here it means no type, which shows as a plain file.
fn lenient_media_type<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = <serde_json::Value as serde::Deserialize>::deserialize(deserializer)?;
    Ok(value.as_str().unwrap_or_default().to_owned())
}

/// The JSON of a `file` message (see `csp.struct.yml`). Unknown fields are ignored; `i` is the
/// deprecated fallback for `j`. What the message can't do without (key, blobs, size) has to be
/// right; the rest is taken leniently, and the spec's limits on name and caption are for sending.
#[derive(serde::Serialize, serde::Deserialize, Default)]
struct FileMessageJson {
    #[serde(
        rename = "j",
        default,
        deserialize_with = "lenient_rendering_type",
        skip_serializing_if = "Option::is_none"
    )]
    rendering_type: Option<u8>,
    #[serde(
        rename = "i",
        default,
        deserialize_with = "lenient_rendering_type",
        skip_serializing_if = "Option::is_none"
    )]
    deprecated_rendering_type: Option<u8>,
    #[serde(rename = "k")]
    key: String,
    #[serde(rename = "b")]
    blob_id: String,
    #[serde(rename = "m", default, deserialize_with = "lenient_media_type")]
    media_type: String,
    #[serde(
        rename = "n",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    name: Option<String>,
    #[serde(rename = "s", deserialize_with = "lenient_size")]
    size: u64,
    #[serde(rename = "t", default, skip_serializing_if = "Option::is_none")]
    thumbnail_blob_id: Option<String>,
    #[serde(
        rename = "p",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    thumbnail_media_type: Option<String>,
    #[serde(
        rename = "d",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    caption: Option<String>,
    #[serde(
        rename = "c",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    correlation_id: Option<String>,
    #[serde(
        rename = "x",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    metadata: Option<FileMetadataJson>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct FileMetadataJson {
    #[serde(
        rename = "w",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    width: Option<u32>,
    #[serde(
        rename = "h",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    height: Option<u32>,
    #[serde(
        rename = "a",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    animated: Option<bool>,
    #[serde(
        rename = "d",
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    duration: Option<f64>,
}

impl InternalCspMessageDecoder for FileMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let invalid =
            |reason: &str| CspMessageDecodeError::InvalidMessage(format!("Invalid file message: {reason}"));
        let json: FileMessageJson =
            serde_json::from_slice(reader.read_remaining()).map_err(|error| invalid(&error.to_string()))?;
        let key: [u8; 32] = HEXLOWER_PERMISSIVE
            .decode(json.key.as_bytes())
            .ok()
            .and_then(|key| key.try_into().ok())
            .ok_or_else(|| invalid("key"))?;
        let blob_id = |hex: &str| -> Result<BlobId, CspMessageDecodeError> {
            HEXLOWER_PERMISSIVE
                .decode(hex.as_bytes())
                .ok()
                .and_then(|id| <[u8; BlobId::LENGTH]>::try_from(id).ok())
                .map(BlobId)
                .ok_or_else(|| invalid("blob id"))
        };
        let rendering_type = match json.rendering_type.or(json.deprecated_rendering_type) {
            Some(1) => FileRenderingType::Media,
            Some(2) => FileRenderingType::Sticker,
            _ => FileRenderingType::File,
        };
        let metadata = json.metadata.unwrap_or_default();
        Ok(Self {
            rendering_type,
            key,
            blob_id: blob_id(&json.blob_id)?,
            media_type: json.media_type,
            name: json.name.filter(|name| !name.is_empty()),
            size: json.size,
            thumbnail_blob_id: json
                .thumbnail_blob_id
                .filter(|id| !id.is_empty())
                .map(|id| blob_id(&id))
                .transpose()?,
            thumbnail_media_type: json.thumbnail_media_type,
            caption: json.caption.filter(|caption| !caption.is_empty()),
            correlation_id: json.correlation_id,
            metadata: FileMetadata {
                width: metadata.width,
                height: metadata.height,
                animated: metadata.animated,
                duration: metadata.duration,
                legacy_nonce: None,
            },
        })
    }
}
impl InternalCspMessageEncoder for FileMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        let rendering_type = match self.rendering_type {
            FileRenderingType::File => 0,
            FileRenderingType::Media => 1,
            FileRenderingType::Sticker => 2,
        };
        let metadata = &self.metadata;
        let has_metadata = metadata.width.is_some()
            || metadata.height.is_some()
            || metadata.animated.is_some()
            || metadata.duration.is_some();
        let json = FileMessageJson {
            rendering_type: Some(rendering_type),
            deprecated_rendering_type: Some(u8::from(rendering_type != 0)),
            key: HEXLOWER.encode(&self.key),
            blob_id: HEXLOWER.encode(&self.blob_id.0),
            media_type: self.media_type.clone(),
            name: self.name.clone(),
            size: self.size,
            thumbnail_blob_id: self.thumbnail_blob_id.map(|id| HEXLOWER.encode(&id.0)),
            thumbnail_media_type: self.thumbnail_media_type.clone(),
            caption: self.caption.clone(),
            correlation_id: self.correlation_id.clone(),
            metadata: has_metadata.then(|| FileMetadataJson {
                width: metadata.width,
                height: metadata.height,
                animated: metadata.animated,
                duration: metadata.duration,
            }),
        };
        writer.write(&serde_json::to_vec(&json).expect("file message JSON must serialize"))
    }
}

/// The deprecated media messages come in as the [`FileMessage`] they would be today, as on Android
/// (`LegacyMessageTransformer`); nobody may send them any more. What follows their fields is ignored.
struct DeprecatedMedia;
impl DeprecatedMedia {
    const AUDIO_MEDIA_TYPE: &str = "audio/aac";
    const IMAGE_MEDIA_TYPE: &str = "image/jpeg";
    const VIDEO_MEDIA_TYPE: &str = "video/mpeg";

    fn file(media_type: &str, key: [u8; 32], blob_id: BlobId, size: u32) -> FileMessage {
        FileMessage {
            rendering_type: FileRenderingType::Media,
            key,
            blob_id,
            media_type: media_type.to_owned(),
            name: None,
            size: u64::from(size),
            thumbnail_blob_id: None,
            thumbnail_media_type: None,
            caption: None,
            correlation_id: None,
            metadata: FileMetadata::default(),
        }
    }

    /// `deprecated-image`: no key, the blob is encrypted between sender and receiver with a nonce.
    fn decode_image(reader: &mut impl ByteReader) -> Result<FileMessage, CspMessageDecodeError> {
        let blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let size = reader.read_u32_le()?;
        let nonce = reader.read_fixed::<24>()?;
        let _ = reader.read_remaining();
        let mut file = Self::file(Self::IMAGE_MEDIA_TYPE, [0; 32], blob_id, size);
        file.metadata.legacy_nonce = Some(nonce);
        Ok(file)
    }

    /// `deprecated-group-image`.
    fn decode_group_image(reader: &mut impl ByteReader) -> Result<FileMessage, CspMessageDecodeError> {
        let blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let size = reader.read_u32_le()?;
        let key = reader.read_fixed::<32>()?;
        let _ = reader.read_remaining();
        Ok(Self::file(Self::IMAGE_MEDIA_TYPE, key, blob_id, size))
    }

    /// `deprecated-audio`, also wrapped for a group.
    fn decode_audio(reader: &mut impl ByteReader) -> Result<FileMessage, CspMessageDecodeError> {
        let duration = reader.read_u16_le()?;
        let blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let size = reader.read_u32_le()?;
        let key = reader.read_fixed::<32>()?;
        let _ = reader.read_remaining();
        let mut file = Self::file(Self::AUDIO_MEDIA_TYPE, key, blob_id, size);
        file.metadata.duration = Some(f64::from(duration));
        Ok(file)
    }

    /// `deprecated-video`, also wrapped for a group.
    fn decode_video(reader: &mut impl ByteReader) -> Result<FileMessage, CspMessageDecodeError> {
        let duration = reader.read_u16_le()?;
        let blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let size = reader.read_u32_le()?;
        let thumbnail_blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let _thumbnail_size = reader.read_u32_le()?;
        let key = reader.read_fixed::<32>()?;
        let _ = reader.read_remaining();
        let mut file = Self::file(Self::VIDEO_MEDIA_TYPE, key, blob_id, size);
        file.thumbnail_blob_id = Some(thumbnail_blob_id);
        file.thumbnail_media_type = Some(Self::IMAGE_MEDIA_TYPE.to_owned());
        file.metadata.duration = Some(f64::from(duration));
        Ok(file)
    }
}

impl InternalCspMessageDecoder for GroupSetupMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let data = reader.read_remaining();
        if data.len() % ThreemaId::LENGTH != 0 {
            return Err(CspMessageDecodeError::InvalidMessage(
                "Misaligned member identities in group setup".to_owned(),
            ));
        }
        let members = data
            .chunks_exact(ThreemaId::LENGTH)
            .map(ThreemaId::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                CspMessageDecodeError::InvalidMessage(format!(
                    "Invalid member identity in group setup: {error}"
                ))
            })?;
        Ok(Self { members })
    }
}
impl InternalCspMessageEncoder for GroupSetupMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        for member in &self.members {
            writer.write(&member.to_bytes())?;
        }
        Ok(())
    }
}

impl InternalCspMessageDecoder for GroupNameMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let name = String::from_utf8(reader.read_remaining().to_vec())
            .map_err(|error| CspMessageDecodeError::InvalidMessage(format!("Invalid group name: {error}")))?;
        Ok(Self { name })
    }
}
impl InternalCspMessageEncoder for GroupNameMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(self.name.as_bytes())
    }
}

impl InternalCspMessageDecoder for SetProfilePictureMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let blob_id = BlobId(reader.read_fixed::<{ BlobId::LENGTH }>()?);
        let size = reader.read_u32_le()?;
        let key = reader.read_fixed::<32>()?;
        Ok(Self { blob_id, size, key })
    }
}
impl InternalCspMessageEncoder for SetProfilePictureMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(&self.blob_id.0)?;
        writer.write_u32_le(self.size)?;
        writer.write(&self.key)
    }
}

/// Messages without content (`group-leave`, `group-sync-request`, `delete-profile-picture`).
macro_rules! empty_message {
    ($($message:ident),*) => {$(
        impl InternalCspMessageDecoder for $message {
            fn decode(_reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
                Ok(Self)
            }
        }
        impl InternalCspMessageEncoder for $message {
            fn encode_into(&self, _writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
                Ok(())
            }
        }
    )*};
}
empty_message!(
    GroupLeaveMessage,
    GroupSyncRequestMessage,
    DeleteProfilePictureMessage,
    RequestProfilePictureMessage
);

impl InternalCspMessageDecoder for PollSetupMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let poll_id = reader.read_u64_le()?;
        let poll = String::from_utf8(reader.read_remaining().to_vec())
            .map_err(|error| CspMessageDecodeError::InvalidMessage(format!("Invalid poll: {error}")))?;
        Ok(Self { poll_id, poll })
    }
}
impl InternalCspMessageEncoder for PollSetupMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write_u64_le(self.poll_id)?;
        writer.write(self.poll.as_bytes())
    }
}

impl InternalCspMessageDecoder for PollVoteMessage {
    fn decode(reader: &mut impl ByteReader) -> Result<Self, CspMessageDecodeError> {
        let creator_identity = ThreemaId::try_from(reader.read_fixed::<{ ThreemaId::LENGTH }>()?.as_slice())
            .map_err(|error| {
                CspMessageDecodeError::InvalidMessage(format!("Invalid poll creator: {error}"))
            })?;
        let poll_id = reader.read_u64_le()?;
        let choices = String::from_utf8(reader.read_remaining().to_vec())
            .map_err(|error| CspMessageDecodeError::InvalidMessage(format!("Invalid poll vote: {error}")))?;
        Ok(Self {
            creator_identity,
            poll_id,
            choices,
        })
    }
}
impl InternalCspMessageEncoder for PollVoteMessage {
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        writer.write(&self.creator_identity.to_bytes())?;
        writer.write_u64_le(self.poll_id)?;
        writer.write(self.choices.as_bytes())
    }
}

struct IncomingMessageBodyExtension;
impl IncomingMessageBodyExtension {
    #[inline]
    fn decode_group_creator_header(
        sender_identity: ThreemaId,
        reader: &mut impl ByteReader,
    ) -> Result<GroupIdentity, CspMessageDecodeError> {
        let group_id = reader.read_u64_le()?;
        Ok(GroupIdentity {
            group_id,
            creator_identity: sender_identity,
        })
    }

    #[inline]
    fn decode_group_member_header(
        reader: &mut impl ByteReader,
    ) -> Result<GroupIdentity, CspMessageDecodeError> {
        let creator_identity = ThreemaId::try_from(reader.read_fixed::<{ ThreemaId::LENGTH }>()?.as_slice())
            .map_err(|error| {
                CspMessageDecodeError::InvalidMessage(format!(
                    "Invalid group creator identity in group message header: {error}"
                ))
            })?;
        let group_id = reader.read_u64_le()?;
        Ok(GroupIdentity {
            group_id,
            creator_identity,
        })
    }
}

impl CspMessageDecoder for IncomingMessageBody {
    /// Try to decode the incoming message's body from the protocol type and the raw data.
    fn decode(
        sender_identity: ThreemaId,
        message_type: CspE2eMessageType,
        message_data: &[u8],
    ) -> Result<Self, CspMessageDecodeError> {
        let mut reader = SliceByteReader::new(message_data);
        let member =
            |reader: &mut SliceByteReader,
             body: fn(&mut SliceByteReader) -> Result<GroupMessageBody, CspMessageDecodeError>| {
                let group_identity = IncomingMessageBodyExtension::decode_group_member_header(reader)?;
                Ok::<_, CspMessageDecodeError>(Self::Group(IncomingGroupMessageBody {
                    group_identity,
                    body: body(reader)?,
                }))
            };
        let creator =
            |reader: &mut SliceByteReader,
             body: fn(&mut SliceByteReader) -> Result<GroupMessageBody, CspMessageDecodeError>| {
                let group_identity =
                    IncomingMessageBodyExtension::decode_group_creator_header(sender_identity, reader)?;
                Ok::<_, CspMessageDecodeError>(Self::Group(IncomingGroupMessageBody {
                    group_identity,
                    body: body(reader)?,
                }))
            };

        // Decode message body.
        let message = match message_type {
            CspE2eMessageType::Text => {
                Self::Contact(ContactMessageBody::Text(TextMessage::decode(&mut reader)?))
            },

            CspE2eMessageType::Location => Self::Contact(ContactMessageBody::Location(
                LocationMessage::decode(&mut reader)?,
            )),

            CspE2eMessageType::DeliveryReceipt => {
                Self::Contact(match ProtocolDeliveryReceiptMessage::decode(&mut reader)? {
                    ProtocolDeliveryReceiptMessage::DeliveryReceipt(delivery_receipt) => {
                        ContactMessageBody::DeliveryReceipt(delivery_receipt)
                    },
                    ProtocolDeliveryReceiptMessage::LegacyReaction(legacy_reaction) => {
                        ContactMessageBody::LegacyReaction(legacy_reaction)
                    },
                })
            },

            CspE2eMessageType::WebSessionResume => Self::Contact(ContactMessageBody::WebSessionResume(
                WebSessionResumeMessage::decode(&mut reader)?,
            )),

            CspE2eMessageType::CallOffer => Self::Contact(ContactMessageBody::CallOffer(
                CallOfferMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::CallAnswer => Self::Contact(ContactMessageBody::CallAnswer(
                CallAnswerMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::CallIceCandidate => Self::Contact(ContactMessageBody::CallIceCandidate(
                CallIceCandidateMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::CallHangup => Self::Contact(ContactMessageBody::CallHangup(
                CallHangupMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::CallRinging => Self::Contact(ContactMessageBody::CallRinging(
                CallRingingMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::TypingIndicator => Self::Contact(ContactMessageBody::TypingIndicator(
                TypingIndicatorMessage::decode(&mut reader)?,
            )),

            CspE2eMessageType::Reaction => Self::Contact(ContactMessageBody::Reaction(
                ReactionMessage::decode(&mut reader)?,
            )),

            CspE2eMessageType::File => {
                Self::Contact(ContactMessageBody::File(FileMessage::decode(&mut reader)?))
            },
            CspE2eMessageType::DeprecatedImage => Self::Contact(ContactMessageBody::File(
                DeprecatedMedia::decode_image(&mut reader)?,
            )),
            CspE2eMessageType::DeprecatedAudio => Self::Contact(ContactMessageBody::File(
                DeprecatedMedia::decode_audio(&mut reader)?,
            )),
            CspE2eMessageType::DeprecatedVideo => Self::Contact(ContactMessageBody::File(
                DeprecatedMedia::decode_video(&mut reader)?,
            )),

            CspE2eMessageType::ContactSetProfilePicture => Self::Contact(
                ContactMessageBody::SetProfilePicture(SetProfilePictureMessage::decode(&mut reader)?),
            ),
            CspE2eMessageType::ContactDeleteProfilePicture => Self::Contact(
                ContactMessageBody::DeleteProfilePicture(DeleteProfilePictureMessage::decode(&mut reader)?),
            ),
            CspE2eMessageType::ContactRequestProfilePicture => Self::Contact(
                ContactMessageBody::RequestProfilePicture(RequestProfilePictureMessage::decode(&mut reader)?),
            ),
            CspE2eMessageType::PollSetup => Self::Contact(ContactMessageBody::PollSetup(
                PollSetupMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::PollVote => Self::Contact(ContactMessageBody::PollVote(
                PollVoteMessage::decode(&mut reader)?,
            )),
            CspE2eMessageType::GroupPollSetup => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::PollSetup(PollSetupMessage::decode(
                    group_reader,
                )?))
            })?,
            CspE2eMessageType::GroupPollVote => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::PollVote(PollVoteMessage::decode(group_reader)?))
            })?,

            CspE2eMessageType::EditMessage => {
                Self::Contact(ContactMessageBody::Edit(EditMessage::decode(&mut reader)?))
            },

            CspE2eMessageType::DeleteMessage => {
                Self::Contact(ContactMessageBody::Delete(DeleteMessage::decode(&mut reader)?))
            },

            CspE2eMessageType::GroupText => {
                let group_identity = IncomingMessageBodyExtension::decode_group_member_header(&mut reader)?;
                let body = GroupMessageBody::Text(TextMessage::decode(&mut reader)?);
                Self::Group(IncomingGroupMessageBody { group_identity, body })
            },

            CspE2eMessageType::GroupLocation => {
                let group_identity = IncomingMessageBodyExtension::decode_group_member_header(&mut reader)?;
                let body = GroupMessageBody::Location(LocationMessage::decode(&mut reader)?);
                Self::Group(IncomingGroupMessageBody { group_identity, body })
            },

            CspE2eMessageType::GroupDeliveryReceipt => {
                let group_identity = IncomingMessageBodyExtension::decode_group_member_header(&mut reader)?;
                let body = match ProtocolDeliveryReceiptMessage::decode(&mut reader)? {
                    ProtocolDeliveryReceiptMessage::DeliveryReceipt(_) => {
                        return Err(CspMessageDecodeError::InvalidMessage(
                            "Unexpected group delivery receipt".to_owned(),
                        ));
                    },
                    ProtocolDeliveryReceiptMessage::LegacyReaction(legacy_reaction) => {
                        GroupMessageBody::LegacyReaction(legacy_reaction)
                    },
                };
                Self::Group(IncomingGroupMessageBody { group_identity, body })
            },

            CspE2eMessageType::GroupFile => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::File(FileMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupImage => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::File(DeprecatedMedia::decode_group_image(
                    group_reader,
                )?))
            })?,
            CspE2eMessageType::GroupAudio => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::File(DeprecatedMedia::decode_audio(
                    group_reader,
                )?))
            })?,
            CspE2eMessageType::GroupVideo => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::File(DeprecatedMedia::decode_video(
                    group_reader,
                )?))
            })?,
            CspE2eMessageType::GroupReaction => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Reaction(ReactionMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupEditMessage => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Edit(EditMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupDeleteMessage => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Delete(DeleteMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupLeave => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Leave(GroupLeaveMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupSyncRequest => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::SyncRequest(GroupSyncRequestMessage::decode(
                    group_reader,
                )?))
            })?,
            CspE2eMessageType::GroupSetup => creator(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Setup(GroupSetupMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupName => creator(&mut reader, |group_reader| {
                Ok(GroupMessageBody::Name(GroupNameMessage::decode(group_reader)?))
            })?,
            CspE2eMessageType::GroupSetProfilePicture => creator(&mut reader, |group_reader| {
                Ok(GroupMessageBody::SetProfilePicture(
                    SetProfilePictureMessage::decode(group_reader)?,
                ))
            })?,
            CspE2eMessageType::GroupDeleteProfilePicture => creator(&mut reader, |group_reader| {
                Ok(GroupMessageBody::DeleteProfilePicture(
                    DeleteProfilePictureMessage::decode(group_reader)?,
                ))
            })?,

            CspE2eMessageType::GroupCallStart => member(&mut reader, |group_reader| {
                Ok(GroupMessageBody::CallStart(GroupCallStartMessage::decode(
                    group_reader,
                )?))
            })?,

            // TODO(LIB-16): Decode the rest.
            _ => {
                return Err(CspMessageDecodeError::InvalidMessage(
                    "Ain't nobody got time to implement those incoming messages".to_owned(),
                ));
            },
        };

        // Done.
        let _ = reader.expect_consumed()?;
        Ok(message)
    }
}

struct OutgoingGroupMessageBodyExtension;
impl OutgoingGroupMessageBodyExtension {
    #[inline]
    fn encode_group_creator_header_into(
        group_identity: &GroupIdentity,
        writer: &mut impl ByteWriter,
    ) -> Result<(), ByteWriterError> {
        writer.write_u64_le(group_identity.group_id)
    }

    #[inline]
    fn encode_group_member_header_into(
        group_identity: &GroupIdentity,
        writer: &mut impl ByteWriter,
    ) -> Result<(), ByteWriterError> {
        writer.write(&group_identity.creator_identity.to_bytes())?;
        writer.write_u64_le(group_identity.group_id)
    }
}

impl InternalCspMessageEncoder for OutgoingGroupMessageBody {
    /// Encode the group message with its respective header (either `group-creator-container` or
    /// `group-member-container`).
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        if self.body.uses_creator_container() {
            OutgoingGroupMessageBodyExtension::encode_group_creator_header_into(
                &self.group_identity,
                writer,
            )?;
        } else {
            OutgoingGroupMessageBodyExtension::encode_group_member_header_into(&self.group_identity, writer)?;
        }
        match &self.body {
            GroupMessageBody::Text(message) => message.encode_into(writer),
            GroupMessageBody::Location(message) => message.encode_into(writer),
            GroupMessageBody::LegacyReaction(message) => message.encode_into(writer),
            GroupMessageBody::File(message) => message.encode_into(writer),
            GroupMessageBody::PollSetup(message) => message.encode_into(writer),
            GroupMessageBody::PollVote(message) => message.encode_into(writer),
            GroupMessageBody::Reaction(message) => message.encode_into(writer),
            GroupMessageBody::Edit(message) => message.encode_into(writer),
            GroupMessageBody::Delete(message) => message.encode_into(writer),
            GroupMessageBody::Setup(message) => message.encode_into(writer),
            GroupMessageBody::Name(message) => message.encode_into(writer),
            GroupMessageBody::Leave(message) => message.encode_into(writer),
            GroupMessageBody::SyncRequest(message) => message.encode_into(writer),
            GroupMessageBody::SetProfilePicture(message) => message.encode_into(writer),
            GroupMessageBody::DeleteProfilePicture(message) => message.encode_into(writer),
            GroupMessageBody::CallStart(message) => message.encode_into(writer),
        }
    }
}

impl InternalCspMessageEncoder for OutgoingContactMessageBody {
    /// Encode the 1:1 message.
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        match &self.body {
            ContactMessageBody::Text(message) => message.encode_into(writer),
            ContactMessageBody::Location(message) => message.encode_into(writer),
            ContactMessageBody::DeliveryReceipt(message) => message.encode_into(writer),
            ContactMessageBody::LegacyReaction(message) => message.encode_into(writer),
            ContactMessageBody::WebSessionResume(message) => message.encode_into(writer),
            ContactMessageBody::TypingIndicator(message) => message.encode_into(writer),
            ContactMessageBody::Reaction(message) => message.encode_into(writer),
            ContactMessageBody::File(message) => message.encode_into(writer),
            ContactMessageBody::PollSetup(message) => message.encode_into(writer),
            ContactMessageBody::PollVote(message) => message.encode_into(writer),
            ContactMessageBody::SetProfilePicture(message) => message.encode_into(writer),
            ContactMessageBody::DeleteProfilePicture(message) => message.encode_into(writer),
            ContactMessageBody::RequestProfilePicture(message) => message.encode_into(writer),
            ContactMessageBody::Edit(message) => message.encode_into(writer),
            ContactMessageBody::Delete(message) => message.encode_into(writer),
            ContactMessageBody::CallOffer(message) => message.encode_into(writer),
            ContactMessageBody::CallAnswer(message) => message.encode_into(writer),
            ContactMessageBody::CallIceCandidate(message) => message.encode_into(writer),
            ContactMessageBody::CallHangup(message) => message.encode_into(writer),
            ContactMessageBody::CallRinging(message) => message.encode_into(writer),
        }
    }
}

impl InternalCspMessageEncoder for OutgoingDistributionListMessageBody {
    /// Encode the distribution list message.
    ///
    /// Note: This behaves identical to the respective 1:1 message.
    fn encode_into(&self, writer: &mut impl ByteWriter) -> Result<(), ByteWriterError> {
        match &self.body {
            DistributionListMessageBody::Text(message) => message.encode_into(writer),
            DistributionListMessageBody::Location(message) => message.encode_into(writer),
        }
    }
}

impl CspMessageEncoder for OutgoingMessageBody {
    fn encode_into<TWriter: ByteWriter>(&self, writer: &mut TWriter) -> Result<(), ByteWriterError> {
        // Encode type.
        writer.write_u8(self.message_type() as u8)?;

        // Encode data.
        let data_length = {
            let offset = writer.offset();
            match self {
                OutgoingMessageBody::Contact(body) => body.encode_into(writer),
                OutgoingMessageBody::DistributionList(body) => body.encode_into(writer),
                OutgoingMessageBody::Group(body) => body.encode_into(writer),
            }?;
            writer
                .offset()
                .checked_sub(offset)
                .expect("Encoded message data must be >= 0 bytes")
        };

        // Add PKCS#7 padding.
        let padding_length: u8 = rand::thread_rng().gen_range(1..=255);
        let padding_length =
            if data_length.saturating_add(padding_length as usize) < MESSAGE_DATA_PADDING_LENGTH_MIN as usize
            {
                MESSAGE_DATA_PADDING_LENGTH_MIN.saturating_sub(data_length.try_into().expect(
                    "data_length + 1..=255 must be < MESSAGE_DATA_PADDING_LENGTH_MIN and therefore u8",
                ))
            } else {
                padding_length
            };
        writer
            .write_in_place(padding_length as usize)?
            .fill(padding_length);

        // Done.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_sender() -> ThreemaId {
        ThreemaId::try_from("ECHOECHO").unwrap()
    }
    use crate::utils::bytes::OwnedVecByteWriter;

    #[test]
    fn group_call_start_round_trip() {
        let start = GroupCallStartMessage {
            protocol_version: 1,
            gck: [7; 32],
            sfu_base_url: "https://sfu.threema.ch".to_owned(),
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        start.encode_into(&mut writer).unwrap();
        // group-member-container: creator identity and group id, then the message
        let mut data = b"ECHOECHO".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        data.extend_from_slice(&writer.into_inner());
        let decoded =
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::GroupCallStart, &data).unwrap();
        assert!(matches!(
            decoded,
            IncomingMessageBody::Group(IncomingGroupMessageBody { body: GroupMessageBody::CallStart(decoded), .. })
                if decoded == start
        ));
        // A GCK of the wrong length is refused.
        let mut writer = OwnedVecByteWriter::new_empty();
        writer
            .write(
                &protobuf::csp_e2e::GroupCallStart {
                    protocol_version: 1,
                    gck: vec![7; 31],
                    sfu_base_url: String::new(),
                }
                .encode_to_vec(),
            )
            .unwrap();
        let mut data = b"ECHOECHO".to_vec();
        data.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        data.extend_from_slice(&writer.into_inner());
        assert!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::GroupCallStart, &data).is_err()
        );
    }

    #[test]
    fn call_offer_keeps_its_json() {
        let json = br#"{"callId":42,"offer":{"sdpType":"offer","sdp":"v=0"}}"#.to_vec();
        let mut writer = OwnedVecByteWriter::new_empty();
        CallOfferMessage { json: json.clone() }
            .encode_into(&mut writer)
            .unwrap();
        assert_eq!(writer.into_inner(), json);
        let decoded =
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::CallOffer, &json).unwrap();
        assert!(matches!(
            decoded,
            IncomingMessageBody::Contact(ContactMessageBody::CallOffer(CallOfferMessage { json: decoded }))
                if decoded == json
        ));
        // Hangup and ringing may be empty
        let decoded = IncomingMessageBody::decode(test_sender(), CspE2eMessageType::CallHangup, &[]).unwrap();
        assert!(matches!(
            decoded,
            IncomingMessageBody::Contact(ContactMessageBody::CallHangup(CallHangupMessage { json })) if json.is_empty()
        ));
    }

    #[test]
    fn typing_indicator_roundtrip() {
        for is_typing in [false, true] {
            let mut writer = OwnedVecByteWriter::new_empty();
            TypingIndicatorMessage { is_typing }
                .encode_into(&mut writer)
                .unwrap();
            let data = writer.into_inner();
            assert_eq!(data, vec![u8::from(is_typing)]);
            let decoded =
                IncomingMessageBody::decode(test_sender(), CspE2eMessageType::TypingIndicator, &data)
                    .unwrap();
            assert!(matches!(
                decoded,
                IncomingMessageBody::Contact(ContactMessageBody::TypingIndicator(TypingIndicatorMessage {
                    is_typing: decoded
                })) if decoded == is_typing
            ));
        }
        assert!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::TypingIndicator, &[2]).is_err()
        );
        assert!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::TypingIndicator, &[1, 0]).is_err()
        );
    }

    fn decode_reaction(data: &[u8]) -> Result<ReactionMessage, CspMessageDecodeError> {
        match IncomingMessageBody::decode(test_sender(), CspE2eMessageType::Reaction, data)? {
            IncomingMessageBody::Contact(ContactMessageBody::Reaction(reaction)) => Ok(reaction),
            _ => panic!("expected a reaction"),
        }
    }

    #[test]
    fn reaction_roundtrip() {
        for action in [
            ReactionAction::Apply("👍🏽".to_owned()),
            ReactionAction::Withdraw("❤️".to_owned()),
        ] {
            let reaction = ReactionMessage {
                message_id: MessageId(0x0123_4567_89ab_cdef),
                action,
            };
            let mut writer = OwnedVecByteWriter::new_empty();
            reaction.encode_into(&mut writer).unwrap();
            assert_eq!(decode_reaction(&writer.into_inner()).unwrap(), reaction);
        }
    }

    #[test]
    fn reaction_wire_format() {
        // fixed64 message_id = 1, bytes apply = 2
        let mut data = vec![0x09];
        data.extend_from_slice(&42_u64.to_le_bytes());
        data.extend_from_slice(&[0x12, 0x04]);
        data.extend_from_slice("👍".as_bytes());
        assert_eq!(
            decode_reaction(&data).unwrap(),
            ReactionMessage {
                message_id: MessageId(42),
                action: ReactionAction::Apply("👍".to_owned()),
            }
        );
    }

    #[test]
    fn invalid_reactions_are_rejected() {
        let encode = |reaction: protobuf::csp_e2e::Reaction| reaction.encode_to_vec();
        let with = |action| {
            encode(protobuf::csp_e2e::Reaction {
                message_id: 1,
                action,
            })
        };
        assert!(decode_reaction(&with(None)).is_err());
        assert!(
            decode_reaction(&with(Some(
                protobuf::csp_e2e::reaction::Action::Apply(Vec::new())
            )))
            .is_err()
        );
        assert!(
            decode_reaction(&with(Some(protobuf::csp_e2e::reaction::Action::Apply(vec![
                0xff
            ]))))
            .is_err()
        );
        assert!(
            decode_reaction(&with(Some(protobuf::csp_e2e::reaction::Action::Withdraw(
                vec![b'a'; 65]
            ))))
            .is_err()
        );
        assert!(
            decode_reaction(&with(Some(protobuf::csp_e2e::reaction::Action::Apply(vec![
                b'a';
                64
            ]))))
            .is_ok()
        );
    }

    #[test]
    fn edit_and_delete_roundtrip() {
        let edit = EditMessage {
            message_id: MessageId(0x0123_4567_89ab_cdef),
            text: "corrected ✓".to_owned(),
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        edit.encode_into(&mut writer).unwrap();
        assert!(matches!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::EditMessage, &writer.into_inner()).unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::Edit(decoded)) if decoded == edit
        ));

        let delete = DeleteMessage {
            message_id: MessageId(42),
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        delete.encode_into(&mut writer).unwrap();
        let data = writer.into_inner();
        // fixed64 message_id = 1
        let mut expected = vec![0x09];
        expected.extend_from_slice(&42_u64.to_le_bytes());
        assert_eq!(data, expected);
        assert!(matches!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::DeleteMessage, &data).unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::Delete(decoded)) if decoded == delete
        ));
        assert!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::DeleteMessage, &[0x09, 1]).is_err()
        );
    }

    #[test]
    fn file_message_from_android() {
        // Shape of an image message sent by the Android app.
        let json = br#"{"j":1,"i":1,"k":"0101010101010101010101010101010101010101010101010101010101010101","b":"00112233445566778899aabbccddeeff","m":"image/jpeg","n":"IMG_1.jpg","s":12345,"t":"ffeeddccbbaa99887766554433221100","p":"image/jpeg","x":{"w":800,"h":600},"unknown":true}"#;
        let IncomingMessageBody::Contact(ContactMessageBody::File(file)) =
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::File, json).unwrap()
        else {
            panic!("expected a file message")
        };
        assert_eq!(file.rendering_type, FileRenderingType::Media);
        assert_eq!(file.key, [1; 32]);
        assert_eq!(file.blob_id.0[15], 0xff);
        assert_eq!(file.size, 12345);
        assert_eq!(file.name.as_deref(), Some("IMG_1.jpg"));
        assert_eq!(file.thumbnail_blob_id.unwrap().0[0], 0xff);
        assert_eq!(file.metadata.width, Some(800));
        assert_eq!(file.caption, None);

        // Round trip.
        let mut writer = OwnedVecByteWriter::new_empty();
        file.encode_into(&mut writer).unwrap();
        let IncomingMessageBody::Contact(ContactMessageBody::File(again)) =
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::File, &writer.into_inner())
                .unwrap()
        else {
            panic!("expected a file message")
        };
        assert_eq!(again, file);
    }

    #[test]
    fn invalid_file_messages_are_rejected() {
        let decode =
            |json: &str| IncomingMessageBody::decode(test_sender(), CspE2eMessageType::File, json.as_bytes());
        // Fallback to the deprecated rendering type, no thumbnail.
        assert!(decode(r#"{"i":0,"k":"0101010101010101010101010101010101010101010101010101010101010101","b":"00112233445566778899aabbccddeeff","m":"application/pdf","s":1}"#).is_ok());
        // Short key, missing blob id, long name.
        assert!(decode(r#"{"k":"01","b":"00112233445566778899aabbccddeeff","m":"a","s":1}"#).is_err());
        assert!(
            decode(
                r#"{"k":"0101010101010101010101010101010101010101010101010101010101010101","m":"a","s":1}"#
            )
            .is_err()
        );
        // No size, or one that isn't a number.
        let key = "0101010101010101010101010101010101010101010101010101010101010101";
        let blob = "00112233445566778899aabbccddeeff";
        assert!(decode(&format!(r#"{{"k":"{key}","b":"{blob}","m":"a"}}"#)).is_err());
        assert!(decode(&format!(r#"{{"k":"{key}","b":"{blob}","m":"a","s":"big"}}"#)).is_err());
    }

    #[test]
    fn deprecated_media_come_in_as_file_messages() {
        let blob = [0xb1; 16];
        let thumbnail = [0xb2; 16];
        let key = [0x4b; 32];
        let nonce = [0x4e; 24];
        let group_header = [b"CREATOR1".as_slice(), &7_u64.to_le_bytes()].concat();
        let contact = |message_type, data: &[u8]| {
            let IncomingMessageBody::Contact(ContactMessageBody::File(file)) =
                IncomingMessageBody::decode(test_sender(), message_type, data).unwrap()
            else {
                panic!("expected a file message")
            };
            file
        };
        let group = |message_type, data: &[u8]| {
            let data = [group_header.as_slice(), data].concat();
            let IncomingMessageBody::Group(IncomingGroupMessageBody {
                body: GroupMessageBody::File(file),
                ..
            }) = IncomingMessageBody::decode(test_sender(), message_type, &data).unwrap()
            else {
                panic!("expected a group file message")
            };
            file
        };

        // Image: blob, size, nonce. Sealed between us, so without a key of its own.
        let image = [blob.as_slice(), &1000_u32.to_le_bytes(), &nonce].concat();
        let file = contact(CspE2eMessageType::DeprecatedImage, &image);
        assert_eq!(file.rendering_type, FileRenderingType::Media);
        assert_eq!(file.media_type, "image/jpeg");
        assert_eq!(file.blob_id.0, blob);
        assert_eq!(file.size, 1000);
        assert_eq!(file.metadata.legacy_nonce, Some(nonce));

        // Group image: blob, size, key.
        let group_image = [blob.as_slice(), &1000_u32.to_le_bytes(), &key].concat();
        let file = group(CspE2eMessageType::GroupImage, &group_image);
        assert_eq!(file.media_type, "image/jpeg");
        assert_eq!((file.key, file.size), (key, 1000));
        assert_eq!(file.metadata.legacy_nonce, None);

        // Audio: duration, blob, size, key.
        let audio = [
            &30_u16.to_le_bytes(),
            blob.as_slice(),
            &2000_u32.to_le_bytes(),
            &key,
        ]
        .concat();
        for file in [
            contact(CspE2eMessageType::DeprecatedAudio, &audio),
            group(CspE2eMessageType::GroupAudio, &audio),
        ] {
            assert_eq!(file.media_type, "audio/aac");
            assert_eq!((file.key, file.size), (key, 2000));
            assert_eq!(file.metadata.duration, Some(30.0));
            assert_eq!(file.blob_id.0, blob);
        }

        // Video: duration, blob, size, thumbnail, its size, key.
        let video = [
            &60_u16.to_le_bytes(),
            blob.as_slice(),
            &3000_u32.to_le_bytes(),
            &thumbnail,
            &40_u32.to_le_bytes(),
            &key,
        ]
        .concat();
        for file in [
            contact(CspE2eMessageType::DeprecatedVideo, &video),
            group(CspE2eMessageType::GroupVideo, &video),
        ] {
            assert_eq!(file.media_type, "video/mpeg");
            assert_eq!((file.key, file.size), (key, 3000));
            assert_eq!(file.metadata.duration, Some(60.0));
            assert_eq!(file.thumbnail_blob_id.unwrap().0, thumbnail);
            assert_eq!(file.thumbnail_media_type.as_deref(), Some("image/jpeg"));
        }

        // Too short is still wrong, and more than there should be is ignored as on Android.
        assert!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::DeprecatedImage, &image[..39])
                .is_err()
        );
        let longer = [image.as_slice(), &[0_u8; 3]].concat();
        assert_eq!(contact(CspE2eMessageType::DeprecatedImage, &longer).size, 1000);
    }

    #[test]
    fn sloppy_file_messages_are_kept() {
        let decode = |fields: &str| {
            let json = format!(
                r#"{{"k":"0101010101010101010101010101010101010101010101010101010101010101","b":"00112233445566778899aabbccddeeff","m":"image/jpeg",{fields}}}"#
            );
            let IncomingMessageBody::Contact(ContactMessageBody::File(file)) =
                IncomingMessageBody::decode(test_sender(), CspE2eMessageType::File, json.as_bytes()).unwrap()
            else {
                panic!("expected a file message")
            };
            file
        };
        // Metadata of the wrong type counts as absent, field by field or as a whole.
        let file = decode(r#""s":1,"x":{"w":800.5,"h":"600","a":"yes","d":"long"}"#);
        assert_eq!(file.metadata, FileMetadata::default());
        let file = decode(r#""s":1,"x":{"w":800,"h":-1,"d":3}"#);
        assert_eq!(file.metadata.width, Some(800));
        assert_eq!(file.metadata.height, None);
        assert_eq!(file.metadata.duration, Some(3.0));
        assert_eq!(decode(r#""s":1,"x":"none""#).metadata, FileMetadata::default());
        // A float size, a negative one, or one in a string.
        assert_eq!(decode(r#""s":12345.0"#).size, 12345);
        assert_eq!(decode(r#""s":-1"#).size, 0);
        assert_eq!(decode(r#""s":"123""#).size, 123);
        assert_eq!(decode(r#""s":"12.5""#).size, 12);
        // An unassigned rendering type is a plain file, and not falling back to `i`.
        for rendering in [r#""j":300,"i":1"#, r#""j":-1,"i":1"#, r#""j":3,"i":1"#] {
            let file = decode(&format!(r#""s":1,{rendering}"#));
            assert_eq!(file.rendering_type, FileRenderingType::File, "{rendering}");
        }
        assert_eq!(
            decode(r#""s":1,"j":1.0"#).rendering_type,
            FileRenderingType::Media
        );
        // Not a number: as if not given.
        assert_eq!(
            decode(r#""s":1,"j":"x","i":2"#).rendering_type,
            FileRenderingType::Sticker
        );
        // A media type that isn't one means none.
        let json = br#"{"k":"0101010101010101010101010101010101010101010101010101010101010101","b":"00112233445566778899aabbccddeeff","m":5,"s":1}"#;
        let IncomingMessageBody::Contact(ContactMessageBody::File(file)) =
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::File, json).unwrap()
        else {
            panic!("expected a file message")
        };
        assert_eq!(file.media_type, "");
        // Texts of the wrong type are gone, and too long ones stay: the limits are for sending.
        let file = decode(r#""s":1,"n":5,"d":["x"],"p":1,"c":{}"#);
        assert_eq!(file.name, None);
        assert_eq!(file.caption, None);
        assert_eq!(file.thumbnail_media_type, None);
        assert_eq!(file.correlation_id, None);
        let long = "ä".repeat(3500);
        let file = decode(&format!(r#""s":1,"n":"{long}","d":"{long}""#));
        assert_eq!(file.name.as_deref(), Some(long.as_str()));
        assert_eq!(file.caption.as_deref(), Some(long.as_str()));
    }

    fn group_roundtrip(sender: &str, body: GroupMessageBody) -> (Vec<u8>, IncomingMessageBody) {
        let creator = ThreemaId::try_from("CREATOR1").unwrap();
        let outgoing = OutgoingGroupMessageBody {
            group_identity: GroupIdentity {
                group_id: 0x0102_0304_0506_0708,
                creator_identity: creator,
            },
            body,
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        outgoing.encode_into(&mut writer).unwrap();
        let data = writer.into_inner();
        let message_type = outgoing.body.message_type();
        let decoded =
            IncomingMessageBody::decode(ThreemaId::try_from(sender).unwrap(), message_type, &data).unwrap();
        (data, decoded)
    }

    #[test]
    fn group_containers() {
        // Creator messages: group id only, the sender is the creator.
        let members = vec![
            ThreemaId::try_from("ECHOECHO").unwrap(),
            ThreemaId::try_from("00000BOB").unwrap(),
        ];
        let (data, decoded) = group_roundtrip(
            "CREATOR1",
            GroupMessageBody::Setup(GroupSetupMessage {
                members: members.clone(),
            }),
        );
        assert_eq!(data.len(), 8 + 16);
        assert_eq!(&data[..8], &0x0102_0304_0506_0708_u64.to_le_bytes());
        let IncomingMessageBody::Group(group) = decoded else {
            panic!("expected a group message")
        };
        assert_eq!(group.group_identity.creator_identity.as_str(), "CREATOR1");
        assert_eq!(group.body, GroupMessageBody::Setup(GroupSetupMessage { members }));

        let (_, decoded) = group_roundtrip(
            "CREATOR1",
            GroupMessageBody::Name(GroupNameMessage {
                name: "Sailors ⛵".to_owned(),
            }),
        );
        assert!(matches!(
            decoded,
            IncomingMessageBody::Group(IncomingGroupMessageBody { body: GroupMessageBody::Name(ref n), .. })
                if n.name == "Sailors ⛵"
        ));

        // Member messages: creator identity + group id, from any member.
        let (data, decoded) = group_roundtrip(
            "00000BOB",
            GroupMessageBody::Text(TextMessage {
                text: "hi group".to_owned(),
            }),
        );
        assert_eq!(&data[..8], b"CREATOR1");
        let IncomingMessageBody::Group(group) = decoded else {
            panic!("expected a group message")
        };
        assert_eq!(group.group_identity.creator_identity.as_str(), "CREATOR1");
        assert_eq!(group.group_identity.group_id, 0x0102_0304_0506_0708);

        for body in [
            GroupMessageBody::Leave(GroupLeaveMessage),
            GroupMessageBody::SyncRequest(GroupSyncRequestMessage),
            GroupMessageBody::Delete(DeleteMessage {
                message_id: MessageId(7),
            }),
        ] {
            let (_, decoded) = group_roundtrip("00000BOB", body.clone());
            let IncomingMessageBody::Group(group) = decoded else {
                panic!("expected a group message")
            };
            assert_eq!(group.body, body);
        }
        // Misaligned setup.
        assert!(
            IncomingMessageBody::decode(
                ThreemaId::try_from("CREATOR1").unwrap(),
                CspE2eMessageType::GroupSetup,
                &[1, 2, 3, 4, 5, 6, 7, 8, b'A']
            )
            .is_err()
        );
    }

    #[test]
    fn polls() {
        let setup = PollSetupMessage {
            poll_id: 0x1122_3344_5566_7788,
            poll: r#"{"d":"Lunch?","s":0,"a":0,"t":1,"o":0,"c":[{"i":0,"n":"Pizza","o":0}]}"#.to_owned(),
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        setup.encode_into(&mut writer).unwrap();
        let data = writer.into_inner();
        assert_eq!(&data[..8], &0x1122_3344_5566_7788_u64.to_le_bytes());
        assert!(matches!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::PollSetup, &data).unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::PollSetup(decoded)) if decoded == setup
        ));
        let vote = GroupMessageBody::PollVote(PollVoteMessage {
            creator_identity: ThreemaId::try_from("00000BOB").unwrap(),
            poll_id: 7,
            choices: "[[0,1],[1,0]]".to_owned(),
        });
        let (data, decoded) = group_roundtrip("ECHOECHO", vote.clone());
        assert_eq!(&data[16..24], b"00000BOB");
        let IncomingMessageBody::Group(group) = decoded else {
            panic!("expected a group message")
        };
        assert_eq!(group.body, vote);
    }

    #[test]
    fn contact_profile_pictures() {
        let set = SetProfilePictureMessage {
            blob_id: BlobId([7; 16]),
            size: 1234,
            key: [9; 32],
        };
        let mut writer = OwnedVecByteWriter::new_empty();
        set.encode_into(&mut writer).unwrap();
        let data = writer.into_inner();
        assert_eq!(data.len(), 16 + 4 + 32);
        assert!(matches!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::ContactSetProfilePicture, &data).unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::SetProfilePicture(decoded)) if decoded == set
        ));
        assert!(matches!(
            IncomingMessageBody::decode(
                test_sender(),
                CspE2eMessageType::ContactRequestProfilePicture,
                &[]
            )
            .unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::RequestProfilePicture(_))
        ));
        assert!(matches!(
            IncomingMessageBody::decode(test_sender(), CspE2eMessageType::ContactDeleteProfilePicture, &[])
                .unwrap(),
            IncomingMessageBody::Contact(ContactMessageBody::DeleteProfilePicture(_))
        ));
    }
}
