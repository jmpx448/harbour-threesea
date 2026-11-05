// SPDX-License-Identifier: AGPL-3.0-only
#ifndef THREESEBACKEND_H
#define THREESEBACKEND_H

#include <functional>

#include <QHash>
#include <QSet>
#include <QSettings>
#include <QJSValue>

class QJSEngine;
class QJsonObject;
#include <QObject>
#include <QRectF>
#include <QReadWriteLock>
#include <QRunnable>
#include <QString>
#include <QStringList>
#include <QThreadPool>
#include <QTimer>
#include <QUrl>
#include <QVariantMap>
#include <QtGlobal>

#include <threesea_core.h>

struct ThreeseaCore;
class ContactListModel;
class VoiceRecorder;
class SecretStore;

// Single QObject owning the Rust core. No business logic here: requests are forwarded as JSON
// (docs/ffi.md); replies/events come back on the core thread and are re-posted to this thread.
class ThreeseBackend : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString coreVersion READ coreVersion CONSTANT)
    // starting | locked (database key encrypted: passphrase first) | needsLicense |
    // needsIdentity | ready | error
    Q_PROPERTY(QString state READ state NOTIFY stateChanged)
    Q_PROPERTY(QString identity READ identity NOTIFY stateChanged)
    // State "starting": the core is bringing the stored data up to date, which can take a while.
    Q_PROPERTY(bool upgrading READ upgrading NOTIFY upgradingChanged)
    // State "error": what went wrong as Errors.js tells it apart, and the same with its code, for
    // support.
    Q_PROPERTY(QString errorKind READ errorKind NOTIFY stateChanged)
    Q_PROPERTY(QString errorText READ errorText NOTIFY stateChanged)
    Q_PROPERTY(QObject *contacts READ contacts CONSTANT)
    // disconnected | connecting | connected
    Q_PROPERTY(QString connectionState READ connectionState NOTIFY connectionStateChanged)
    Q_PROPERTY(int unreadCount READ unreadCount NOTIFY unreadCountChanged)
    // Who the unread messages are waiting for an answer from: the name of the chat list's newest
    // unread chat, for the cover. Empty while nothing is unread.
    Q_PROPERTY(QString unreadFrom READ unreadFrom NOTIFY unreadFromChanged)
    // Keep the process (and the connection) alive when the window is closed.
    Q_PROPERTY(bool keepRunning READ keepRunning WRITE setKeepRunning NOTIFY keepRunningChanged)
    // How much goes to the system log, Qt and core alike: "warn" (default), "info", "debug".
    // Device-local (Settings → Advanced); no level logs content (CLAUDE.md §7.3).
    Q_PROPERTY(QString logLevel READ logLevel WRITE setLogLevel NOTIFY logLevelChanged)
    // Device-local notification settings (not part of the Threema Safe backup).
    Q_PROPERTY(bool notificationsEnabled READ notificationsEnabled WRITE setNotificationsEnabled
                   NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool notificationPreview READ notificationPreview WRITE setNotificationPreview
                   NOTIFY notificationSettingsChanged)
    // Android's sound and vibration for new messages, each for chats and groups: the tone and the
    // vibration of the current sound profile's instant messages. Device-local.
    Q_PROPERTY(bool soundChats READ soundChats WRITE setSoundChats NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool vibrateChats READ vibrateChats WRITE setVibrateChats NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool soundGroups READ soundGroups WRITE setSoundGroups NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool vibrateGroups READ vibrateGroups WRITE setVibrateGroups NOTIFY notificationSettingsChanged)
    Q_PROPERTY(QString toneChats READ toneChats WRITE setToneChats NOTIFY notificationSettingsChanged)
    Q_PROPERTY(QString toneGroups READ toneGroups WRITE setToneGroups NOTIFY notificationSettingsChanged)
    // An incoming call's tone ("": the system's) and whether it vibrates. Device-local.
    Q_PROPERTY(QString toneCalls READ toneCalls WRITE setToneCalls NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool vibrateCalls READ vibrateCalls WRITE setVibrateCalls NOTIFY notificationSettingsChanged)
    // Opt-in: match the address book (hashed) against Threema, daily. Device-local.
    Q_PROPERTY(bool addressBookSync READ addressBookSync WRITE setAddressBookSync NOTIFY notificationSettingsChanged)
    // Last address book match (ms since epoch), 0: never.
    Q_PROPERTY(double addressBookSyncedAt READ addressBookSyncedAt WRITE setAddressBookSyncedAt
                   NOTIFY addressBookSyncedAtChanged)
    // The SDK emulator's software renderer blends translucent colours with alpha squared; QML
    // compensates (Theme.rgba alpha -> sqrt(alpha)) so the look matches a device.
    Q_PROPERTY(bool emulator READ emulator CONSTANT)
    // Android's "Auto-play animated images" (GIFs play in the chat by themselves). Device-local.
    Q_PROPERTY(bool autoPlayGifs READ autoPlayGifs WRITE setAutoPlayGifs NOTIFY autoPlayGifsChanged)
    // Enter sends the message (Shift+Enter: new line); off: the other way round. Device-local.
    Q_PROPERTY(bool enterToSend READ enterToSend WRITE setEnterToSend NOTIFY notificationSettingsChanged)
    // Sent videos (Android's video size): 0 original, 1 small (480 px), 2 medium (848 px).
    // Scaling needs the system's GStreamer, so it only happens in the Full flavour.
    Q_PROPERTY(int videoSize READ videoSize WRITE setVideoSize NOTIFY notificationSettingsChanged)
    // Longest side of sent photos (Android's sizes; 0: original size, still re-encoded).
    Q_PROPERTY(int imageSize READ imageSize WRITE setImageSize NOTIFY notificationSettingsChanged)
    // Android's "in-app sounds"/"in-app vibration": feedback while the app itself is in front
    // (the chime between two voice messages, a message arriving in the open chat). Device-local.
    Q_PROPERTY(bool inAppSounds READ inAppSounds WRITE setInAppSounds NOTIFY notificationSettingsChanged)
    Q_PROPERTY(bool inAppVibrate READ inAppVibrate WRITE setInAppVibrate NOTIFY notificationSettingsChanged)
    // Android's "proximity sensor": hold the phone to the ear to hear a voice message on the
    // earpiece. Device-local; the Lite flavour has no sensor permission, so it has no effect.
    Q_PROPERTY(bool earToPlay READ earToPlay WRITE setEarToPlay NOTIFY notificationSettingsChanged)
    // Emoji as Threema's pictures (Twemoji, bundled) instead of the system font. Device-local.
    Q_PROPERTY(bool threemaEmoji READ threemaEmoji WRITE setThreemaEmoji NOTIFY threemaEmojiChanged)
    // Messages of only 1–3 emoji are shown at double size (Android's default). Device-local.
    Q_PROPERTY(bool biggerEmojis READ biggerEmojis WRITE setBiggerEmojis NOTIFY biggerEmojisChanged)
    // Size of the message text in chats (Android's "font size"): 0 small, 1 normal, 2 large.
    Q_PROPERTY(int chatFontSize READ chatFontSize WRITE setChatFontSize NOTIFY chatFontSizeChanged)
    // Picture behind the messages in every chat (Android's wallpaper); "": none. Device-local.
    Q_PROPERTY(QString wallpaper READ wallpaper WRITE setWallpaper NOTIFY notificationSettingsChanged)
    // Full flavour: a systemd user service starts the app in the background at login.
    Q_PROPERTY(bool full READ full CONSTANT)
    // State "locked": whether the passphrase is typed on the keyboard (else digits).
    Q_PROPERTY(bool passphraseKeyboard READ passphraseKeyboard NOTIFY dataEncryptionChanged)
    // The stored data's key is encrypted with a passphrase (Settings → Security).
    Q_PROPERTY(bool dataEncrypted READ dataEncrypted NOTIFY dataEncryptionChanged)
    // The database was opened with the passphrase this run (the app lock needn't ask again).
    Q_PROPERTY(bool unlockedAtStart READ unlockedAtStart NOTIFY stateChanged)
    // Removing the ID stopped halfway (state "error"): retryDataRemoval goes on with it.
    Q_PROPERTY(bool dataRemovalStuck READ dataRemovalStuck NOTIFY stateChanged)

