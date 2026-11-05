// SPDX-License-Identifier: AGPL-3.0-only
// JsonListModel and ContactListModel: update() ends with exactly the given rows, with few row
// signals (a chat moving to the top is one move, deep audit 2026-10-27, P03). And a command's
// result as a QML callback gets it. Exit 0: passed.
#include "contactlistmodel.h"
#include "coreresult.h"
#include "jsonlistmodel.h"

#include <QCoreApplication>
#include <QElapsedTimer>
#include <QJsonDocument>
#include <cstdio>
#include <random>

namespace {

int failures = 0;

void check(bool ok, const char *what)
{
    if (!ok) {
        std::printf("FAILED: %s\n", what);
        ++failures;
    }
}

struct Counts
{
    int inserted = 0;
    int removed = 0;
    int moved = 0;
    int changed = 0;
};

void count(QAbstractItemModel *model, Counts *counts)
{
    QObject::connect(model, &QAbstractItemModel::rowsInserted, [counts](const QModelIndex &, int first, int last) {
        counts->inserted += last - first + 1;
    });
    QObject::connect(model, &QAbstractItemModel::rowsRemoved, [counts](const QModelIndex &, int first, int last) {
        counts->removed += last - first + 1;
    });
    QObject::connect(model, &QAbstractItemModel::rowsMoved, [counts]() { ++counts->moved; });
    QObject::connect(model, &QAbstractItemModel::dataChanged, [counts]() { ++counts->changed; });
}

QVariantList rows(const QList<int> &ids, const QString &field, int version = 0)
{
    QVariantList list;
    for (int id : ids) {
        QVariantMap row;
        row.insert(field, field == QLatin1String("identity") ? QVariant(QString::number(id)) : QVariant(id));
        row.insert(QStringLiteral("v"), version);
        list << row;
    }
    return list;
}

QList<int> keys(const QAbstractListModel &model, const QString &field, int role)
{
    Q_UNUSED(field)
    QList<int> list;
    for (int i = 0; i < model.rowCount(); ++i) {
        list << model.data(model.index(i), role).toInt();
    }
    return list;
}

} // namespace

