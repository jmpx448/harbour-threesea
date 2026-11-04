//! Message structs.
use core::{fmt, str};

use duplicate::duplicate_item;
use educe::Educe;
use libthreema_macros::Name;

use crate::{
    common::{BlobId, GroupIdentity, MessageFlags, MessageId, ThreemaId},
    protobuf::common::CspE2eMessageType,
    utils::{
        apply::Apply,
        debug::{Name as _, debug_slice_length},
    },
};

/// Lifetime of the message on the server.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum MessageLifetime {
    /// The message is kept indefinitely until received.
    Indefinite,

    /// The message is kept for a short amount of time until it is silently dropped (usually 30s).
    Brief,

    /// The message is only transmitted if the receiver is currently online.
    ///
    /// This is a combination of the _no server queuing_ and _no server acknowledgement_ flags
    /// which are only used in conjunction.
    Ephemeral,
}

/// Message properties associated to a message type.
#[expect(
    clippy::struct_excessive_bools,
    reason = "The one place where it's reasonable"
)]
#[derive(Debug, Clone, Name)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct MessageProperties {
    /// Whether the message should be/has been pushed.
    pub(crate) push: bool,

    /// Lifetime of the message on the server.
    pub(crate) lifetime: MessageLifetime,

    /// Whether the message requires user profile distribution.
    pub(crate) user_profile_distribution: bool,

    /// Whether the message requires to create a _direct_ contact upon reception.
    pub(crate) requires_direct_contact: bool,

    /// Whether the message requires replay protection (by storing the nonce).
    pub(crate) replay_protection: bool,

    /// Whether the message should be reflected in case it is incoming.
    pub(crate) reflect_incoming: bool,

    /// Whether the message should be reflected in case it is outgoing.
    #[expect(dead_code, reason = "Will use later")]
    pub(crate) reflect_outgoing: bool,

    /// Whether an update that the message has been _sent_ should be reflected (in case it is
    /// outgoing).
    #[expect(dead_code, reason = "Will use later")]
    pub(crate) reflect_sent_update: bool,

    /// Whether delivery receipts may be sent in response to this message.
    pub(crate) delivery_receipts: bool,
}
impl Apply<MessageProperties> for MessageFlags {
    fn apply(&mut self, value: MessageProperties) {
        if value.push {
            self.0 |= MessageFlags::SEND_PUSH_NOTIFICATION;
        }
        match value.lifetime {
            MessageLifetime::Indefinite => {},
            MessageLifetime::Brief => {
                self.0 |= MessageFlags::SHORT_LIVED_SERVER_QUEUING;
            },
            MessageLifetime::Ephemeral => {
                self.0 |= MessageFlags::NO_SERVER_QUEUING | MessageFlags::NO_SERVER_ACKNOWLEDGEMENT;
            },
        }
    }
}

/// Message properties associated to a group message type.
///
/// This is a reduced version of [`MessageProperties`].
#[expect(
    clippy::struct_excessive_bools,
    reason = "The one place where it's reasonable"
)]
struct GroupMessageProperties {
    push: bool,
    lifetime: MessageLifetime,
    user_profile_distribution: bool,
    replay_protection: bool,
    reflect_incoming: bool,
    reflect_outgoing: bool,
    reflect_sent_update: bool,
}
impl GroupMessageProperties {
    const fn into_message_properties(self) -> MessageProperties {
        MessageProperties {
            push: self.push,
            lifetime: self.lifetime,
            user_profile_distribution: self.user_profile_distribution,
            requires_direct_contact: false,
            replay_protection: self.replay_protection,
            reflect_incoming: self.reflect_incoming,
            reflect_outgoing: self.reflect_outgoing,
            reflect_sent_update: self.reflect_sent_update,
            delivery_receipts: false,
        }
    }
}

/// Message behaviour overrides that need to be stored.
#[derive(Clone, Copy, Default, Name)]
#[cfg_attr(test, derive(PartialEq))]
pub struct MessageOverrides {
    /// Whether delivery receipts should be omitted. If `true`, this overrides the default behaviour.
    pub disable_delivery_receipts: bool,
}
impl Apply<MessageOverrides> for MessageProperties {
    fn apply(&mut self, value: MessageOverrides) {
        if value.disable_delivery_receipts {
            self.delivery_receipts = false;
        }
    }
}
impl Apply<MessageOverrides> for MessageFlags {
    fn apply(&mut self, value: MessageOverrides) {
        if value.disable_delivery_receipts {
            self.0 |= MessageFlags::NO_DELIVERY_RECEIPTS;
        }
    }
}
impl fmt::Debug for MessageOverrides {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Keep this format in sync with [`MessageFlags`]!
        write!(
            formatter,
            "{}({})",
            Self::NAME,
            itertools::join(
                [self.disable_delivery_receipts.then_some("no-receipts")]
                    .into_iter()
                    .flatten(),
                ", ",
            ),
        )
    }
}
impl From<MessageFlags> for MessageOverrides {
    fn from(flags: MessageFlags) -> Self {
        Self {
            disable_delivery_receipts: flags.0 & MessageFlags::NO_DELIVERY_RECEIPTS != 0,
        }
    }
}