public:
    // `engine` is used to hand results to QML callbacks; it also becomes the parent.
    explicit ThreeseBackend(QJSEngine *engine);
    ~ThreeseBackend() override;
    // The process ends and the window goes next: what comes in from here is only kept track of.
    void prepareToEnd();
    // The process ends: sends what the closing pages handed over, stops the core and waits for the worker tasks, a few seconds at most. If
    // that says false something still runs against this object: leave with _exit, not through
    // the destructors.
    bool shutdown();

    QString coreVersion() const;
    QString state() const { return m_state; }
    QString identity() const { return m_identity; }
    bool upgrading() const { return m_upgrading; }
    QString errorKind() const { return m_errorKind; }
    QString errorText() const { return m_errorText; }
    QObject *contacts() const;
    QString connectionState() const { return m_connectionState; }
    int unreadCount() const { return m_unreadCount; }
    QString unreadFrom() const { return m_unreadFrom; }
    bool keepRunning() const;
    QString logLevel() const;
    // Qt's (and QML's console) messages at the stored level; main() calls it before anything logs.
    static void applyQtLogLevel();
    bool notificationsEnabled() const;
    void setNotificationsEnabled(bool enabled);
    bool notificationPreview() const;
    bool addressBookSync() const { return QSettings().value(QStringLiteral("addressBookSync"), false).toBool(); }
    void setAddressBookSync(bool sync)
    {
        QSettings().setValue(QStringLiteral("addressBookSync"), sync);
        emit notificationSettingsChanged();
    }
    double addressBookSyncedAt() const { return QSettings().value(QStringLiteral("addressBookSyncedAt"), 0).toDouble(); }
    void setAddressBookSyncedAt(double at)
    {
        QSettings().setValue(QStringLiteral("addressBookSyncedAt"), at);
        emit addressBookSyncedAtChanged();
    }
    void setNotificationPreview(bool preview);
    int imageSize() const { return QSettings().value(QStringLiteral("imageSize"), 1600).toInt(); }
    void setImageSize(int side);
    // These four are read per bubble while a chat scrolls: from members, not the settings file.
    bool threemaEmoji() const { return m_threemaEmoji; }
    void setThreemaEmoji(bool threema);
    bool biggerEmojis() const { return m_biggerEmojis; }
    void setBiggerEmojis(bool bigger);
    bool soundChats() const { return QSettings().value(QStringLiteral("soundChats"), true).toBool(); }
    void setSoundChats(bool on)
    {
        QSettings().setValue(QStringLiteral("soundChats"), on);
        emit notificationSettingsChanged();
    }
    bool vibrateChats() const { return QSettings().value(QStringLiteral("vibrateChats"), true).toBool(); }
    void setVibrateChats(bool on)
    {
        QSettings().setValue(QStringLiteral("vibrateChats"), on);
        emit notificationSettingsChanged();
    }
    bool soundGroups() const { return QSettings().value(QStringLiteral("soundGroups"), true).toBool(); }
    void setSoundGroups(bool on)
    {
        QSettings().setValue(QStringLiteral("soundGroups"), on);
        emit notificationSettingsChanged();
    }
    bool vibrateGroups() const { return QSettings().value(QStringLiteral("vibrateGroups"), true).toBool(); }
    void setVibrateGroups(bool on)
    {
        QSettings().setValue(QStringLiteral("vibrateGroups"), on);
        emit notificationSettingsChanged();
    }
    QString toneChats() const { return QSettings().value(QStringLiteral("toneChats")).toString(); }
    void setToneChats(const QString &path)
    {
        QSettings().setValue(QStringLiteral("toneChats"), path);
        emit notificationSettingsChanged();
    }
    QString toneGroups() const { return QSettings().value(QStringLiteral("toneGroups")).toString(); }
    void setToneGroups(const QString &path)
    {
        QSettings().setValue(QStringLiteral("toneGroups"), path);
        emit notificationSettingsChanged();
    }
    QString toneCalls() const { return QSettings().value(QStringLiteral("toneCalls")).toString(); }
    void setToneCalls(const QString &path)
    {
        QSettings().setValue(QStringLiteral("toneCalls"), path);
        emit notificationSettingsChanged();
    }
    bool vibrateCalls() const { return QSettings().value(QStringLiteral("vibrateCalls"), true).toBool(); }
    void setVibrateCalls(bool on)
    {
        QSettings().setValue(QStringLiteral("vibrateCalls"), on);
        emit notificationSettingsChanged();
    }
    int videoSize() const { return qBound(0, QSettings().value(QStringLiteral("videoSize"), 0).toInt(), 2); }
    void setVideoSize(int size)
    {
        QSettings().setValue(QStringLiteral("videoSize"), qBound(0, size, 2));
        emit notificationSettingsChanged();
    }
    int chatFontSize() const { return m_chatFontSize; }
    void setChatFontSize(int size);
    QString wallpaper() const { return QSettings().value(QStringLiteral("wallpaper")).toString(); }
    void setWallpaper(const QString &path)
    {
        QSettings().setValue(QStringLiteral("wallpaper"),
                             path.startsWith(QLatin1String("file://")) ? QUrl(path).toLocalFile() : path);
        emit notificationSettingsChanged();
    }
    bool inAppSounds() const { return QSettings().value(QStringLiteral("inAppSounds"), true).toBool(); }
    void setInAppSounds(bool on)
    {
        QSettings().setValue(QStringLiteral("inAppSounds"), on);
        emit notificationSettingsChanged();
    }
    bool inAppVibrate() const { return QSettings().value(QStringLiteral("inAppVibrate"), true).toBool(); }
    void setInAppVibrate(bool on)
    {
        QSettings().setValue(QStringLiteral("inAppVibrate"), on);
        emit notificationSettingsChanged();
    }
    bool earToPlay() const { return QSettings().value(QStringLiteral("earToPlay"), true).toBool(); }
    void setEarToPlay(bool on)
    {
        QSettings().setValue(QStringLiteral("earToPlay"), on);
        emit notificationSettingsChanged();
    }
    bool autoPlayGifs() const { return m_autoPlayGifs; }
    void setAutoPlayGifs(bool play);
    bool enterToSend() const { return QSettings().value(QStringLiteral("enterToSend"), true).toBool(); }
    void setEnterToSend(bool send)
    {
        QSettings().setValue(QStringLiteral("enterToSend"), send);
        emit notificationSettingsChanged();
    }
    // The i486 build only runs in the emulator (no x86 devices run Sailfish OS 5); the sandbox
    // hides /etc/sailfishos-emulator, so decide at compile time.
