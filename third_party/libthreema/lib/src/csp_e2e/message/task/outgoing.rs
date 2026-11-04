//! Task for sending an outgoing message.

use aead::AeadMutInPlace as _;

use rand::Rng as _;

use crate::{
    common::{
        Delta, MessageFlags, MessageId, MessageMetadata, Nonce, ThreemaId,
        keys::{ClientKey, CspE2eKey, PublicKey},
    },
    csp::payload::MessageWithMetadataBox,
    csp_e2e::message::{codec::CspMessageEncoder as _, payload::OutgoingMessageWithMetadataBox},
    model::message::{
        ContactMessageBody, MessageProperties, OutgoingContactMessageBody, OutgoingMessage,
        OutgoingMessageBody,
    },
    protobuf::common::CspE2eMessageType,
    utils::bytes::OwnedVecByteWriter,
};

/// Minimum length of `data || padding` in a message container (same as the codec).
const MESSAGE_DATA_PADDING_LENGTH_MIN: usize = 32;

/// Error when encoding an outgoing message.
#[derive(Debug, thiserror::Error)]
pub enum OutgoingMessageEncodeError {
    /// The receiver's public key is non-contributory.
    #[error("Invalid receiver public key")]
    InvalidReceiverKey,

    /// Encoding the message failed (e.g. metadata or message too large).
    #[error("Encoding message failed: {0}")]
    EncodingFailed(String),
}

/// The legacy nickname and the metadata's delta for a sender's nickname.
fn sender_nicknames(sender_nickname: Delta<&str>) -> (Option<&str>, Delta<&str>) {
    match sender_nickname {
        Delta::Update("") => (None, Delta::Remove),
        Delta::Update(nickname) => (Some(nickname), sender_nickname),
        other => (None, other),
    }
}

/// Encode and encrypt an outgoing 1:1 message into a CSP `message-with-metadata-box` payload.
///
/// Stop-gap until the outgoing message task exists (TODO(LIB-51)): the caller is responsible for
/// choosing a fresh `nonce` (and recording it as used), for sending the payload and for awaiting
/// the server's message-ack. `sender_nickname` is sent in the (encrypted) metadata and, towards
/// Gateway IDs, as the legacy nickname. [`Delta::Remove`] tells the receiver to drop the name it
/// knows, [`Delta::Unchanged`] leaves it alone. An empty name counts as a removal.
///
/// # Errors
///
/// Returns [`OutgoingMessageEncodeError`] if the receiver key is non-contributory or encoding
/// fails.
pub fn encode_outgoing_contact_message(
    sender_identity: ThreemaId,
    client_key: &ClientKey,
    sender_nickname: Delta<&str>,
    receiver_identity: ThreemaId,
    receiver_public_key: &PublicKey,
    message: &OutgoingMessage,
    nonce: Nonce,
) -> Result<MessageWithMetadataBox, OutgoingMessageEncodeError> {
    let shared_secret = client_key
        .derive_csp_e2e_key(receiver_public_key)
        .ok_or(OutgoingMessageEncodeError::InvalidReceiverKey)?;
    let encoded = encode_and_encrypt_message(
        sender_identity,
        sender_nicknames(sender_nickname),
        receiver_identity,
        shared_secret,
        message,
        nonce,
    );
    MessageWithMetadataBox::try_from(encoded)
        .map_err(|error| OutgoingMessageEncodeError::EncodingFailed(error.to_string()))
}

/// Encode a 1:1 message body to its type and data (no padding), e.g. as the inner message of a
/// forward security envelope ([`crate::csp_e2e::fs::encapsulate`]).
#[must_use]
pub fn encode_contact_message_body(receiver_identity: ThreemaId, body: &ContactMessageBody) -> (u8, Vec<u8>) {
    encode_message_body(&OutgoingMessageBody::Contact(OutgoingContactMessageBody {
        receiver_identity,
        body: body.clone(),
    }))
}

/// Encode any outgoing message body (1:1 or group, with its group container) to its type and data
/// (no padding), e.g. as the inner message of a forward security envelope.
///
/// # Panics
///
/// Never: encoding into a growable buffer cannot fail and always yields a type and padding.
#[must_use]
pub fn encode_message_body(body: &OutgoingMessageBody) -> (u8, Vec<u8>) {
    // Reuse the regular container encoding, then strip the type byte and the PKCS#7 padding.
    let mut writer = OwnedVecByteWriter::new_empty();
    body.encode_into(&mut writer)
        .expect("encoding a message body with OwnedVecByteWriter should not fail");
    let container = writer.into_inner();
    let padding = usize::from(*container.last().expect("container has a type and padding"));
    let data_end = container.len().saturating_sub(padding).max(1);
    (
        *container.first().expect("container has a type"),
        container.get(1..data_end).unwrap_or_default().to_vec(),
    )
}