/// A text message.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct TextMessage {
    /// Text of the message.
    #[educe(Debug(method(debug_slice_length)))]
    pub text: String,
}
impl TextMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        requires_direct_contact: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: true,
        delivery_receipts: true,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::Text;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: true,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupText;
}

/// A location message.
#[derive(Clone, Name)]
#[cfg_attr(test, derive(PartialEq))]
pub struct LocationMessage {
    /// Latitude.
    pub latitude: f64,

    /// Longitude.
    pub longitude: f64,

    /// Accuracy of the original sender's position in meters.
    pub accuracy_m: Option<f64>,

    /// Closest name of the location or a point of interest.
    pub name: Option<String>,

    /// An arbitrary address, not following any standardised pattern. May contain line breaks.
    pub address: Option<String>,
}
impl LocationMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        requires_direct_contact: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: true,
        delivery_receipts: true,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::Location;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: true,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupLocation;
}
impl fmt::Debug for LocationMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(Self::NAME)
    }
}

/// The type of delivery receipt.
#[derive(Debug, Clone, Copy, strum::FromRepr)]
#[repr(u8)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DeliveryReceiptType {
    /// The referred messages have been received (i.e. it was delivered to the recipient).
    Received = 0x01,
    /// The referred messages have been read.
    Read = 0x02,
}

/// The type of legacy reaction.
#[derive(Debug, Clone, Copy, strum::FromRepr)]
#[repr(u8)]
#[cfg_attr(test, derive(PartialEq))]
pub enum LegacyReactionType {
    /// The user reacted with _acknowledge_ (i.e. _thumbs up_) to the messages.
    Acknowledge = 0x03,
    /// The user reacted with _decline_ (i.e. _thumbs down_) to the messages.
    Decline = 0x04,
}

/// A delivery receipt.
///
/// Note: This only covers real delivery receipts, not legacy reactions. These are always mapped to and from
/// [`LegacyReactionMessage`].
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct DeliveryReceiptMessage {
    /// Type of delivery receipt.
    pub receipt_type: DeliveryReceiptType,

    /// Message ids of referred messages the receipt type should be applied to.
    pub message_ids: Vec<MessageId>,
}
impl DeliveryReceiptMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: false,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::DeliveryReceipt;
}

/// A legacy reaction message.
///
/// Note: Delivery receipts containing legacy reactions are always mapped to and from this message internally.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct LegacyReactionMessage {
    /// Type of legacy reaction.
    pub reaction_type: LegacyReactionType,

    /// Message ids of referred messages the reaction should be applied to.
    pub message_ids: Vec<MessageId>,
}
impl LegacyReactionMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::DeliveryReceipt;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupDeliveryReceipt;
}

/// A control message from Threema Web, requesting a session to be resumed.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct WebSessionResumeMessage {
    /// Raw `push-payload` of a `web-session-resume` message.
    #[educe(Debug(method(debug_slice_length)))]
    pub push_payload: Vec<u8>,
}
impl WebSessionResumeMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = true;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Ephemeral,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: false,
        reflect_outgoing: false,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::WebSessionResume;
}

/// How a [`FileMessage`] should be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRenderingType {
    /// Render as a file.
    File,
    /// Render as media (e.g. an image, audio or video).
    Media,
    /// Render as a sticker (for transparent images).
    Sticker,
}

/// Optional metadata of a [`FileMessage`] (depends on the media type).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileMetadata {
    /// Width in px (images, videos).
    pub width: Option<u32>,
    /// Height in px (images, videos).
    pub height: Option<u32>,
    /// Whether the image is animated.
    pub animated: Option<bool>,
    /// Duration in seconds (audio, videos).
    pub duration: Option<f64>,
}

/// A `file` message: a file (or media) uploaded to the blob server, encrypted with `key` (file
/// with nonce `00..01`, thumbnail with nonce `00..02`).
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct FileMessage {
    /// Rendering type.
    pub rendering_type: FileRenderingType,

    /// Random symmetric key used to encrypt the file and thumbnail blobs.
    #[educe(Debug(ignore))]
    pub key: [u8; 32],

    /// Blob ID of the file data.
    pub blob_id: BlobId,

    /// Media type of the file.
    pub media_type: String,

    /// Optional file name.
    #[educe(Debug(ignore))]
    pub name: Option<String>,

    /// File size in bytes.
    pub size: u64,

    /// Optional blob ID of the thumbnail.
    pub thumbnail_blob_id: Option<BlobId>,

    /// Media type of the thumbnail (`image/jpeg` if not set).
    pub thumbnail_media_type: Option<String>,

    /// Optional caption.
    #[educe(Debug(ignore))]
    pub caption: Option<String>,

    /// Optional 32 byte ASCII string to collocate multiple media files.
    pub correlation_id: Option<String>,

    /// Optional metadata.
    pub metadata: FileMetadata,
}
impl FileMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = TextMessage::CONTACT_PROPERTIES;
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::File;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = TextMessage::GROUP_PROPERTIES;
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupFile;
}

