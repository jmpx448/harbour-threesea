// SPDX-License-Identifier: AGPL-3.0-only
// SecretStore: the database key lives in Sailfish Secrets, and when Secrets fails the app fails
// closed (CLAUDE.md §5): no new key while a database may exist, no fallback, one answer per
// question, and an interrupted replacement never loses the key. Exit 0: passed.
#include "secretstore.h"

#include "fakesecrets.h"

#include <QCoreApplication>
#include <cstdio>
#include <cstring>

#include "threesea_core.h"

// The core's check, stood in for: a wrapped key here starts with "WRAP", then 1 for a passphrase
// typed on the keyboard.
extern "C" int threesea_key_wrapped(const uint8_t *blob, size_t blob_len)
{
    if (blob_len < 5 || std::memcmp(blob, "WRAP", 4) != 0) {
        return -1;
    }
    return blob[4] == 1 ? 1 : 0;
}

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

const QString Key = QStringLiteral("databasekey");
const QString Staging = QStringLiteral("databasekey-new");

FakeDaemon &daemon()
{
    return FakeDaemon::instance();
}

// What one SecretStore said.
struct Heard
{
    QList<QByteArray> keys;
    QList<QByteArray> wrapped;
    QList<bool> keyboard;
    QStringList failed;
    QList<bool> stored;
    int confirmations = 0;

    int answers() const { return keys.size() + wrapped.size() + failed.size() + stored.size(); }
};

// Lets everything run: until the store has answered and the daemon has nothing left to do. No
// clock: a store that never answers runs out of turns, however slow the machine.
void settle(const Heard &heard)
{
    for (int turn = 0; turn < 100000; ++turn) {
        if (heard.answers() > 0 && daemon().pending == 0) {
            break;
        }
        QCoreApplication::processEvents();
        QCoreApplication::sendPostedEvents(nullptr, QEvent::DeferredDelete);
    }
    check(heard.answers() > 0, "an answer");
    check(daemon().pending == 0, "no request left running");
}

void listen(SecretStore &store, Heard &heard)
{
    QObject::connect(&store, &SecretStore::keyReady, [&heard](const QByteArray &key) { heard.keys << key; });
    QObject::connect(&store, &SecretStore::wrappedKeyReady, [&heard](const QByteArray &blob, bool keyboard) {
        heard.wrapped << blob;
        heard.keyboard << keyboard;
    });
    QObject::connect(&store, &SecretStore::failed, [&heard](const QString &kind) { heard.failed << kind; });
    QObject::connect(&store, &SecretStore::stored, [&heard](bool ok) { heard.stored << ok; });
    QObject::connect(&store, &SecretStore::confirmationNeeded, [&heard]() { ++heard.confirmations; });
}

Heard load(bool allowCreate)
{
    SecretStore store;
    Heard heard;
    listen(store, heard);
    store.loadKey(allowCreate);
    settle(heard);
    check(heard.answers() == 1, "exactly one answer");
    return heard;
}

Heard storeData(const QByteArray &data)
{
    SecretStore store;
    Heard heard;
    listen(store, heard);
    store.store(data);
    settle(heard);
    check(heard.answers() == 1, "exactly one answer to storing");
    return heard;
}

Heard replace()
{
    SecretStore store;
    Heard heard;
    listen(store, heard);
    store.replaceKey();
    settle(heard);
    check(heard.answers() == 1, "exactly one answer to replacing");
    return heard;
}

void withKey(const QByteArray &key)
{
    daemon().reset();
    daemon().collectionExists = true;
    daemon().secrets.insert(Key, key);
}

const QByteArray OldKey(32, 'o');

void firstStart()
{
    daemon().reset();
    const Heard heard = load(true);
    check(heard.keys.size() == 1 && heard.keys.first().size() == 32, "a new 32-byte key");
    check(daemon().collectionAsExpected, "an owner-only collection behind the device lock, kept unlocked");
    check(heard.keys.value(0) == daemon().secrets.value(Key), "the key handed out is the one stored");
    check(heard.keys.value(0) != QByteArray(32, '\0'), "not all zeros");

    const Heard again = load(false);
    check(again.keys == heard.keys, "the next start gets the same key");
    check(!daemon().log.filter(QStringLiteral("write")).isEmpty(), "written once");
}