/// Like [`encode_outgoing_contact_message`], but the message container carries the forward
/// security `envelope` (type `0xa0`) instead of the message body. ID, created-at, flags and
/// nickname are those of `message`, as required for encapsulated messages.
///
/// # Errors
///
/// See [`encode_outgoing_contact_message`].
pub fn encode_outgoing_contact_message_encapsulated(
    sender_identity: ThreemaId,
    client_key: &ClientKey,
    sender_nickname: Delta<&str>,
    receiver_identity: ThreemaId,
    receiver_public_key: &PublicKey,
    message: &OutgoingMessage,
    envelope: &[u8],
    nonce: Nonce,
) -> Result<MessageWithMetadataBox, OutgoingMessageEncodeError> {
    let shared_secret = client_key
        .derive_csp_e2e_key(receiver_public_key)
        .ok_or(OutgoingMessageEncodeError::InvalidReceiverKey)?;
    let encoded = encode_and_encrypt_message_with_container(
        sender_identity,
        sender_nicknames(sender_nickname),
        receiver_identity,
        shared_secret,
        message,
        Some((CspE2eMessageType::ForwardSecurityEnvelope as u8, envelope)),
        nonce,
    );
    MessageWithMetadataBox::try_from(encoded)
        .map_err(|error| OutgoingMessageEncodeError::EncodingFailed(error.to_string()))
}

/// Encode a forward security control message (Init, Accept, Reject, Terminate) for `receiver`:
/// type `0xa0`, a new random message ID, no flags and no nickname.
///
/// # Errors
///
/// See [`encode_outgoing_contact_message`].
pub fn encode_outgoing_fs_control_message(
    sender_identity: ThreemaId,
    client_key: &ClientKey,
    receiver_identity: ThreemaId,
    receiver_public_key: &PublicKey,
    envelope: &[u8],
    created_at: u64,
    nonce: Nonce,
) -> Result<MessageWithMetadataBox, OutgoingMessageEncodeError> {
    let shared_secret = client_key
        .derive_csp_e2e_key(receiver_public_key)
        .ok_or(OutgoingMessageEncodeError::InvalidReceiverKey)?;
    let id = MessageId::random();
    let mut metadata: Vec<u8> = MessageMetadata {
        message_id: id,
        created_at,
        nickname: Delta::Unchanged,
    }
    .into();
    encrypt_metadata_in_place(&shared_secret, &nonce, &mut metadata);
    let mut message_container = padded_container(CspE2eMessageType::ForwardSecurityEnvelope as u8, envelope);
    encrypt_message_container_in_place(&shared_secret, &nonce, &mut message_container);
    drop(shared_secret);
    MessageWithMetadataBox::try_from(OutgoingMessageWithMetadataBox {
        sender_identity,
        receiver_identity,
        id,
        legacy_created_at: created_at
            .checked_div_euclid(1000)
            .expect("ms to s conversion should work")
            .try_into()
            .unwrap_or(u32::MAX),
        flags: MessageFlags::default(),
        legacy_sender_nickname: None,
        metadata,
        nonce,
        message_container,
    })
    .map_err(|error| OutgoingMessageEncodeError::EncodingFailed(error.to_string()))
}

/// `type || data || PKCS#7 padding` (same padding rules as [`OutgoingMessageBody`] encoding).
fn padded_container(message_type: u8, data: &[u8]) -> Vec<u8> {
    let padding_length: u8 = rand::thread_rng().gen_range(1..=255);
    let padding_length =
        if data.len().saturating_add(padding_length as usize) < MESSAGE_DATA_PADDING_LENGTH_MIN {
            u8::try_from(MESSAGE_DATA_PADDING_LENGTH_MIN.saturating_sub(data.len())).unwrap_or(u8::MAX)
        } else {
            padding_length
        };
    let mut container = Vec::with_capacity(
        data.len()
            .saturating_add(1)
            .saturating_add(padding_length as usize),
    );
    container.push(message_type);
    container.extend_from_slice(data);
    container.resize(
        container.len().saturating_add(padding_length as usize),
        padding_length,
    );
    container
}