/// A `poll-setup` message: creates (state open) or closes (state closed, with the results) a
/// poll. `poll` is the protocol's JSON object, interpreted by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct PollSetupMessage {
    /// Poll ID, unique per creator within the conversation.
    pub poll_id: u64,
    /// UTF-8 JSON of the poll (see `csp.struct.yml`).
    #[educe(Debug(ignore))]
    pub poll: String,
}
impl PollSetupMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = TextMessage::CONTACT_PROPERTIES;
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::PollSetup;
    // Exempt from blocking in groups (the poll state must stay consistent).
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = TextMessage::GROUP_PROPERTIES;
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupPollSetup;
}

/// A `poll-vote` message: the sender's choices for a poll.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct PollVoteMessage {
    /// Creator of the poll.
    pub creator_identity: ThreemaId,
    /// Poll ID.
    pub poll_id: u64,
    /// UTF-8 JSON list of `[choice id, 0|1]` for every choice.
    #[educe(Debug(ignore))]
    pub choices: String,
}
impl PollVoteMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::PollVote;
    // Exempt from blocking in groups (the poll state must stay consistent).
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupPollVote;
}

/// What a [`ReactionMessage`] does with its emoji.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum ReactionAction {
    /// Apply the emoji reaction (additive, multiple reactions per sender are allowed).
    Apply(String),

    /// Withdraw a specific emoji reaction.
    Withdraw(String),
}

/// A `Reaction` message: apply or withdraw an emoji reaction to a referred message.
///
/// Peers without `REACTION_SUPPORT` only understand legacy reactions, see [`LegacyReactionMessage`].
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct ReactionMessage {
    /// Unique ID of the referred message.
    pub message_id: MessageId,

    /// The emoji to apply or withdraw.
    pub action: ReactionAction,
}
impl ReactionMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::Reaction;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupReaction;
}

/// An `EditMessage` message: replace the text (or caption) of a message the sender sent before.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct EditMessage {
    /// Unique ID of the referred message to be edited.
    pub message_id: MessageId,

    /// Text (or caption) to update the referred message with (should be ≤ 6000 bytes).
    pub text: String,
}
impl EditMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::EditMessage;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupEditMessage;
}

/// A `GroupCallStart` message: announces and immediately starts a group call.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupCallStartMessage {
    /// Protocol version used for group calls of this group (currently `1`).
    pub protocol_version: u32,

    /// The secret Group Call Key (`GCK`) used for this call.
    pub gck: [u8; 32],

    /// The base URL of the SFU, used to peek and join the call.
    pub sfu_base_url: String,
}
impl GroupCallStartMessage {
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupCallStart;
}

/// A `DeleteMessage` message: remove a message the sender sent before.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct DeleteMessage {
    /// Unique ID of the referred message to be removed.
    pub message_id: MessageId,
}
impl DeleteMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::DeleteMessage;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = false;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupDeleteMessage;
}

/// A `typing-indicator` message: whether the sender is currently typing in the 1:1 conversation.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct TypingIndicatorMessage {
    /// Whether the sender is typing (`1`) or stopped typing (`0`).
    pub is_typing: bool,
}
impl TypingIndicatorMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Ephemeral,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: false,
        reflect_incoming: true,
        reflect_outgoing: false,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::TypingIndicator;
}

/// A `call-offer` message: an offer starting a call. The body is the message's UTF-8 JSON object
/// (`csp.struct.yml`), handled by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallOfferMessage {
    /// The JSON-encoded body.
    #[educe(Debug(method(debug_slice_length)))]
    pub json: Vec<u8>,
}
impl CallOfferMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Brief,
        user_profile_distribution: true,
        requires_direct_contact: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::CallOffer;
}

/// A `call-answer` message: an answer (accept or reject) to a call offer. The body is the message's UTF-8 JSON object
/// (`csp.struct.yml`), handled by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallAnswerMessage {
    /// The JSON-encoded body.
    #[educe(Debug(method(debug_slice_length)))]
    pub json: Vec<u8>,
}
impl CallAnswerMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Brief,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::CallAnswer;
}

/// A `call-ice-candidate` message: ICE candidates for an ongoing call. The body is the message's UTF-8 JSON object
/// (`csp.struct.yml`), handled by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallIceCandidateMessage {
    /// The JSON-encoded body.
    #[educe(Debug(method(debug_slice_length)))]
    pub json: Vec<u8>,
}
impl CallIceCandidateMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Brief,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: false,
        reflect_incoming: true,
        reflect_outgoing: false,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::CallIceCandidate;
}

/// A `call-hangup` message: the end of a call. The body is the message's UTF-8 JSON object
/// (`csp.struct.yml`), handled by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallHangupMessage {
    /// The JSON-encoded body.
    #[educe(Debug(method(debug_slice_length)))]
    pub json: Vec<u8>,
}
impl CallHangupMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::CallHangup;
}

/// A `call-ringing` message: that the callee's device is ringing. The body is the message's UTF-8 JSON object
/// (`csp.struct.yml`), handled by the application.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct CallRingingMessage {
    /// The JSON-encoded body.
    #[educe(Debug(method(debug_slice_length)))]
    pub json: Vec<u8>,
}
impl CallRingingMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: true,
        lifetime: MessageLifetime::Brief,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::CallRinging;
}