void storedKey()
{
    withKey(OldKey);
    const Heard heard = load(false);
    check(heard.keys == QList<QByteArray>{OldKey}, "the stored key");
    check(daemon().log.filter(QStringLiteral("write")).isEmpty(), "nothing written");
    check(daemon().log.filter(QStringLiteral("remove")).isEmpty(), "nothing removed");
}

void failsClosed()
{
    // A database exists and its key is not there: never a new key, which would lose it for good.
    daemon().reset();
    daemon().collectionExists = true;
    Heard heard = load(false);
    check(heard.failed == QStringList{QStringLiteral("keyMissing")}, "a missing key is said");
    check(daemon().secrets.isEmpty(), "and no new one made");

    // Secrets answers with an error: that says nothing about the key, so nothing is made either.
    daemon().reset();
    daemon().collectionExists = true;
    daemon().failRead.insert(Key, Sailfish::Secrets::Result::DaemonError);
    heard = load(false);
    check(heard.failed == QStringList{QStringLiteral("secrets")}, "a daemon error is said");
    check(daemon().secrets.isEmpty(), "and no key made");
    check(heard.keys.isEmpty(), "no key handed out");

    // The same error on the staging copy, after the key wasn't found.
    daemon().reset();
    daemon().collectionExists = true;
    daemon().failRead.insert(Staging, Sailfish::Secrets::Result::DaemonError);
    heard = load(false);
    check(heard.failed == QStringList{QStringLiteral("secrets")}, "an error on the copy is said");
    check(daemon().secrets.isEmpty(), "and no key made after it");

    // The collection can't be made.
    daemon().reset();
    daemon().failCreate.insert(QString(), Sailfish::Secrets::Result::DatabaseError);
    heard = load(true);
    check(heard.failed == QStringList{QStringLiteral("secrets")}, "no collection: said");
    check(heard.keys.isEmpty(), "no collection: no key");

    // The new key can't be stored: not used either, as the next start wouldn't find it.
    daemon().reset();
    daemon().failWrite.insert(Key, Sailfish::Secrets::Result::DatabaseError);
    heard = load(true);
    check(heard.failed == QStringList{QStringLiteral("secrets")}, "a key not stored is said");
    check(heard.keys.isEmpty(), "and not handed out");

    // A stored key of the wrong size is no key: refused, not replaced.
    withKey(QByteArray(16, 'x'));
    heard = load(false);
    check(heard.failed == QStringList{QStringLiteral("keyMissing")}, "a damaged key is refused");
    check(daemon().secrets.value(Key) == QByteArray(16, 'x'), "and left as it was");
}

void wrappedKey()
{
    QByteArray blob("WRAP");
    blob.append(char(1));
    blob.append(QByteArray(40, 'w'));
    withKey(blob);
    const Heard heard = load(false);
    check(heard.wrapped == QList<QByteArray>{blob}, "a wrapped key goes to the passphrase");
    check(heard.keyboard == QList<bool>{true}, "typed on the keyboard");
    check(heard.keys.isEmpty(), "not as a key");
}

void interruptedReplacement()
{
    // Stopped after the old key was removed and before the new one was written: the copy is it.
    const QByteArray newKey(32, 'n');
    daemon().reset();
    daemon().collectionExists = true;
    daemon().secrets.insert(Staging, newKey);
    const Heard heard = load(false);
    check(heard.keys == QList<QByteArray>{newKey}, "the staging copy is the key");
    check(daemon().secrets.value(Key) == newKey, "and becomes the key again");
    check(!daemon().secrets.contains(Staging), "the copy goes once that is done");
}

