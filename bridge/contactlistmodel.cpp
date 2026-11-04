// SPDX-License-Identifier: AGPL-3.0-only
#include "contactlistmodel.h"

#include <QHash>

namespace {

QString identityOf(const QVariantMap &row)
{
    return row.value(QStringLiteral("identity")).toString();
}

} // namespace

ContactListModel::ContactListModel(QObject *parent)
    : QAbstractListModel(parent)
{
}

int ContactListModel::rowCount(const QModelIndex &parent) const
{
    return parent.isValid() ? 0 : m_rows.size();
}

QHash<int, QByteArray> ContactListModel::roleNames() const
{
    return {
        { IdentityRole, "identity" },
        { DisplayNameRole, "displayName" },
        { FirstNameRole, "firstName" },
        { LastNameRole, "lastName" },
        { NicknameRole, "nickname" },
        { VerificationRole, "verification" },
        { WorkRole, "work" },
        { InactiveRole, "inactive" },
        { InvalidRole, "invalid" },
        { SectionRole, "section" },
        { PictureVersionRole, "pictureVersion" },
    };
}

QVariant ContactListModel::data(const QModelIndex &index, int role) const
{
    if (!index.isValid() || index.row() >= m_rows.size()) {
        return QVariant();
    }
    const QVariantMap &row = m_rows.at(index.row());
    const QByteArray name = roleNames().value(role);
    return name.isEmpty() ? QVariant() : row.value(QString::fromLatin1(name));
}

int ContactListModel::indexOf(const QString &identity, int from) const
{
    for (int i = from; i < m_rows.size(); ++i) {
        if (identityOf(m_rows.at(i)) == identity) {
            return i;
        }
    }
    return -1;
}

void ContactListModel::update(const QVariantList &contacts)
{
    const int oldCount = m_rows.size();
    // As JsonListModel::update: unwanted rows go, moved rows move (one row, not all between).
    QHash<QString, int> wanted;
    for (const QVariant &contact : contacts) {
        ++wanted[identityOf(contact.toMap())];
    }
    for (int i = 0; i < contacts.size(); ++i) {
        const QVariantMap row = contacts.at(i).toMap();
        const QString identity = identityOf(row);
        while (i < m_rows.size() && wanted.value(identityOf(m_rows.at(i))) == 0) {
            beginRemoveRows(QModelIndex(), i, i);
            m_rows.removeAt(i);
            endRemoveRows();
        }
        --wanted[identity];
        const int existing = i < m_rows.size() && identityOf(m_rows.at(i)) == identity ? i : indexOf(identity, i + 1);
        if (existing > i) {
            beginMoveRows(QModelIndex(), existing, existing, QModelIndex(), i);
            m_rows.move(existing, i);
            endMoveRows();
        }
        if (existing >= i) {
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
    if (m_rows.size() > contacts.size()) {
        beginRemoveRows(QModelIndex(), contacts.size(), m_rows.size() - 1);
        m_rows.erase(m_rows.begin() + contacts.size(), m_rows.end());
        endRemoveRows();
    }
    if (m_rows.size() != oldCount) {
        emit countChanged();
    }
}