/// Message body of a 1:1 message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum ContactMessageBody {
    /// See [`TextMessage`].
    Text(TextMessage),

    /// See [`LocationMessage`].
    Location(LocationMessage),

    /// See [`DeliveryReceiptMessage`].
    DeliveryReceipt(DeliveryReceiptMessage),

    /// See [`LegacyReactionMessage`].
    LegacyReaction(LegacyReactionMessage),

    /// See [`WebSessionResumeMessage`].
    WebSessionResume(WebSessionResumeMessage),

    /// See [`TypingIndicatorMessage`].
    TypingIndicator(TypingIndicatorMessage),

    /// See [`ReactionMessage`].
    Reaction(ReactionMessage),

    /// See [`FileMessage`].
    File(FileMessage),

    /// See [`SetProfilePictureMessage`].
    SetProfilePicture(SetProfilePictureMessage),

    /// See [`DeleteProfilePictureMessage`].
    DeleteProfilePicture(DeleteProfilePictureMessage),

    /// See [`RequestProfilePictureMessage`].
    RequestProfilePicture(RequestProfilePictureMessage),

    /// See [`PollSetupMessage`].
    PollSetup(PollSetupMessage),

    /// See [`PollVoteMessage`].
    PollVote(PollVoteMessage),

    /// See [`EditMessage`].
    Edit(EditMessage),

    /// See [`DeleteMessage`].
    Delete(DeleteMessage),

    /// See [`CallOfferMessage`].
    CallOffer(CallOfferMessage),

    /// See [`CallAnswerMessage`].
    CallAnswer(CallAnswerMessage),

    /// See [`CallIceCandidateMessage`].
    CallIceCandidate(CallIceCandidateMessage),

    /// See [`CallHangupMessage`].
    CallHangup(CallHangupMessage),

    /// See [`CallRingingMessage`].
    CallRinging(CallRingingMessage),
    // TODO(LIB-16): Complete
}
impl ContactMessageBody {
    /// Get the 1:1 message's type.
    const fn message_type(&self) -> CspE2eMessageType {
        match &self {
            ContactMessageBody::Text(_) => TextMessage::CONTACT_TYPE,
            ContactMessageBody::Location(_) => LocationMessage::CONTACT_TYPE,
            ContactMessageBody::DeliveryReceipt(_) => DeliveryReceiptMessage::CONTACT_TYPE,
            ContactMessageBody::LegacyReaction(_) => LegacyReactionMessage::CONTACT_TYPE,
            ContactMessageBody::WebSessionResume(_) => WebSessionResumeMessage::CONTACT_TYPE,
            ContactMessageBody::TypingIndicator(_) => TypingIndicatorMessage::CONTACT_TYPE,
            ContactMessageBody::Reaction(_) => ReactionMessage::CONTACT_TYPE,
            ContactMessageBody::File(_) => FileMessage::CONTACT_TYPE,
            ContactMessageBody::SetProfilePicture(_) => SetProfilePictureMessage::CONTACT_TYPE,
            ContactMessageBody::DeleteProfilePicture(_) => DeleteProfilePictureMessage::CONTACT_TYPE,
            ContactMessageBody::RequestProfilePicture(_) => RequestProfilePictureMessage::CONTACT_TYPE,
            ContactMessageBody::PollSetup(_) => PollSetupMessage::CONTACT_TYPE,
            ContactMessageBody::PollVote(_) => PollVoteMessage::CONTACT_TYPE,
            ContactMessageBody::Edit(_) => EditMessage::CONTACT_TYPE,
            ContactMessageBody::Delete(_) => DeleteMessage::CONTACT_TYPE,
            ContactMessageBody::CallOffer(_) => CallOfferMessage::CONTACT_TYPE,
            ContactMessageBody::CallAnswer(_) => CallAnswerMessage::CONTACT_TYPE,
            ContactMessageBody::CallIceCandidate(_) => CallIceCandidateMessage::CONTACT_TYPE,
            ContactMessageBody::CallHangup(_) => CallHangupMessage::CONTACT_TYPE,
            ContactMessageBody::CallRinging(_) => CallRingingMessage::CONTACT_TYPE,
        }
    }