#ifdef THREESEA_FULL
    bool full() const { return true; }
#else
    bool full() const { return false; }
#endif
#ifdef Q_PROCESSOR_X86_32
    bool emulator() const { return true; }
#else
    bool emulator() const { return false; }
#endif
    void setKeepRunning(bool keep);
    void setLogLevel(const QString &level);

    // Decrypted media of a message (thumbnail or file); empty if not on the device. Thread-safe
    // (image providers call it from their loader threads).
    QByteArray readMedia(qint64 message, bool thumbnail) const;
    // Where the players read (MediaReader): one decrypted file on its way at a time, whichever
    // player asks, and this object outlives every read. Each player keeps what it read, outside the
    // core's budget for transfers, and a read briefly holds two copies.
    QThreadPool *mediaReads() const { return &m_mediaReads; }
    // A picture by name (group pictures: the group key); empty if none. Thread-safe.
    QByteArray readPicture(const QString &name) const;

    // Set our group's picture from an image file (cropped to a 512 px square JPEG).
    Q_INVOKABLE void setGroupPicture(const QString &key, const QString &path,
                                     const QJSValue &callback = QJSValue());
    // The first QR code in an image (a camera frame from grabToImage, or a file); "" if none.
    Q_INVOKABLE void decodeQr(const QVariant &image, const QJSValue &callback);
    Q_INVOKABLE QString decodeQrFile(const QString &path) const;
    // Set our profile picture from an image file (same preparation).
    Q_INVOKABLE void setProfilePicture(const QString &path, const QJSValue &callback = QJSValue());

    // Calls `callback(ok, result, errorKind, errorMessage)` when the core replies.
    // `lock` (private chats hidden again, at once) from a chat being left, naming the files still
    // being prepared: they reach the core after the lock, which takes just those (`pendingFiles`).
    Q_INVOKABLE void lockWithFilesOnTheirWay();
    Q_INVOKABLE void call(const QString &command, const QVariantMap &args = QVariantMap(),
                          const QJSValue &callback = QJSValue());
    // The same for C++ callers (e.g. CallManager): `done(ok, result, errorKind)`.
    using NativeCallback = std::function<void(bool, const QVariantMap &, const QString &)>;
    void request(const QString &command, const QVariantMap &args, NativeCallback done = NativeCallback());
    void refreshChatTones();
    void importOldChatTones();
    // This build makes and takes calls (Full, ADR 0012).
    static bool callsSupported();
    // Send a picked file to a contact (`asPhoto`: re-encoded picture with thumbnail, else the
    // file as it is). The input's text goes along as `caption`. Callback like `call`.
    // A video as media: `frame` is a grabToImage() result of its thumbnail (none: no thumbnail),
    // `seconds` its length (0: unknown). One whose metadata can't be taken out is refused
    // (`videoKeepsMetadata`), or with `keepMetadata`, the user having said so, goes as a plain file.
    Q_INVOKABLE void sendVideo(const QString &identity, const QString &path, const QVariant &frame,
                               double seconds, const QString &caption, const QJSValue &callback,
                               bool keepMetadata = false);
    Q_INVOKABLE void sendFile(const QString &identity, const QString &path, bool asPhoto,
                              const QString &caption, const QJSValue &callback = QJSValue());
    // Send the voice recorder's finished recording (AAC, or PCM that the core encodes as Opus).
    Q_INVOKABLE void sendVoice(const QString &identity, int durationMs, const QJSValue &callback = QJSValue());
    // Notification tones (device-local, file paths): the system's tones as [{path, title}];
    // a tone's title; the tone of all chats / all groups and of one chat ("": the default, i.e.
    // the sound profile's instant message tone, or for one chat the tone of all chats/groups).
    Q_INVOKABLE QVariantList alertTones() const;
    Q_INVOKABLE QString toneTitle(const QString &path) const;
    Q_INVOKABLE QString chatTone(const QString &identity) const;
    Q_INVOKABLE void setChatTone(const QString &identity, const QString &path);
    // In-app camera: where captures go (the app's cache), and removing one once it was sent
    // (it is in the encrypted store then; only files in that folder are removed: returns whether
    // this was one).
    Q_INVOKABLE QString captureFile(const QString &extension) const;
    Q_INVOKABLE bool removeCapture(const QString &path) const;
    // A picked photo turned by `rotation` (0, 90, 180, 270 degrees clockwise) and cut to `crop`
    // — a rectangle in fractions (0..1) of the turned picture — written as a new capture.
    // Returns its path, empty if the picture could not be read or written. EXIF data (the
    // camera's GPS!) stays behind with the original, as it does for everything we send.
    Q_INVOKABLE QString editPhoto(const QString &path, const QRectF &crop, int rotation) const;
    // Keep a GIF from the device as a favourite (`addGifFavourite`).
    Q_INVOKABLE void addGifFavourite(const QString &path, const QJSValue &callback = QJSValue());
    VoiceRecorder *voiceRecorder() const;
    // A stored Opus voice message as 16-bit mono 48 kHz PCM; empty if not possible. Thread-safe.
    QByteArray decodeVoice(qint64 message) const;
    bool passphraseKeyboard() const
    {
        return threesea_key_wrapped(reinterpret_cast<const uint8_t *>(m_wrappedKey.constData()),
                                    static_cast<size_t>(m_wrappedKey.size())) == 1;
    }
    bool unlockedAtStart() const { return m_unlockedAtStart; }
    bool dataRemovalStuck() const { return m_removingData && m_state == QLatin1String("error"); }
    bool dataEncrypted() const { return !m_wrappedKey.isEmpty(); }
    // Whether `passphrase` opens the encrypted key; callback(ok). Slow (scrypt), off the GUI thread.
    Q_INVOKABLE void checkPassphrase(const QString &passphrase, const QJSValue &callback = QJSValue());
    // State "locked": decrypt the database key with the passphrase and start; callback(ok).
    Q_INVOKABLE void unlockDatabase(const QString &passphrase, const QJSValue &callback = QJSValue());
    // Store the database key encrypted with `passphrase` (or, `encrypt` false, plain again) in
    // Sailfish Secrets; callback(ok). The core only records it (setLock keyEncrypted).
    Q_INVOKABLE void setKeyEncryption(bool encrypt, const QString &passphrase, bool keyboard,
                                      const QJSValue &callback = QJSValue());
    // Re-query the onboarding state.
    Q_INVOKABLE void refresh();
    // Settings → Remove ID: the state goes to "removing" and on to "starting" with a new, empty
    // database under a new key; everything of the ID goes (ADR 0018). dataRemoved or
    // dataRemovalFailed says how it ended.
    Q_INVOKABLE void removeAllData(const QString &pin);
    Q_INVOKABLE void retryDataRemoval();
    // From state "error", not while removing the ID: from the key on, as if just launched.
    Q_INVOKABLE void retryStart();
    // Reload the contact list model.
    Q_INVOKABLE void refreshContacts();
    // Recount unread messages (cover, notifications).
    Q_INVOKABLE void refreshUnread();