pub(super) fn encode_metadata(
    sender_nickname: Delta<&str>,
    message: &OutgoingMessage,
    effective_message_properties: &MessageProperties,
) -> Vec<u8> {
    MessageMetadata {
        message_id: message.id,
        created_at: message.created_at,
        nickname: if effective_message_properties.user_profile_distribution {
            sender_nickname.map(ToOwned::to_owned)
        } else {
            Delta::Unchanged
        },
    }
    .into()
}

// TODO(LIB-51): Make encryption fallible
pub(super) fn encrypt_metadata_in_place(shared_secret: &CspE2eKey, nonce: &Nonce, metadata: &mut Vec<u8>) {
    shared_secret
        .message_metadata_cipher()
        .0
        .encrypt_in_place(&nonce.0.into(), b"", metadata)
        .expect("metadata encryption should not fail");
}

// TODO(LIB-51): Make encryption fallible
pub(super) fn encrypt_message_container_in_place(
    shared_secret: &CspE2eKey,
    nonce: &Nonce,
    message_container: &mut Vec<u8>,
) {
    shared_secret
        .message_cipher()
        .0
        .encrypt_in_place(&nonce.0.into(), b"", message_container)
        .expect("message container encryption should not fail");
}

pub(super) fn encode_and_encrypt_message(
    sender_identity: ThreemaId,
    nicknames: (Option<&str>, Delta<&str>),
    receiver_identity: ThreemaId,
    shared_secret: CspE2eKey,
    message: &OutgoingMessage,
    nonce: Nonce,
) -> OutgoingMessageWithMetadataBox {
    encode_and_encrypt_message_with_container(
        sender_identity,
        nicknames,
        receiver_identity,
        shared_secret,
        message,
        None,
        nonce,
    )
}