    /// Get the 1:1 message's properties.
    const fn properties(&self) -> MessageProperties {
        match &self {
            ContactMessageBody::Text(_) => TextMessage::CONTACT_PROPERTIES,
            ContactMessageBody::Location(_) => LocationMessage::CONTACT_PROPERTIES,
            ContactMessageBody::DeliveryReceipt(_) => DeliveryReceiptMessage::CONTACT_PROPERTIES,
            ContactMessageBody::LegacyReaction(_) => LegacyReactionMessage::CONTACT_PROPERTIES,
            ContactMessageBody::WebSessionResume(_) => WebSessionResumeMessage::CONTACT_PROPERTIES,
            ContactMessageBody::TypingIndicator(_) => TypingIndicatorMessage::CONTACT_PROPERTIES,
            ContactMessageBody::Reaction(_) => ReactionMessage::CONTACT_PROPERTIES,
            ContactMessageBody::File(_) => FileMessage::CONTACT_PROPERTIES,
            ContactMessageBody::SetProfilePicture(_) => SetProfilePictureMessage::CONTACT_PROPERTIES,
            ContactMessageBody::DeleteProfilePicture(_) => DeleteProfilePictureMessage::CONTACT_PROPERTIES,
            ContactMessageBody::RequestProfilePicture(_) => RequestProfilePictureMessage::CONTACT_PROPERTIES,
            ContactMessageBody::PollSetup(_) => PollSetupMessage::CONTACT_PROPERTIES,
            ContactMessageBody::PollVote(_) => PollVoteMessage::CONTACT_PROPERTIES,
            ContactMessageBody::Edit(_) => EditMessage::CONTACT_PROPERTIES,
            ContactMessageBody::Delete(_) => DeleteMessage::CONTACT_PROPERTIES,
            ContactMessageBody::CallOffer(_) => CallOfferMessage::CONTACT_PROPERTIES,
            ContactMessageBody::CallAnswer(_) => CallAnswerMessage::CONTACT_PROPERTIES,
            ContactMessageBody::CallIceCandidate(_) => CallIceCandidateMessage::CONTACT_PROPERTIES,
            ContactMessageBody::CallHangup(_) => CallHangupMessage::CONTACT_PROPERTIES,
            ContactMessageBody::CallRinging(_) => CallRingingMessage::CONTACT_PROPERTIES,
        }
    }

    /// Determine whether the 1:1 message is exempt from blocking.
    #[must_use]
    pub const fn is_exempt_from_blocking(&self) -> bool {
        match &self {
            ContactMessageBody::Text(_) => TextMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::Location(_) => LocationMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::DeliveryReceipt(_) => DeliveryReceiptMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::LegacyReaction(_) => LegacyReactionMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::WebSessionResume(_) => WebSessionResumeMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::TypingIndicator(_) => TypingIndicatorMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::Reaction(_) => ReactionMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::File(_) => FileMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::SetProfilePicture(_) => {
                SetProfilePictureMessage::CONTACT_EXEMPT_FROM_BLOCKING
            },
            ContactMessageBody::DeleteProfilePicture(_) => {
                DeleteProfilePictureMessage::CONTACT_EXEMPT_FROM_BLOCKING
            },
            ContactMessageBody::RequestProfilePicture(_) => {
                RequestProfilePictureMessage::CONTACT_EXEMPT_FROM_BLOCKING
            },
            ContactMessageBody::PollSetup(_) => PollSetupMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::PollVote(_) => PollVoteMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::Edit(_) => EditMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::Delete(_) => DeleteMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::CallOffer(_) => CallOfferMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::CallAnswer(_) => CallAnswerMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::CallIceCandidate(_) => CallIceCandidateMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::CallHangup(_) => CallHangupMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            ContactMessageBody::CallRinging(_) => CallRingingMessage::CONTACT_EXEMPT_FROM_BLOCKING,
        }
    }
}

/// Message body of a distribution list message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum DistributionListMessageBody {
    /// See [`TextMessage`].
    Text(TextMessage),

    /// See [`LocationMessage`].
    Location(LocationMessage),
    // TODO(LIB-57): Complete
}
impl DistributionListMessageBody {
    /// Get the distribution list message's type.
    ///
    /// Note: These are always just 1:1 message types.
    const fn message_type(&self) -> CspE2eMessageType {
        match &self {
            DistributionListMessageBody::Text(_) => TextMessage::CONTACT_TYPE,
            DistributionListMessageBody::Location(_) => LocationMessage::CONTACT_TYPE,
        }
    }

    /// Get the distribution list message's properties.
    ///
    /// Note: The properties are identical to the 1:1 message's properties.
    const fn properties(&self) -> MessageProperties {
        match &self {
            DistributionListMessageBody::Text(_) => TextMessage::CONTACT_PROPERTIES,
            DistributionListMessageBody::Location(_) => LocationMessage::CONTACT_PROPERTIES,
        }
    }

    /// Determine whether the distribution list message is exempt from blocking.
    ///
    /// Note: This behaves identical to the respective 1:1 message.
    const fn is_exempt_from_blocking(&self) -> bool {
        match &self {
            DistributionListMessageBody::Text(_) => TextMessage::CONTACT_EXEMPT_FROM_BLOCKING,
            DistributionListMessageBody::Location(_) => LocationMessage::CONTACT_EXEMPT_FROM_BLOCKING,
        }
    }
}

/// A `group-setup` message (creator → members): the members besides the creator. An empty list
/// disbands the group; members not in the list have been removed.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupSetupMessage {
    /// The members (without the creator).
    pub members: Vec<ThreemaId>,
}
impl GroupSetupMessage {
    // Dedicated blocking steps (the receiver checks whether the group exists).
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: true,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupSetup;
}

/// A `group-name` message (creator → members).
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupNameMessage {
    /// The group's name.
    pub name: String,
}
impl GroupNameMessage {
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupName;
}