signals:
    void stateChanged();
    void coreEvent(const QString &name, const QVariantMap &data);
    // The networks in use have changed and calmed down: the window wakes the connection.
    void networkChangedSettled();
    void connectionStateChanged();
    void unreadCountChanged();
    void unreadFromChanged();
    void keepRunningChanged();
    void logLevelChanged();
    void notificationSettingsChanged();
    void addressBookSyncedAtChanged();
    void threemaEmojiChanged();
    void biggerEmojisChanged();
    void chatFontSizeChanged();
    void autoPlayGifsChanged();
    void dataEncryptionChanged();
    // The PIN was accepted: calls and players stop now, the core can take a while.
    void dataRemovalStarted();
    // The ID's data is going, with what the UI and the players hold of it. `linkedDevicesRemain`:
    // the device group could not be left, so linked devices keep the ID.
    void dataRemoved(bool linkedDevicesRemain);
    // Refused (the PIN) before anything happened; the previous state is back.
    void dataRemovalFailed(const QString &kind, const QString &message);
    void upgradingChanged();
    // Internal: key work done on a worker thread (queued back to this thread).
    void keyWorkDone(quint64 id, const QByteArray &result);

private slots:
    void handleMessage(const QByteArray &json);
    void startAgain();
    void onKeyWorkDone(quint64 id, const QByteArray &result);
    void reportNetwork();
    void finishDataRemoval();
    void filePrepared(const QVariantMap &request, const QByteArray &data, const QByteArray &thumbnail,
                      const QString &error);

