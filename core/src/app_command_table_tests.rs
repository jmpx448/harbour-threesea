// SPDX-License-Identifier: AGPL-3.0-only
//! Every command the UI can send, and what it may do with a private chat while the private chats
//! are hidden (CLAUDE.md §7.8). The gate (`check_readable`) is up to each command, so a new
//! command that forgets it would show a hidden chat to whoever asks: here each one is listed, and
//! each that names a chat or a message is tried on a hidden one. A command missing from the table
//! fails the test, so the question is asked for every new one.
use std::time::Duration;

use serde_json::{Value, json};
use zeroize::Zeroizing;

use super::{App, StartConfig};
use crate::{media::MediaPart, store::StoredIdentity};

/// What a command may do with a hidden private chat.
#[derive(Clone, Copy)]
enum Gate {
    /// It names no chat or message: nothing to gate.
    None,
    /// Tried with these arguments on the hidden chat, its messages, group or list, it answers
    /// `locked` before it waits for anything, and leaves the chat as it was.
    Locked(&'static str),
    /// Like [`Gate::Locked`] for a hidden group or list, which answers `groupNotFound` as one that
    /// doesn't exist would: the answer doesn't even say it is there.
    Missing(&'static str),
    /// Tried with these arguments, it answers without anything of the hidden chats and leaves
    /// their messages and drafts as they were: lists and searches, which leave the hidden chats
    /// out; what is about the contact, who stays visible as groups name their members; and the
    /// chat's place in the list, which shows nothing of it.
    Hidden(&'static str),
}

use Gate::{Hidden, Locked, Missing};

/// Placeholders in the arguments (each `"$NAME"` a JSON string where the command takes text, or
/// one replaced by a number): the hidden chat `$CHAT`, its group `$GROUP` and list `$LIST`; of
/// the chat a message `$MESSAGE` (local id) or `$MESSAGE_ID` (message id), our own text message
/// `$OUTGOING` / `$OUTGOING_ID` (failed), a received file `$FILE`, our file `$OUTGOING_FILE`, our
/// poll `$POLL`; a favourite GIF `$FAVOURITE`, a folder `$DIR` and an open chat `$OPEN`.
const TABLE: &[(&str, Gate)] = &[
    ("status", Gate::None),
    ("checkLicense", Gate::None),
    ("licenseStatus", Gate::None),
    ("forgetLicense", Gate::None),
    ("spamPotential", Locked(r#"{"identity":"$CHAT"}"#)),
    ("dismissSpam", Locked(r#"{"identity":"$CHAT"}"#)),
    ("reportSpam", Locked(r#"{"identity":"$CHAT","block":true}"#)),
    ("createIdentity", Gate::None),
    ("restoreIdentityBackup", Gate::None),
    ("exportIdentityBackup", Gate::None),
    ("safeCheckPassword", Gate::None),
    ("safeStatus", Gate::None),
    ("safeEnable", Gate::None),
    ("safeBackupNow", Gate::None),
    ("safeDisable", Gate::None),
    ("safeRestore", Gate::None),
    ("safeRetryImport", Gate::None),
    ("safeDeclineSetup", Gate::None),
    ("dismissDataImportHint", Gate::None),
    ("safeSearchId", Gate::None),
    ("androidBackupInspect", Gate::None),
    ("androidBackupRestore", Gate::None),
    ("androidBackupImport", Gate::None),
    ("androidBackupCancel", Gate::None),
    ("androidBackupExport", Gate::None),
    ("androidBackupSchedule", Gate::None),
    ("androidBackupExportStatus", Gate::None),
    ("contacts", Hidden(r#"{}"#)),
    ("contact", Hidden(r#"{"identity":"$CHAT"}"#)),
    ("addContact", Hidden(r#"{"identity":"$CHAT"}"#)),
    (
        "updateContact",
        Hidden(r#"{"identity":"$CHAT","firstName":"X"}"#),
    ),
    (
        "setContactPolicies",
        Hidden(r#"{"identity":"$CHAT","typing":1}"#),
    ),
    ("clearContactPolicies", Gate::None),
    ("deleteContact", Hidden(r#"{"identity":"$CHAT"}"#)),
    ("profile", Gate::None),
    ("declineNickname", Gate::None),
    ("setNickname", Gate::None),
    ("settings", Gate::None),
    ("setSetting", Gate::None),
    ("linkEmail", Gate::None),
    ("linkMobile", Gate::None),
    ("normalizeMobile", Gate::None),
    ("verifyMobile", Gate::None),
    ("callMobile", Gate::None),
    ("revocationStatus", Gate::None),
    ("setRevocationPassword", Gate::None),
    ("deleteAllData", Gate::None),
    ("conversations", Hidden(r#"{}"#)),
    ("findChats", Hidden(r#"{"query":"PRIVATE"}"#)),
    ("unreadSummary", Hidden(r#"{}"#)),
    ("lockStatus", Gate::None),
    ("setLock", Gate::None),
    ("unlock", Gate::None),
    ("lock", Gate::None),
    (
        "setPrivate",
        Locked(r#"{"identity":"$CHAT","private":false}"#),
    ),
    (
        "setArchived",
        Hidden(r#"{"identity":"$CHAT","archived":true}"#),
    ),
    (
        "setConversationUnread",
        Hidden(r#"{"identity":"$CHAT","unread":true}"#),
    ),
    ("setDeviceLabel", Gate::None),
    ("setDeviceAlias", Gate::None),
    ("messages", Locked(r#"{"identity":"$CHAT","limit":10}"#)),
    ("searchMessages", Hidden(r#"{"query":"PRIVATE"}"#)),
    ("sendText", Locked(r#"{"identity":"$CHAT","text":"x"}"#)),
    (
        "quotedMessage",
        Locked(r#"{"identity":"$CHAT","messageId":"$MESSAGE_ID"}"#),
    ),
    ("markRead", Locked(r#"{"identity":"$CHAT"}"#)),
    ("setTyping", Hidden(r#"{"identity":"$CHAT","typing":true}"#)),
    (
        "sendFile",
        Locked(
            r#"{"identity":"$CHAT","path":"/nonexistent","mediaType":"text/plain","rendering":"file"}"#,
        ),
    ),
    (
        "saveFile",
        Locked(r#"{"message":"$FILE","directory":"$DIR"}"#),
    ),
    ("retryUpload", Locked(r#"{"message":"$OUTGOING_FILE"}"#)),
    ("resendMessage", Locked(r#"{"message":"$OUTGOING"}"#)),
    ("cancelTransfer", Locked(r#"{"message":"$OUTGOING_FILE"}"#)),
    ("group", Missing(r#"{"key":"$GROUP"}"#)),
    ("createGroup", Gate::None),
    ("renameGroup", Missing(r#"{"key":"$GROUP","name":"x"}"#)),
    (
        "setGroupMembers",
        Missing(r#"{"key":"$GROUP","members":[]}"#),
    ),
    ("leaveGroup", Missing(r#"{"key":"$GROUP"}"#)),
    ("matchAddressBook", Gate::None),
    (
        "sendLocation",
        Locked(r#"{"identity":"$CHAT","latitude":1.0,"longitude":2.0}"#),
    ),
    ("findPlaces", Gate::None),
    (
        "setBlocked",
        Hidden(r#"{"identity":"$CHAT","blocked":true}"#),
    ),
    ("blockedContacts", Gate::None),
    (
        "setSyncExcluded",
        Hidden(r#"{"identity":"$CHAT","excluded":true}"#),
    ),
    ("syncExcluded", Gate::None),
    ("refreshContacts", Gate::None),
    ("createDistributionList", Gate::None),
    ("distributionList", Missing(r#"{"key":"$LIST"}"#)),
    (
        "updateDistributionList",
        Missing(r#"{"key":"$LIST","name":"x"}"#),
    ),
    ("setPinned", Hidden(r#"{"identity":"$CHAT","pinned":true}"#)),
    ("movePinned", Hidden(r#"{"identity":"$CHAT","offset":1}"#)),
    ("setDraft", Locked(r#"{"identity":"$CHAT","text":"x"}"#)),
    ("draft", Locked(r#"{"identity":"$CHAT"}"#)),
    ("chatMedia", Locked(r#"{"identity":"$CHAT"}"#)),
    ("setMediaKinds", Gate::None),
    ("message", Locked(r#"{"message":"$MESSAGE"}"#)),
    (
        "setStarred",
        Locked(r#"{"message":"$MESSAGE","starred":true}"#),
    ),
    ("setVoiceListened", Locked(r#"{"message":"$MESSAGE"}"#)),
    ("starredMessages", Hidden(r#"{}"#)),
    (
        "forwardMessages",
        Hidden(r#"{"messages":["$MESSAGE"],"identity":"$OPEN"}"#),
    ),
    (
        "setChatNotifications",
        Hidden(r#"{"identity":"$CHAT","policy":"never"}"#),
    ),
    ("chatTones", Hidden(r#"{}"#)),
    (
        "setChatTone",
        Locked(r#"{"identity":"$CHAT","tone":"/x.ogg"}"#),
    ),
    ("importChatTones", Gate::None),
    ("parseLink", Gate::None),
    ("myQrCode", Gate::None),
    ("verifyQrCode", Gate::None),
    ("qrKind", Gate::None),
    ("setProfilePicture", Gate::None),
    ("removeProfilePicture", Gate::None),
    ("setProfilePictureShare", Gate::None),
    (
        "setProfilePictureRecipient",
        Hidden(r#"{"identity":"$CHAT","share":true}"#),
    ),
    (
        "createPoll",
        Locked(
            r#"{"identity":"$CHAT","description":"q","choices":["a","b"],"multiple":false,"announce":false}"#,
        ),
    ),
    ("vote", Locked(r#"{"message":"$POLL","choices":[0]}"#)),
    ("closePoll", Locked(r#"{"message":"$POLL"}"#)),
    ("polls", Locked(r#"{"identity":"$CHAT"}"#)),
    ("setGroupPicture", Missing(r#"{"key":"$GROUP"}"#)),
    ("removeGroupPicture", Missing(r#"{"key":"$GROUP"}"#)),
    ("downloadFile", Locked(r#"{"message":"$FILE"}"#)),
    (
        "editMessage",
        Locked(r#"{"identity":"$CHAT","messageId":"$OUTGOING_ID","text":"x"}"#),
    ),
    (
        "deleteMessage",
        Locked(
            r#"{"identity":"$CHAT","messageId":"$MESSAGE_ID","outgoing":false,"everyone":false}"#,
        ),
    ),
    (
        "react",
        Locked(r#"{"identity":"$CHAT","messageId":"$MESSAGE_ID","emoji":"x","apply":true}"#),
    ),
    ("deleteConversation", Locked(r#"{"identity":"$CHAT"}"#)),
    ("setNetwork", Gate::None),
    ("displaySettings", Gate::None),
    ("setDisplaySettings", Gate::None),
    ("multiDevice", Gate::None),
    ("clearReflectedLeftOut", Gate::None),
    ("linkedDevices", Gate::None),
    ("dropDevices", Gate::None),
    ("deactivateMultiDevice", Gate::None),
    ("startLinking", Gate::None),
    ("confirmLinking", Gate::None),
    ("cancelLinking", Gate::None),
    ("callTurnServers", Hidden(r#"{"identity":"$CHAT"}"#)),
    ("callStart", Hidden(r#"{"identity":"$CHAT","sdp":"v=0"}"#)),
    ("callAccept", Gate::None),
    ("callReject", Gate::None),
    ("callHangUp", Gate::None),
    ("callFailed", Gate::None),
    ("callTimeout", Gate::None),
    ("callCandidates", Gate::None),
    ("callConnected", Gate::None),
    ("callChannel", Gate::None),
    ("callCapture", Gate::None),
    ("setVideoQuality", Gate::None),
    ("addressBookCountry", Gate::None),
    ("setAddressBookCountry", Gate::None),
    ("groupCallRefresh", Missing(r#"{"conversation":"$GROUP"}"#)),
    (
        "groupCallJoin",
        Missing(r#"{"conversation":"$GROUP","fingerprint":"00","create":true}"#),
    ),
    ("groupCallChannel", Gate::None),
    ("groupCallLeave", Gate::None),
    ("groupCallMute", Gate::None),
    ("groupCallCamera", Gate::None),
    ("groupCallTick", Gate::None),
    ("webSessions", Gate::None),
    ("webSetEnabled", Gate::None),
    ("webPair", Gate::None),
    ("webStart", Gate::None),
    ("webStop", Gate::None),
    ("webDelete", Gate::None),
    ("webDeleteAll", Gate::None),
    ("webRename", Gate::None),
    ("webAcceptCertificate", Gate::None),
    ("webDeclineCertificate", Gate::None),
    ("webRemoveServer", Gate::None),
    ("gifFavourites", Gate::None),
    ("recentEmoji", Gate::None),
    ("useEmoji", Gate::None),
    ("addGifFavourite", Locked(r#"{"message":"$FILE"}"#)),
    ("removeGifFavourite", Gate::None),
    (
        "sendGifFavourite",
        Locked(r#"{"identity":"$CHAT","favourite":"$FAVOURITE"}"#),
    ),
    ("updateSettings", Gate::None),
    ("setUpdateCheck", Gate::None),
    ("checkForUpdate", Gate::None),
    ("setImageSize", Gate::None),
    ("setLogLevel", Gate::None),
    ("forwardSecurity", Gate::None),
    ("setForwardSecurity", Gate::None),
    ("logText", Gate::None),
    ("logPackage", Gate::None),
    ("logClear", Gate::None),
    ("networkSettings", Gate::None),
    ("setNetworkSettings", Gate::None),
    ("mediaSettings", Gate::None),
    ("setMediaSettings", Gate::None),
    (
        "exportChat",
        Locked(r#"{"identity":"$CHAT","directory":"$DIR"}"#),
    ),
    ("storageInfo", Gate::None),
    ("demoData", Gate::None),
    ("cleanUp", Gate::None),
    ("emptyConversation", Locked(r#"{"identity":"$CHAT"}"#)),
    ("reconnect", Gate::None),
    ("wake", Gate::None),
];

const SECRETS: [&str; 5] = [
    "PRIVATE BODY",
    "PRIVATE REPLY",
    "PRIVATE DRAFT",
    "PRIVATE GROUP NAME",
    "PRIVATE LIST NAME",
];

struct Fixture {
    _dir: tempfile::TempDir,
    /// What `$DIR` stands for.
    out: tempfile::TempDir,
    app: App,
    group: String,
    list: String,
    /// `"$NAME"` and what it stands for.
    numbers: Vec<(&'static str, i64)>,
    texts: Vec<(&'static str, String)>,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let mut app = App::new(
        &StartConfig {
            data_dir: dir.path().to_owned(),
            app_version: "test".into(),
            language: "en".into(),
            country: "CH".into(),
            model: "test".into(),
            os_version: "test".into(),
            pictures_dir: None,
            calls: true,
            log_level: None,
            update_check: false,
        },
        &[3; 32],
    )
    .unwrap();
    app.store
        .set_identity(&StoredIdentity {
            threema_id: "AAAAAAAA".into(),
            client_key: Zeroizing::new([7; 32]),
            server_group: "a".into(),
        })
        .unwrap();
    for id in ["BBBBBBBB", "CCCCCCCC"] {
        app.store
            .upsert_contact(&crate::contacts::test_contact(id))
            .unwrap();
    }
    let store = &app.store;
    let message = store
        .insert_incoming("BBBBBBBB", 1, "text", "PRIVATE BODY", 1)
        .unwrap()
        .unwrap();
    let outgoing = store
        .insert_outgoing("BBBBBBBB", 2, "text", "PRIVATE REPLY")
        .unwrap();
    store.set_outgoing_failed(outgoing.id).unwrap();
    let file_body = json!({"name": "PRIVATE BODY.txt", "mediaType": "text/plain"}).to_string();
    let file = store
        .insert_incoming("BBBBBBBB", 3, "file", &file_body, 1)
        .unwrap()
        .unwrap();
    let outgoing_file = store
        .insert_outgoing("BBBBBBBB", 4, "file", &file_body)
        .unwrap();
    for id in [file.id, outgoing_file.id] {
        app.media
            .write(id, MediaPart::File, b"PRIVATE BODY")
            .unwrap();
    }
    store
        .insert_incoming("CCCCCCCC", 5, "text", "open text", 1)
        .unwrap();
    store
        .set_setting("draft:BBBBBBBB", "PRIVATE DRAFT")
        .unwrap();
    let (favourite, _) = store.add_gif_favourite("00", Some(1), Some(1), 1).unwrap();
    let group = app
        .create_group("PRIVATE GROUP NAME", &["BBBBBBBB".into()])
        .unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();
    app.store
        .insert_incoming(&group, 6, "text", "PRIVATE BODY", 1)
        .unwrap();
    let list = crate::distribution_lists::DistributionList {
        key: "d:00000000000000ff".into(),
        name: "PRIVATE LIST NAME".into(),
        members: vec!["BBBBBBBB".into()],
        created_at: 1,
    };
    app.store.upsert_distribution_list(&list).unwrap();
    let poll = app
        .create_poll(
            "BBBBBBBB",
            "PRIVATE BODY?",
            &["a".into(), "b".into()],
            false,
            false,
        )
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    app.store
        .set_lock(Some(&crate::applock::Lock::with_pin(
            "2468", false, true, 0,
        )))
        .unwrap();
    for conversation in ["BBBBBBBB", group.as_str(), list.key.as_str()] {
        app.store.set_private(conversation, true).unwrap();
    }
    let out = tempfile::tempdir().unwrap();
    let hex = |id: u64| format!("{id:016x}");
    Fixture {
        numbers: vec![
            ("MESSAGE", message.id),
            ("OUTGOING", outgoing.id),
            ("FILE", file.id),
            ("OUTGOING_FILE", outgoing_file.id),
            ("POLL", poll),
            ("FAVOURITE", favourite),
        ],
        texts: vec![
            ("CHAT", "BBBBBBBB".into()),
            ("OPEN", "CCCCCCCC".into()),
            ("MESSAGE_ID", hex(message.message_id)),
            ("OUTGOING_ID", hex(outgoing.message_id)),
            ("GROUP", group.clone()),
            ("LIST", list.key.clone()),
            ("DIR", out.path().display().to_string()),
        ],
        _dir: dir,
        out,
        app,
        group,
        list: list.key,
    }
}

impl Fixture {
    fn request(&self, name: &str, arguments: &str) -> String {
        let mut arguments = arguments.to_owned();
        for (placeholder, number) in &self.numbers {
            arguments = arguments.replace(&format!("\"${placeholder}\""), &number.to_string());
        }
        // Longest first: `$MESSAGE_ID` is not `$MESSAGE` with something after it.
        let mut texts = self.texts.clone();
        texts.sort_by_key(|(placeholder, _)| std::cmp::Reverse(placeholder.len()));
        for (placeholder, text) in texts {
            arguments = arguments.replace(&format!("${placeholder}"), &text);
        }
        assert!(!arguments.contains('$'), "{name}: {arguments}");
        let mut request: Value = serde_json::from_str(&arguments).unwrap();
        request["id"] = 1.into();
        request["cmd"] = name.into();
        request.to_string()
    }

    /// The hidden chats from the inside, to see that a command changed nothing of them.
    fn hidden(&self) -> String {
        let store = &self.app.store;
        let chats = ["BBBBBBBB", self.group.as_str(), self.list.as_str()].map(|chat| {
            let messages = store.conversation_messages(chat, 100).unwrap();
            let messages: Vec<Value> = messages.iter().map(|m| store.message_json(m)).collect();
            json!({
                "messages": messages,
                "draft": store.setting(&format!("draft:{chat}")).unwrap(),
                "private": store.is_private(chat).unwrap(),
            })
        });
        let group = store
            .group(&self.group)
            .unwrap()
            .map(|group| json!({"name": group.name, "members": group.members, "left": group.left}));
        let list = store
            .distribution_list(&self.list)
            .unwrap()
            .map(|list| json!({"name": list.name, "members": list.members}));
        json!({"chats": chats, "group": group, "list": list}).to_string()
    }

    /// Files the command left in the folder it was given.
    fn written(&self) -> usize {
        std::fs::read_dir(self.out.path()).unwrap().count()
    }
}

/// The command names of `Command`, as the UI sends them.
fn commands() -> Vec<String> {
    let source = std::fs::read_to_string("src/app.rs").unwrap();
    let start = source.find("pub enum Command {").unwrap();
    let end = source[start..].find("\n}\n").unwrap() + start;
    source[start..end]
        .lines()
        .filter_map(|line| {
            let name = line.strip_prefix("    ")?;
            let name: String = name
                .chars()
                .take_while(char::is_ascii_alphanumeric)
                .collect();
            name.starts_with(|c: char| c.is_ascii_uppercase())
                .then(|| name[..1].to_lowercase() + &name[1..])
        })
        .collect()
}

#[test]
fn every_command_is_in_the_table() {
    let listed: Vec<&str> = TABLE.iter().map(|(name, _)| *name).collect();
    let commands = commands();
    for name in &commands {
        assert!(
            listed.contains(&name.as_str()),
            "`{name}` is not in TABLE: can it show or change a hidden private chat?"
        );
    }
    for name in &listed {
        assert!(
            commands.iter().any(|command| command == name),
            "`{name}` is in TABLE but not a command"
        );
    }
}

/// Each command that names a chat or a message, on a hidden one. The core is offline: whatever
/// would wait for a server waits for ever, and the test's clock, which only moves when nothing
/// else can, ends the wait without any real time passing. A gate answers before any of that.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn hidden_private_chats_stay_hidden_from_every_command() {
    tokio::task::LocalSet::new().run_until(every_gate()).await;
}

async fn every_gate() {
    crate::https::stall_requests();
    let mut failures = Vec::new();
    // A refused command changes nothing, so those share one core (a new one after one that did);
    // the others each get their own.
    let mut shared = fixture();
    for (name, gate) in TABLE {
        let (Locked(arguments) | Missing(arguments) | Hidden(arguments)) = *gate else {
            continue;
        };
        let mut own = None;
        let fixture = match gate {
            Locked(_) | Missing(_) => &mut shared,
            _ => own.insert(fixture()),
        };
        let failed = failures.len();
        let request = fixture.request(name, arguments);
        let before = fixture.hidden();
        let reply = tokio::time::timeout(
            Duration::from_secs(3600),
            Box::pin(fixture.app.handle(&request)),
        )
        .await;
        if fixture.hidden() != before {
            failures.push(format!("{name}: the hidden chats changed"));
        }
        if fixture.written() > 0 {
            failures.push(format!("{name}: wrote a file"));
        }
        match reply {
            // Waiting for a server sends nothing back, so leaks nothing; but a gate comes first.
            Err(_) if matches!(gate, Locked(_) | Missing(_)) => {
                failures.push(format!("{name}: went on to the network"));
            }
            Err(_) => {}
            Ok(reply) => {
                if let Some(secret) = SECRETS.iter().find(|secret| reply.contains(*secret)) {
                    failures.push(format!("{name}: the reply has {secret:?}: {reply}"));
                }
                let reply: Value = serde_json::from_str(&reply).unwrap();
                match gate {
                    Locked(_) if reply["error"]["kind"] != "locked" => {
                        failures.push(format!("{name}: not refused: {reply}"));
                    }
                    Missing(_) if reply["error"]["kind"] != "groupNotFound" => {
                        failures.push(format!("{name}: not refused as missing: {reply}"));
                    }
                    Hidden(_) if reply["error"]["message"] == "invalid request" => {
                        failures.push(format!("{name}: arguments not understood"));
                    }
                    _ => {}
                }
            }
        }
        if matches!(gate, Locked(_) | Missing(_)) && failures.len() > failed {
            shared = self::fixture();
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