/// A `group-leave` message (member → members and creator).
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupLeaveMessage;
impl GroupLeaveMessage {
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupLeave;
}

/// A `group-sync-request` message (member → creator).
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct GroupSyncRequestMessage;
impl GroupSyncRequestMessage {
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupSyncRequest;
}

/// A `set-profile-picture` message: a contact's (1:1) or group's (creator → members) picture.
#[derive(Clone, Educe)]
#[educe(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub struct SetProfilePictureMessage {
    /// Blob ID of the picture.
    pub blob_id: BlobId,
    /// Picture size in bytes.
    pub size: u32,
    /// Key of the picture blob (nonce `00..01`).
    #[educe(Debug(ignore))]
    pub key: [u8; 32],
}
impl SetProfilePictureMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::ContactSetProfilePicture;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupSetProfilePicture;
}

/// A `delete-profile-picture` message (1:1, or group: creator → members).
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct DeleteProfilePictureMessage;
impl DeleteProfilePictureMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::ContactDeleteProfilePicture;
    const GROUP_EXEMPT_FROM_BLOCKING: bool = true;
    const GROUP_PROPERTIES: GroupMessageProperties = GroupMessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: true,
        reflect_sent_update: false,
    };
    const GROUP_TYPE: CspE2eMessageType = CspE2eMessageType::GroupDeleteProfilePicture;
}

/// A `contact-request-profile-picture` message: the sender wants our profile picture again.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct RequestProfilePictureMessage;
impl RequestProfilePictureMessage {
    const CONTACT_EXEMPT_FROM_BLOCKING: bool = false;
    const CONTACT_PROPERTIES: MessageProperties = MessageProperties {
        push: false,
        lifetime: MessageLifetime::Indefinite,
        user_profile_distribution: false,
        requires_direct_contact: false,
        replay_protection: true,
        reflect_incoming: true,
        reflect_outgoing: false,
        reflect_sent_update: false,
        delivery_receipts: false,
    };
    const CONTACT_TYPE: CspE2eMessageType = CspE2eMessageType::ContactRequestProfilePicture;
}

/// Message body of a group message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum GroupMessageBody {
    /// See [`TextMessage`].
    Text(TextMessage),

    /// See [`LocationMessage`].
    Location(LocationMessage),

    /// See [`LegacyReactionMessage`].
    LegacyReaction(LegacyReactionMessage),

    /// See [`FileMessage`].
    File(FileMessage),

    /// See [`PollSetupMessage`].
    PollSetup(PollSetupMessage),

    /// See [`PollVoteMessage`].
    PollVote(PollVoteMessage),

    /// See [`ReactionMessage`].
    Reaction(ReactionMessage),

    /// See [`EditMessage`].
    Edit(EditMessage),

    /// See [`DeleteMessage`].
    Delete(DeleteMessage),

    /// See [`GroupSetupMessage`].
    Setup(GroupSetupMessage),

    /// See [`GroupNameMessage`].
    Name(GroupNameMessage),

    /// See [`GroupLeaveMessage`].
    Leave(GroupLeaveMessage),

    /// See [`GroupSyncRequestMessage`].
    SyncRequest(GroupSyncRequestMessage),

    /// See [`SetProfilePictureMessage`].
    SetProfilePicture(SetProfilePictureMessage),

    /// See [`DeleteProfilePictureMessage`].
    DeleteProfilePicture(DeleteProfilePictureMessage),

    /// See [`GroupCallStartMessage`].
    CallStart(GroupCallStartMessage),
    // TODO(LIB-53): polls
}
impl GroupMessageBody {
    /// Get the group message's type.
    pub(crate) const fn message_type(&self) -> CspE2eMessageType {
        match &self {
            GroupMessageBody::Text(_) => TextMessage::GROUP_TYPE,
            GroupMessageBody::Location(_) => LocationMessage::GROUP_TYPE,
            GroupMessageBody::LegacyReaction(_) => LegacyReactionMessage::GROUP_TYPE,
            GroupMessageBody::File(_) => FileMessage::GROUP_TYPE,
            GroupMessageBody::PollSetup(_) => PollSetupMessage::GROUP_TYPE,
            GroupMessageBody::PollVote(_) => PollVoteMessage::GROUP_TYPE,
            GroupMessageBody::Reaction(_) => ReactionMessage::GROUP_TYPE,
            GroupMessageBody::Edit(_) => EditMessage::GROUP_TYPE,
            GroupMessageBody::Delete(_) => DeleteMessage::GROUP_TYPE,
            GroupMessageBody::Setup(_) => GroupSetupMessage::GROUP_TYPE,
            GroupMessageBody::Name(_) => GroupNameMessage::GROUP_TYPE,
            GroupMessageBody::Leave(_) => GroupLeaveMessage::GROUP_TYPE,
            GroupMessageBody::SyncRequest(_) => GroupSyncRequestMessage::GROUP_TYPE,
            GroupMessageBody::SetProfilePicture(_) => SetProfilePictureMessage::GROUP_TYPE,
            GroupMessageBody::DeleteProfilePicture(_) => DeleteProfilePictureMessage::GROUP_TYPE,
            GroupMessageBody::CallStart(_) => GroupCallStartMessage::GROUP_TYPE,
        }
    }