int main(int argc, char **argv)
{
    QCoreApplication app(argc, argv);

    JsonListModel json;
    json.setKeyField(QStringLiteral("id"));
    json.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
    const int idRole = Qt::UserRole + 1;
    QList<int> ids;
    for (int i = 0; i < 1000; ++i) {
        ids << i;
    }
    json.update(rows(ids, QStringLiteral("id")));
    Counts counts;
    count(&json, &counts);

    // The last chat gets a message: it comes to the top.
    QList<int> moved = ids;
    moved.prepend(moved.takeLast());
    json.update(rows(moved, QStringLiteral("id")));
    check(keys(json, QStringLiteral("id"), idRole) == moved, "json: last to top");
    check(counts.moved == 1 && counts.removed == 0 && counts.inserted == 0, "json: last to top is one move");
    std::printf("last to top: %d moved, %d removed, %d inserted\n", counts.moved, counts.removed, counts.inserted);

    // Random changes: always the given rows, in order.
    std::mt19937 engine(42);
    auto bounded = [&engine](int low, int high) { return std::uniform_int_distribution<int>(low, high - 1)(engine); };
    QList<int> current = moved;
    for (int round = 0; round < 300; ++round) {
        QList<int> next = current;
        const int operations = bounded(1, 6);
        for (int o = 0; o < operations; ++o) {
            switch (bounded(0, 4)) {
            case 0:
                if (!next.isEmpty()) next.removeAt(bounded(0, next.size()));
                break;
            case 1:
                next.insert(bounded(0, next.size() + 1), 2000 + round * 10 + o);
                break;
            default:
                if (next.size() > 1) next.move(bounded(0, next.size()), bounded(0, next.size()));
            }
        }
        json.update(rows(next, QStringLiteral("id"), round));
        if (keys(json, QStringLiteral("id"), idRole) != next) {
            check(false, "json: random update");
            break;
        }
        current = next;
    }
    json.update(QVariantList());
    check(json.rowCount() == 0, "json: emptied");

    // A page of older messages: one insert for the whole page, overlapping rows updated in
    // place (the chat page's scrolling back).
    JsonListModel page;
    page.setKeyField(QStringLiteral("id"));
    page.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
    page.update(rows({3, 2, 1}, QStringLiteral("id")));
    Counts pageCounts;
    count(&page, &pageCounts);
    QVariantList older = rows({1, 0, -1, -2}, QStringLiteral("id"), 7);
    page.insertAll(older, page.rowCount());
    check(keys(page, QStringLiteral("id"), idRole) == QList<int>({3, 2, 1, 0, -1, -2}), "json: page appended");
    check(pageCounts.inserted == 3, "json: the page is one insert of three new rows");
    check(pageCounts.changed == 1, "json: the row both pages hold is updated once");
    check(page.get(2).value(QStringLiteral("v")).toInt() == 7, "json: the known row took the new values");
    page.insertAll(QVariantList(), page.rowCount());
    check(page.rowCount() == 6, "json: an empty page changes nothing");

    // What a bubble asks of the rows: one field, or a row by one field, without a row copy.
    {
        JsonListModel lookup;
        lookup.setKeyField(QStringLiteral("id"));
        lookup.setRoleList({QStringLiteral("id"), QStringLiteral("messageId")});
        QVariantList lookupRows;
        lookupRows << QVariantMap{{QStringLiteral("id"), 1}, {QStringLiteral("messageId"), QStringLiteral("aa")}};
        lookupRows << QVariantMap{{QStringLiteral("id"), 2}};
        lookupRows << QVariantMap{{QStringLiteral("id"), 3}, {QStringLiteral("messageId"), QStringLiteral("cc")}};
        lookup.update(lookupRows);
        check(lookup.indexOfField(QStringLiteral("messageId"), QStringLiteral("cc")) == 2, "indexOfField: found");
        check(lookup.indexOfField(QStringLiteral("messageId"), QStringLiteral("bb")) == -1, "indexOfField: not here");
        check(lookup.indexOfField(QStringLiteral("messageId"), QString()) == -1,
              "indexOfField: a row without the field isn't a row with an empty one");
        check(lookup.indexOfField(QStringLiteral("id"), 2.0) == 1, "indexOfField: a number as QML sends it");
        check(lookup.fieldAt(2, QStringLiteral("messageId")).toString() == QStringLiteral("cc"), "fieldAt: the field");
        check(!lookup.fieldAt(1, QStringLiteral("messageId")).isValid(), "fieldAt: no such field");
        check(!lookup.fieldAt(3, QStringLiteral("id")).isValid() && !lookup.fieldAt(-1, QStringLiteral("id")).isValid(),
              "fieldAt: no such row");
    }

    // What the chat page's scrolling back costs in the model: a page of a hundred messages
    // row by row against the same page as one insert (user report 2026-10-29: the UI stopped
    // while a page came in).
    {
        QList<int> history;
        for (int i = 0; i < 2000; ++i) {
            history << i;
        }
        QVariantList pageRows;
        for (int i = 1; i <= 100; ++i) {
            QVariantMap row;
            row.insert(QStringLiteral("id"), -i);
            row.insert(QStringLiteral("v"), 0);
            pageRows << row;
        }
        JsonListModel one;
        one.setKeyField(QStringLiteral("id"));
        one.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
        one.update(rows(history, QStringLiteral("id")));
        JsonListModel many;
        many.setKeyField(QStringLiteral("id"));
        many.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
        many.update(rows(history, QStringLiteral("id")));

        QElapsedTimer timer;
        timer.start();
        for (const QVariant &row : pageRows) {
            many.upsert(row.toMap(), many.rowCount());
        }
        const qint64 rowByRow = timer.nsecsElapsed();
        timer.restart();
        one.insertAll(pageRows, one.rowCount());
        const qint64 atOnce = timer.nsecsElapsed();
        check(one.rowCount() == many.rowCount(), "page: both ways end with the same rows");
        std::printf("a page of 100 into 2000 rows: row by row %.2f ms, as one insert %.2f ms\n",
                    double(rowByRow) / 1e6, double(atOnce) / 1e6);
        check(atOnce < rowByRow, "page: one insert is not slower than row by row");
    }

    // Letting the scrolled-through pages go again: one removal, the newest rows kept.
    {
        JsonListModel loaded;
        loaded.setKeyField(QStringLiteral("id"));
        loaded.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
        QList<int> all;
        for (int i = 0; i < 500; ++i) {
            all << i;
        }
        loaded.update(rows(all, QStringLiteral("id")));
        Counts trimCounts;
        count(&loaded, &trimCounts);
        loaded.truncate(200);
        check(loaded.rowCount() == 200, "truncate: the newest 200 stay");
        check(loaded.get(0).value(QStringLiteral("id")).toInt() == 0
                  && loaded.get(199).value(QStringLiteral("id")).toInt() == 199,
              "truncate: and they are the first ones");
        check(trimCounts.removed == 300, "truncate: 300 rows removed");
        loaded.truncate(500);
        check(loaded.rowCount() == 200, "truncate: more than there is changes nothing");

        // And the other end: the newest rows go while the reader is deep in the history.
        Counts dropCounts;
        JsonListModel window;
        window.setKeyField(QStringLiteral("id"));
        window.setRoleList({QStringLiteral("id"), QStringLiteral("v")});
        window.update(rows(all, QStringLiteral("id")));
        count(&window, &dropCounts);
        window.dropFirst(300);
        check(window.rowCount() == 200, "dropFirst: 200 left");
        check(window.get(0).value(QStringLiteral("id")).toInt() == 300, "dropFirst: from the front");
        check(dropCounts.removed == 300, "dropFirst: one removal of 300 rows");
        window.dropFirst(0);
        window.dropFirst(-5);
        check(window.rowCount() == 200, "dropFirst: nothing to drop changes nothing");
    }

    ContactListModel contacts;
    QList<int> people;
    for (int i = 0; i < 50; ++i) {
        people << i;
    }
    contacts.update(rows(people, QStringLiteral("identity")));
    Counts contactCounts;
    count(&contacts, &contactCounts);
    QList<int> reordered = people;
    reordered.move(40, 3);
    reordered.removeAt(10);
    contacts.update(rows(reordered, QStringLiteral("identity")));
    QList<int> got;
    const QHash<int, QByteArray> names = contacts.roleNames();
    const int identityRole = names.key("identity");
    for (int i = 0; i < contacts.rowCount(); ++i) {
        got << contacts.data(contacts.index(i), identityRole).toString().toInt();
    }
    check(got == reordered, "contacts: reorder and remove");
    check(contactCounts.moved == 1 && contactCounts.removed == 1 && contactCounts.inserted == 0,
          "contacts: one move, one removal");

    // Null stays null, the core's "no such thing" (a quote of a message this device doesn't have);
    // a result is an object, and an error reply's missing one an empty object.
    {
        QJSEngine engine;
        const QJsonObject reply = QJsonDocument::fromJson(
                R"({"none": null, "message": {"id": 7, "quote": null}})").object();
        check(coreResultForScript(&engine, reply.value(QStringLiteral("none"))).isNull(), "null stays null");
        const QJSValue message = coreResultForScript(&engine, reply.value(QStringLiteral("message")));
        check(message.isObject() && message.property(QStringLiteral("id")).toInt() == 7, "a result is the object");
        const QJSValue missing = coreResultForScript(&engine, reply.value(QStringLiteral("absent")));
        check(missing.isObject() && !missing.isNull(), "no result at all: an empty object");
    }

    std::printf(failures ? "%d failure(s)\n" : "passed\n", failures);
    return failures ? 1 : 0;
}