/// `container`: send this (type, data) instead of the message body (forward security envelope).
fn encode_and_encrypt_message_with_container(
    sender_identity: ThreemaId,
    (legacy_sender_nickname, sender_nickname): (Option<&str>, Delta<&str>),
    receiver_identity: ThreemaId,
    shared_secret: CspE2eKey,
    message: &OutgoingMessage,
    container: Option<(u8, &[u8])>,
    nonce: Nonce,
) -> OutgoingMessageWithMetadataBox {
    let effective_message_properties = message.effective_properties();

    // Encode and encrypt metadata
    let metadata = {
        let mut metadata = encode_metadata(sender_nickname, message, &effective_message_properties);
        encrypt_metadata_in_place(&shared_secret, &nonce, &mut metadata);
        metadata
    };

    // Encode and encrypt message container
    let message_container = if let Some((message_type, data)) = container {
        let mut message_container = padded_container(message_type, data);
        encrypt_message_container_in_place(&shared_secret, &nonce, &mut message_container);
        message_container
    } else {
        // Note: The effort to calculate the required capacity for all message types would be unreasonable, so
        // we don't.
        let mut writer = OwnedVecByteWriter::new_empty();

        // Encode message container (i.e. message type, data and PKCS#7 padding)
        message
            .body
            .encode_into(&mut writer)
            .expect("encoding message container with OwnedVecByteWriter should not fail");
        let mut message_container = writer.into_inner();

        // Encrypt message container
        encrypt_message_container_in_place(&shared_secret, &nonce, &mut message_container);
        message_container
    };

    // Drop shared secret ASAP
    drop(shared_secret);

    // Encode legacy created-at timestamp
    let legacy_created_at: u32 = message
        .created_at
        .checked_div_euclid(1000)
        .expect("ms to s conversion should work")
        .try_into()
        .unwrap_or(u32::MAX);

    // Encode legacy sender nickname (if applicable and towards a Gateway ID)
    let legacy_sender_nickname =
        if effective_message_properties.user_profile_distribution && receiver_identity.is_gateway_id() {
            legacy_sender_nickname.map(ToOwned::to_owned)
        } else {
            None
        };

    // Done
    OutgoingMessageWithMetadataBox {
        sender_identity,
        receiver_identity,
        id: message.id,
        legacy_created_at,
        flags: message.flags(),
        legacy_sender_nickname,
        metadata,
        nonce,
        message_container,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        common::MessageId,
        model::message::{
            ContactMessageBody, MessageOverrides, OutgoingContactMessageBody, OutgoingMessageBody,
            TextMessage,
        },
    };

    fn text_message(receiver: ThreemaId) -> OutgoingMessage {
        OutgoingMessage {
            id: MessageId(0x0102_0304_0506_0708),
            overrides: MessageOverrides::default(),
            created_at: 1_700_000_000_000,
            body: OutgoingMessageBody::Contact(OutgoingContactMessageBody {
                receiver_identity: receiver,
                body: ContactMessageBody::Text(TextMessage {
                    text: "hello".to_owned(),
                }),
            }),
        }
    }

    #[test]
    fn encodes_contact_message_payload() {
        let sender = ThreemaId::try_from("AAAAAAAA").unwrap();
        let receiver = ThreemaId::try_from("ECHOECHO").unwrap();
        let receiver_key = ClientKey::from([2; ClientKey::LENGTH]).public_key();
        let payload = encode_outgoing_contact_message(
            sender,
            &ClientKey::from([1; ClientKey::LENGTH]),
            Delta::Update("Tester"),
            receiver,
            &receiver_key,
            &text_message(receiver),
            Nonce([3; Nonce::LENGTH]),
        )
        .unwrap();
        assert_eq!(payload.sender_identity, sender);
        assert_eq!(payload.id, MessageId(0x0102_0304_0506_0708));
        // The plaintext must not appear in the encoded payload.
        assert!(!payload.bytes.windows(5).any(|window| window == b"hello"));
    }

    #[test]
    fn contact_message_body_roundtrip_without_padding() {
        use crate::{
            csp_e2e::message::codec::CspMessageDecoder as _,
            model::message::{IncomingMessageBody, LocationMessage},
        };

        let receiver = ThreemaId::try_from("ECHOECHO").unwrap();
        let (message_type, data) = encode_contact_message_body(
            receiver,
            &ContactMessageBody::Text(TextMessage {
                text: "hello".to_owned(),
            }),
        );
        assert_eq!(message_type, 0x01);
        assert_eq!(data, b"hello");
        let decoded = IncomingMessageBody::decode(
            ThreemaId::try_from("ECHOECHO").unwrap(),
            CspE2eMessageType::Text,
            &data,
        )
        .unwrap();
        assert!(matches!(
            decoded,
            IncomingMessageBody::Contact(ContactMessageBody::Text(TextMessage { text })) if text == "hello"
        ));

        let (message_type, data) = encode_contact_message_body(
            receiver,
            &ContactMessageBody::Location(LocationMessage {
                latitude: 47.0,
                longitude: 8.0,
                accuracy_m: None,
                name: None,
                address: None,
            }),
        );
        assert_eq!(message_type, 0x10);
        assert!(
            IncomingMessageBody::decode(
                ThreemaId::try_from("ECHOECHO").unwrap(),
                CspE2eMessageType::Location,
                &data
            )
            .is_ok()
        );
    }

    /// What the receiver reads about the sender's nickname: nothing leaves its name alone, and an
    /// empty one makes it drop the name.
    #[test]
    fn nickname_delta_reaches_the_metadata() {
        use prost::Message as _;

        fn nickname_in_metadata(nickname: Delta<&str>) -> (Option<String>, Option<&str>) {
            let (legacy, nickname) = sender_nicknames(nickname);
            let message = text_message(ThreemaId::try_from("ECHOECHO").unwrap());
            let metadata = encode_metadata(nickname, &message, &message.effective_properties());
            (
                crate::protobuf::csp_e2e::MessageMetadata::decode(metadata.as_slice())
                    .unwrap()
                    .nickname,
                legacy,
            )
        }

        assert_eq!(
            nickname_in_metadata(Delta::Update("Tester")),
            (Some("Tester".to_owned()), Some("Tester"))
        );
        assert_eq!(nickname_in_metadata(Delta::Remove), (Some(String::new()), None));
        assert_eq!(
            nickname_in_metadata(Delta::Update("")),
            (Some(String::new()), None)
        );
        assert_eq!(nickname_in_metadata(Delta::Unchanged), (None, None));
    }

    #[test]
    fn rejects_non_contributory_receiver_key() {
        let receiver = ThreemaId::try_from("ECHOECHO").unwrap();
        let result = encode_outgoing_contact_message(
            ThreemaId::try_from("AAAAAAAA").unwrap(),
            &ClientKey::from([1; ClientKey::LENGTH]),
            Delta::Remove,
            receiver,
            &PublicKey::from([0; PublicKey::LENGTH]),
            &text_message(receiver),
            Nonce([3; Nonce::LENGTH]),
        );
        assert!(matches!(
            result,
            Err(OutgoingMessageEncodeError::InvalidReceiverKey)
        ));
    }
}