    /// Get the group message's properties.
    const fn properties(&self) -> GroupMessageProperties {
        match &self {
            GroupMessageBody::Text(_) => TextMessage::GROUP_PROPERTIES,
            GroupMessageBody::Location(_) => LocationMessage::GROUP_PROPERTIES,
            GroupMessageBody::LegacyReaction(_) => LegacyReactionMessage::GROUP_PROPERTIES,
            GroupMessageBody::File(_) => FileMessage::GROUP_PROPERTIES,
            GroupMessageBody::PollSetup(_) => PollSetupMessage::GROUP_PROPERTIES,
            GroupMessageBody::PollVote(_) => PollVoteMessage::GROUP_PROPERTIES,
            GroupMessageBody::Reaction(_) => ReactionMessage::GROUP_PROPERTIES,
            GroupMessageBody::Edit(_) => EditMessage::GROUP_PROPERTIES,
            GroupMessageBody::Delete(_) => DeleteMessage::GROUP_PROPERTIES,
            GroupMessageBody::Setup(_) => GroupSetupMessage::GROUP_PROPERTIES,
            GroupMessageBody::Name(_) => GroupNameMessage::GROUP_PROPERTIES,
            GroupMessageBody::Leave(_) => GroupLeaveMessage::GROUP_PROPERTIES,
            GroupMessageBody::SyncRequest(_) => GroupSyncRequestMessage::GROUP_PROPERTIES,
            GroupMessageBody::SetProfilePicture(_) => SetProfilePictureMessage::GROUP_PROPERTIES,
            GroupMessageBody::DeleteProfilePicture(_) => DeleteProfilePictureMessage::GROUP_PROPERTIES,
            GroupMessageBody::CallStart(_) => GroupCallStartMessage::GROUP_PROPERTIES,
        }
    }

    /// Whether the message is wrapped by `group-creator-container` (sent by the creator) instead of
    /// `group-member-container`.
    #[must_use]
    pub const fn uses_creator_container(&self) -> bool {
        matches!(
            self,
            GroupMessageBody::Setup(_)
                | GroupMessageBody::Name(_)
                | GroupMessageBody::SetProfilePicture(_)
                | GroupMessageBody::DeleteProfilePicture(_)
        )
    }

    /// Determine whether the group message is exempt from blocking.
    #[inline]
    #[must_use]
    pub fn is_exempt_from_blocking(&self) -> bool {
        match &self {
            GroupMessageBody::Text(_) => TextMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Location(_) => LocationMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::LegacyReaction(_) => LegacyReactionMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::File(_) => FileMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::PollSetup(_) => PollSetupMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::PollVote(_) => PollVoteMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Reaction(_) => ReactionMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Edit(_) => EditMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Delete(_) => DeleteMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Setup(_) => GroupSetupMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Name(_) => GroupNameMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::Leave(_) => GroupLeaveMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::SyncRequest(_) => GroupSyncRequestMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::SetProfilePicture(_) => SetProfilePictureMessage::GROUP_EXEMPT_FROM_BLOCKING,
            GroupMessageBody::DeleteProfilePicture(_) => {
                DeleteProfilePictureMessage::GROUP_EXEMPT_FROM_BLOCKING
            },
            GroupMessageBody::CallStart(_) => GroupCallStartMessage::GROUP_EXEMPT_FROM_BLOCKING,
        }
    }
}

/// An incoming group message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct IncomingGroupMessageBody {
    /// The specific group identity.
    pub group_identity: GroupIdentity,

    /// The message's body.
    pub body: GroupMessageBody,
}

/// Incoming message bodies for their respective conversations.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum IncomingMessageBody {
    /// An incoming 1:1 message from a contact.
    Contact(ContactMessageBody),

    /// An incoming group message.
    Group(IncomingGroupMessageBody),
}
impl IncomingMessageBody {
    /// Decode a message body from its CSP type and data, e.g. from a reflected D2D
    /// `IncomingMessage` or `OutgoingMessage` (type and body as in the envelope). `sender_identity`
    /// is the sender (for outgoing messages: the user), used for `group-creator-container`s.
    ///
    /// # Errors
    ///
    /// Returns an error message if the type is unknown or the data does not decode.
    pub fn decode_reflected(
        sender_identity: ThreemaId,
        message_type: i32,
        message_data: &[u8],
    ) -> Result<Self, String> {
        use crate::csp_e2e::message::codec::CspMessageDecoder as _;
        let message_type = CspE2eMessageType::try_from(message_type)
            .map_err(|_| format!("Unknown message type {message_type}"))?;
        Self::decode(sender_identity, message_type, message_data).map_err(|error| error.to_string())
    }

    /// Get the incoming message's type.
    const fn message_type(&self) -> CspE2eMessageType {
        match &self {
            IncomingMessageBody::Contact(body) => body.message_type(),
            IncomingMessageBody::Group(body) => body.body.message_type(),
        }
    }