void replacing()
{
    const QByteArray newData(32, 'n');
    withKey(OldKey);
    Heard heard = storeData(newData);
    check(heard.stored == QList<bool>{true}, "stored");
    check(daemon().secrets.value(Key) == newData, "the new data is the key");
    check(!daemon().secrets.contains(Staging), "no copy left");
    check(load(false).keys == QList<QByteArray>{newData}, "the next start uses it");

    // The copy can't be written: nothing changes, and the answer says so.
    withKey(OldKey);
    daemon().failWrite.insert(Staging, Sailfish::Secrets::Result::DatabaseError);
    heard = storeData(newData);
    check(heard.stored == QList<bool>{false}, "a failed copy: not stored");
    check(daemon().secrets.value(Key) == OldKey, "the old key stays");
    check(load(false).keys == QList<QByteArray>{OldKey}, "and the next start uses it");

    // The old key can't be removed: it stays in force, and the answer says so.
    withKey(OldKey);
    daemon().failRemove.insert(Key, Sailfish::Secrets::Result::DatabaseError);
    heard = storeData(newData);
    check(heard.stored == QList<bool>{false}, "a key that can't go: not stored");
    check(load(false).keys == QList<QByteArray>{OldKey}, "the next start uses the old one");

    // The last write fails after the old key went: the copy is what the next start uses, and
    // the answer has to say that it was stored.
    withKey(OldKey);
    daemon().failWrite.insert(Key, Sailfish::Secrets::Result::DatabaseError);
    heard = storeData(newData);
    check(heard.stored == QList<bool>{true}, "only the copy: stored");
    check(load(false).keys == QList<QByteArray>{newData}, "the next start uses the copy");

    // Reading the current key fails: nothing is touched.
    withKey(OldKey);
    daemon().failRead.insert(Key, Sailfish::Secrets::Result::DaemonError);
    heard = storeData(newData);
    check(heard.stored == QList<bool>{false}, "unknown current key: not stored");
    check(daemon().log.filter(QStringLiteral("remove")).isEmpty(), "and nothing removed");
    check(daemon().secrets.value(Key) == OldKey, "the old key untouched");
}

void lockedAfterBoot()
{
    // Unlocked: nothing to explain, and no prompt.
    withKey(OldKey);
    Heard heard = load(false);
    check(heard.confirmations == 0, "unlocked: nothing said");
    check(daemon().log.filter(QStringLiteral("read")).size() == 1, "unlocked: read once");

    // Locked: said before the prompt, and the prompt brings the key.
    withKey(OldKey);
    daemon().collectionLocked = true;
    heard = load(false);
    check(heard.confirmations == 1, "locked: said once");
    check(daemon().prompts == 1, "locked: one prompt");
    check(heard.keys == QList<QByteArray>{OldKey}, "locked: the key after the prompt");

    // Only asked after the attempt without the prompt.
    check(daemon().log.filter(QStringLiteral("read")).size() == 2, "locked: read twice");

    // The same for the other way the daemon says it.
    withKey(OldKey);
    daemon().collectionLocked = true;
    daemon().lockedAnswer = Sailfish::Secrets::Result::OperationRequiresUserInteraction;
    heard = load(false);
    check(heard.confirmations == 1 && heard.keys == QList<QByteArray>{OldKey}, "needs interaction: the same");

    // The prompt ends in an error: fails closed, as for any failed read.
    withKey(OldKey);
    daemon().collectionLocked = true;
    daemon().failRead.insert(Key, Sailfish::Secrets::Result::DaemonError);
    heard = load(false);
    check(heard.confirmations == 1, "cancelled: was said");
    check(heard.failed == QStringList{QStringLiteral("secrets")}, "cancelled: said as a failure");
    check(heard.keys.isEmpty(), "cancelled: no key");
    check(daemon().secrets.value(Key) == OldKey, "cancelled: the key untouched");
}

void newKeyForANewId()
{
    withKey(OldKey);
    Heard heard = replace();
    check(heard.keys.size() == 1 && heard.keys.first() != OldKey, "a new key for the new ID");
    check(daemon().secrets.value(Key) == heard.keys.value(0), "stored");

    // The old one can't be removed: no new key while it is still there.
    withKey(OldKey);
    daemon().failRemove.insert(Key, Sailfish::Secrets::Result::DatabaseError);
    heard = replace();
    check(heard.failed == QStringList{QStringLiteral("oldKeyStored")}, "an old key that stays is said");
    check(heard.keys.isEmpty(), "and no new key");
    check(daemon().secrets.value(Key) == OldKey, "nothing else changed");
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    firstStart();
    storedKey();
    failsClosed();
    wrappedKey();
    interruptedReplacement();
    replacing();
    lockedAfterBoot();
    newKeyForANewId();
    std::printf(failures ? "%d FAILED\n" : "passed\n", failures);
    return failures ? 1 : 0;
}