private:
    // The settings the bubbles read, from the settings file.
    void loadBubbleSettings();
    static void onCoreMessage(void *userData, const char *json);
    void startCore(QByteArray key);
    // A 512 px square JPEG as the attachment; with `safeCopy` also Threema Safe's 400 px, q=60
    // version as the second one.
    void sendSquarePicture(const QString &path, QVariantMap request, const QJSValue &callback,
                           bool safeCopy = false);
    void fail(const QString &kind, const QString &detail = QString());
    void failFromCore(const QJsonObject &error);
    void setUpgrading(bool upgrading);
    // A command that can't reach a core still gets its answer: a refusal, or whoever waits for it
    // (a page holding its text, a reply from a notification) would wait for ever. `id`: the
    // sources of a file command, cleaned up after the answer.
    void answerNoCore(const QJSValue &callback, quint64 id = 0);
    void stopCore();
    void requestDataRemoval(const QString &pin);
    void dataRemovalRefused(const QVariantMap &error, const QString &kind);
    void removeTempSources(quint64 id, const QJSValue &callbackAnswer = QJSValue());
    QString startConfigJson() const;

    QJSEngine *m_engine;
    ThreeseaCore *m_core = nullptr;
    bool m_threemaEmoji = true;
    bool m_biggerEmojis = true;
    int m_chatFontSize = 1;
    bool m_autoPlayGifs = true;
    // Guards m_core against image loader threads while the core starts and stops.
    mutable QReadWriteLock m_coreLock;
    SecretStore *m_secrets;
    ContactListModel *m_contacts;
    VoiceRecorder *m_voiceRecorder;
    // Unread recounts asked for in a burst (many messages arriving at once) run once.
    QTimer m_unreadTimer;
    quint64 m_nextId = 1;
    QHash<quint64, QJSValue> m_callbacks;
    // What a send is reading from (id -> files); once it has finished, those in the capture cache
    // go: the capture, and a video's scaled-down copy. Here, not in the chat page's callback: that
    // runs in nothing once the page has gone, and the plain file would stay.
    QMultiHash<quint64, QString> m_tempSources;
    // Files with a FilePreparer running, by request: the chat each is for.
    QHash<quint64, QString> m_filesPreparing;
    QHash<quint64, NativeCallback> m_nativeCallbacks;
    // The chats' own tones (from the core, while it runs): the notifications ask for them at once.
    QVariantMap m_chatTones;
    QSet<quint64> m_statusRequests;
    QSet<quint64> m_contactRequests;
    QString m_state = QStringLiteral("starting");
    QString m_identity;
    bool m_upgrading = false;
    QString m_errorKind;
    QString m_errorText;
    QString m_connectionState = QStringLiteral("disconnected");
    int m_unreadCount = 0;
    QString m_unreadFrom;
    // The database key while the core runs (to encrypt it when key encryption is switched on);
    // memory only, wiped on exit.
    QByteArray m_dbKey;
    mutable QThreadPool m_mediaReads;
    QByteArray m_wrappedKey;
    class QNetworkConfigurationManager *m_network = nullptr;
    // The networks in use: a change wakes the connection, once things have calmed down.
    QStringList m_activeNetworks;
    QTimer m_networkWakeTimer;
    bool m_unlockedAtStart = false;
    // Removing the ID: from the request until the new core runs. Status replies don't count then.
    bool m_removingData = false;
    // The window was told (once: a repeat after a failure has nothing new to say).
    bool m_dataRemovedSent = false;
    QString m_stateBeforeRemoval;
    // The old core has stopped: what it sent last is still queued, and is for nobody any more.
    bool m_discardCoreMessages = false;
    // prepareToEnd() was called.
    bool m_ending = false;
    // Files on their way, which the core answers when it is done with them, and what the closing
    // pages handed over: if the process ends before a good answer they are gone unseen.
    QSet<quint64> m_mustArrive;
    void tellWhatIsLost();
    // Requests from before the removal (a video still being prepared) stay out of the new ID.
    quint64 m_firstLiveId = 1;
    // Pending key work: what to do with the result.
    QHash<quint64, std::function<void(const QByteArray &)>> m_keyWork;
};

#endif // THREESEBACKEND_H