    /// Get the incoming message's properties respective to the associated conversation.
    const fn properties(&self) -> MessageProperties {
        match &self {
            IncomingMessageBody::Contact(body) => body.properties(),
            IncomingMessageBody::Group(body) => body.body.properties().into_message_properties(),
        }
    }

    /// Determine whether the incoming message is exempt from blocking.
    fn is_exempt_from_blocking(&self) -> bool {
        match &self {
            IncomingMessageBody::Contact(body) => body.is_exempt_from_blocking(),
            IncomingMessageBody::Group(body) => body.body.is_exempt_from_blocking(),
        }
    }
}

/// An outgoing 1:1 message to a contact.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct OutgoingContactMessageBody {
    /// Identity of the receiver.
    pub receiver_identity: ThreemaId,

    /// The message's body.
    pub body: ContactMessageBody,
}

/// An outgoing distribution list message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct OutgoingDistributionListMessageBody {
    /// The specific distribution list identity.
    pub distribution_list_identity: u64,

    /// The message's body.
    pub body: DistributionListMessageBody,
}

/// An outgoing group message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct OutgoingGroupMessageBody {
    /// The specific group identity.
    pub group_identity: GroupIdentity,

    /// The message's body.
    pub body: GroupMessageBody,
}

/// Outgoing message bodies for their respective conversations.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub enum OutgoingMessageBody {
    /// An outgoing 1:1 message to a contact.
    Contact(OutgoingContactMessageBody),

    /// An outgoing distribution list message.
    DistributionList(OutgoingDistributionListMessageBody),

    /// An outgoing group message.
    Group(OutgoingGroupMessageBody),
}
impl OutgoingMessageBody {
    /// Get the outgoing message's type.
    pub(crate) const fn message_type(&self) -> CspE2eMessageType {
        match &self {
            OutgoingMessageBody::Contact(body) => body.body.message_type(),
            OutgoingMessageBody::DistributionList(body) => body.body.message_type(),
            OutgoingMessageBody::Group(body) => body.body.message_type(),
        }
    }

    /// Get the outgoing message's properties respective to the associated conversation.
    const fn properties(&self) -> MessageProperties {
        match &self {
            OutgoingMessageBody::Contact(body) => body.body.properties(),
            OutgoingMessageBody::DistributionList(body) => body.body.properties(),
            OutgoingMessageBody::Group(body) => body.body.properties().into_message_properties(),
        }
    }

    /// Determine whether the outgoing message is exempt from blocking.
    fn is_exempt_from_blocking(&self) -> bool {
        match &self {
            OutgoingMessageBody::Contact(body) => body.body.is_exempt_from_blocking(),
            OutgoingMessageBody::DistributionList(body) => body.body.is_exempt_from_blocking(),
            OutgoingMessageBody::Group(body) => body.body.is_exempt_from_blocking(),
        }
    }
}

/// An incoming message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct IncomingMessage {
    /// Identity of the sender.
    pub sender_identity: ThreemaId,

    /// The ID of the message.
    pub id: MessageId,

    /// Behaviour overrides.
    pub overrides: MessageOverrides,

    /// Unix-ish timestamp in milliseconds for when the message has been created.
    pub created_at: u64,

    /// The message's body for the respective conversation.
    pub body: IncomingMessageBody,
}

/// An outgoing message.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(PartialEq))]
pub struct OutgoingMessage {
    /// The ID of the message.
    pub id: MessageId,

    /// Behaviour overrides.
    pub overrides: MessageOverrides,

    /// Unix-ish timestamp in milliseconds for when the message has been created.
    pub created_at: u64,

    /// The message's body for the respective conversation.
    pub body: OutgoingMessageBody,
}
impl OutgoingMessage {
    /// Computes [`MessageFlags`] from the [`MessageProperties`] and then the [`MessageOverrides`].
    pub(crate) fn flags(&self) -> MessageFlags {
        MessageFlags::default()
            .chain_apply(self.properties())
            .chain_apply(self.overrides)
    }
}

#[duplicate_item(
    message_struct;
    [ IncomingMessage ];
    [ OutgoingMessage ];
)]
#[expect(clippy::allow_attributes, reason = "duplicate shenanigans")]
impl message_struct {
    /// Get the message's type.
    #[inline]
    #[must_use]
    pub const fn message_type(&self) -> CspE2eMessageType {
        self.body.message_type()
    }

    /// Get the message's properties.
    ///
    /// IMPORTANT: This does **NOT** include the overrides from [`MessageOverrides`]!
    #[inline]
    const fn properties(&self) -> MessageProperties {
        self.body.properties()
    }

    /// Get the effective message's properties, i.e. those with the [`MessageOverrides`] applied.
    #[inline]
    pub(crate) fn effective_properties(&self) -> MessageProperties {
        self.properties().chain_apply(self.overrides)
    }

    /// Determine whether the message is exempt from blocking.
    #[inline]
    #[allow(unused, reason = "TODO(LIB-51)")]
    pub(crate) fn is_exempt_from_blocking(&self) -> bool {
        self.body.is_exempt_from_blocking()
    }
}
