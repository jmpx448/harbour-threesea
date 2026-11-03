// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CONTACTLISTMODEL_H
#define CONTACTLISTMODEL_H

#include <QAbstractListModel>
#include <QVariantList>
#include <QVariantMap>

// Contacts as delivered by the core (`contacts` command). Updates are applied row by row
// (insert/remove/dataChanged) so list views keep their position and animations.
class ContactListModel : public QAbstractListModel
{
    Q_OBJECT
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    enum Role {
        IdentityRole = Qt::UserRole + 1,
        DisplayNameRole,
        FirstNameRole,
        LastNameRole,
        NicknameRole,
        VerificationRole,
        WorkRole,
        InactiveRole,
        InvalidRole,
        SectionRole,
        PictureVersionRole,
    };

    explicit ContactListModel(QObject *parent = nullptr);

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role) const override;
    QHash<int, QByteArray> roleNames() const override;

    // Replace the content with `contacts` (already sorted by the core).
    void update(const QVariantList &contacts);

signals:
    void countChanged();

private:
    int indexOf(const QString &identity, int from) const;

    QList<QVariantMap> m_rows;
};

#endif // CONTACTLISTMODEL_H
