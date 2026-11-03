// SPDX-License-Identifier: AGPL-3.0-only
#include "jsonlistmodel.h"

#include <QHash>

JsonListModel::JsonListModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int JsonListModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_rows.size();
}

QHash<int, QByteArray> JsonListModel::roleNames() const
{
    QHash<int, QByteArray> names;
    for (int i = 0; i < m_roles.size(); ++i) {
        names.insert(Qt::UserRole + 1 + i, m_roles.at(i).toUtf8());
    }
    return names;
}

QVariant JsonListModel::data(const QModelIndex &index, int role) const
{
    const int column = role - Qt::UserRole - 1;
    if (!index.isValid() || index.row() >= m_rows.size() || column < 0 || column >= m_roles.size()) {
        return QVariant();
    }
    return m_rows.at(index.row()).value(m_roles.at(column));
}

void JsonListModel::setKeyField(const QString &field)
{
    if (field != m_keyField) {
        m_keyField = field;
        emit keyFieldChanged();
    }
}

void JsonListModel::setRoleList(const QStringList &roles)
{
    if (roles != m_roles) {
        beginResetModel();
        m_roles = roles;
        endResetModel();
        emit rolesChanged();
    }
}

QVariant JsonListModel::keyOf(const QVariantMap &row) const
{
    return row.value(m_keyField);
}

int JsonListModel::indexOf(const QVariant &key, int from) const
{
    for (int i = from; i < m_rows.size(); ++i) {
        if (keyOf(m_rows.at(i)) == key) {
            return i;
        }
    }
    return -1;
}

int JsonListModel::indexOfKey(const QVariant &key) const
{
    return indexOf(key, 0);
}

QVariantMap JsonListModel::get(int index) const
{
    return index >= 0 && index < m_rows.size() ? m_rows.at(index) : QVariantMap();
}

void JsonListModel::update(const QVariantList &rows)
{
    const int oldCount = m_rows.size();
    // Keys still to be placed (from position i on), to tell a row that moved from one that went.
    QHash<QString, int> wanted;
    for (const QVariant &row : rows) {
        ++wanted[keyOf(row.toMap()).toString()];
    }
    for (int i = 0; i < rows.size(); ++i) {
        const QVariantMap row = rows.at(i).toMap();
        const QVariant key = keyOf(row);
        // Rows here that aren't wanted any more go first.
        while (i < m_rows.size() && wanted.value(keyOf(m_rows.at(i)).toString()) == 0) {
            beginRemoveRows(QModelIndex(), i, i);
            m_rows.removeAt(i);
            endRemoveRows();
        }
        --wanted[key.toString()];
        if (i < m_rows.size() && keyOf(m_rows.at(i)) == key) {
            if (m_rows.at(i) != row) {
                m_rows[i] = row;
                emit dataChanged(index(i), index(i));
            }
            continue;
        }
        // Further down: one move (a chat coming to the top moves one row, not all in between;
        // deep audit 2026-10-27, P03).
        const int existing = indexOf(key, i + 1);
        if (existing > i) {
            beginMoveRows(QModelIndex(), existing, existing, QModelIndex(), i);
            m_rows.move(existing, i);
            endMoveRows();
            if (m_rows.at(i) != row) {
                m_rows[i] = row;
                emit dataChanged(index(i), index(i));
            }
            continue;
        }
        beginInsertRows(QModelIndex(), i, i);
        m_rows.insert(i, row);
        endInsertRows();
    }
    if (m_rows.size() > rows.size()) {
        beginRemoveRows(QModelIndex(), rows.size(), m_rows.size() - 1);
        m_rows.erase(m_rows.begin() + rows.size(), m_rows.end());
        endRemoveRows();
    }
    if (m_rows.size() != oldCount) {
        emit countChanged();
    }
}

void JsonListModel::upsert(const QVariantMap &row, int position)
{
    const int existing = indexOf(keyOf(row), 0);
    if (existing >= 0) {
        if (m_rows.at(existing) != row) {
            m_rows[existing] = row;
            emit dataChanged(index(existing), index(existing));
        }
        return;
    }
    position = qBound(0, position, m_rows.size());
    beginInsertRows(QModelIndex(), position, position);
    m_rows.insert(position, row);
    endInsertRows();
    emit countChanged();
}

void JsonListModel::insertAll(const QVariantList &rows, int position)
{
    if (rows.isEmpty()) {
        return;
    }
    // The keys that are here, once, instead of a scan per row.
    QHash<QString, int> byKey;
    byKey.reserve(m_rows.size());
    for (int row = 0; row < m_rows.size(); ++row) {
        byKey.insert(keyOf(m_rows.at(row)).toString(), row);
    }
    QList<QVariantMap> fresh;
    fresh.reserve(rows.size());
    for (const QVariant &value : rows) {
        const QVariantMap row = value.toMap();
        const auto known = byKey.constFind(keyOf(row).toString());
        if (known == byKey.constEnd()) {
            fresh.append(row);
            continue;
        }
        // Already here (pages can overlap): update it where it stands.
        const int at = known.value();
        if (m_rows.at(at) != row) {
            m_rows[at] = row;
            emit dataChanged(index(at), index(at));
        }
    }
    if (fresh.isEmpty()) {
        return;
    }
    position = qBound(0, position, m_rows.size());
    beginInsertRows(QModelIndex(), position, position + fresh.size() - 1);
    for (int i = 0; i < fresh.size(); ++i) {
        m_rows.insert(position + i, fresh.at(i));
    }
    endInsertRows();
    emit countChanged();
}

void JsonListModel::truncate(int rows)
{
    rows = qMax(0, rows);
    if (rows >= m_rows.size()) {
        return;
    }
    beginRemoveRows(QModelIndex(), rows, m_rows.size() - 1);
    m_rows.erase(m_rows.begin() + rows, m_rows.end());
    endRemoveRows();
    emit countChanged();
}

void JsonListModel::dropFirst(int rows)
{
    rows = qMin(rows, m_rows.size());
    if (rows <= 0) {
        return;
    }
    beginRemoveRows(QModelIndex(), 0, rows - 1);
    m_rows.erase(m_rows.begin(), m_rows.begin() + rows);
    endRemoveRows();
    emit countChanged();
}

void JsonListModel::remove(const QVariant &key)
{
    const int row = indexOf(key, 0);
    if (row < 0) {
        return;
    }
    beginRemoveRows(QModelIndex(), row, row);
    m_rows.removeAt(row);
    endRemoveRows();
    emit countChanged();
}

void JsonListModel::clear()
{
    if (m_rows.isEmpty()) {
        return;
    }
    beginResetModel();
    m_rows.clear();
    endResetModel();
    emit countChanged();
}
