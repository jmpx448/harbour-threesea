// SPDX-License-Identifier: AGPL-3.0-only
#ifndef JSONLISTMODEL_H
#define JSONLISTMODEL_H

#include <QAbstractListModel>
#include <QStringList>
#include <QVariantList>
#include <QVariantMap>

// A list of JSON objects from the core, identified by `keyField`. Exposed to QML; changes are
// applied row by row (insert/remove/dataChanged) so views keep their position and animations.
// Roles are the names in `roles`.
class JsonListModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(QString keyField READ keyField WRITE setKeyField NOTIFY keyFieldChanged)
    Q_PROPERTY(QStringList roles READ roleList WRITE setRoleList NOTIFY rolesChanged)
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    explicit JsonListModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    QString keyField() const { return m_keyField; }
    void setKeyField(const QString &field);
    QStringList roleList() const { return m_roles; }
    void setRoleList(const QStringList &roles);

    // Make the content equal to `rows` with minimal changes.
    Q_INVOKABLE void update(const QVariantList &rows);
    // Insert `row` at `position` or, if its key exists, update that row in place.
    Q_INVOKABLE void upsert(const QVariantMap &row, int position = 0);
    // The same for a whole page of rows, as one insert: scrolling back through a chat used to
    // insert its hundred messages one by one, and the view relaid out after every single one.
    Q_INVOKABLE void insertAll(const QVariantList &rows, int position = 0);
    // Keep the first `rows` rows and drop the rest, as one removal (the chat page keeps a
    // window around what the reader is looking at and lets the rest go).
    Q_INVOKABLE void truncate(int rows);
    // Drop the first `rows` rows, as one removal (the other end of that window).
    Q_INVOKABLE void dropFirst(int rows);
    Q_INVOKABLE QVariantMap get(int index) const;
    Q_INVOKABLE int indexOfKey(const QVariant &key) const;
    // The first row whose `field` is `value`, or -1. No row crosses into QML on the way.
    Q_INVOKABLE int indexOfField(const QString &field, const QVariant &value) const;
    // One field of a row, without copying the whole row into QML as get() does; undefined in
    // QML if there is no such row or field.
    Q_INVOKABLE QVariant fieldAt(int row, const QString &field) const;
    // Remove the row with this key, if any.
    Q_INVOKABLE void remove(const QVariant &key);
    Q_INVOKABLE void clear();

signals:
    void keyFieldChanged();
    void rolesChanged();
    void countChanged();

private:
    QVariant keyOf(const QVariantMap &row) const;
    int indexOf(const QVariant &key, int from) const;

    QString m_keyField = QStringLiteral("id");
    QStringList m_roles;
    QList<QVariantMap> m_rows;
};

#endif // JSONLISTMODEL_H
