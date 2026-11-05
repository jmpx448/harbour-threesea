// SPDX-License-Identifier: AGPL-3.0-only
#ifndef LISTFILTER_H
#define LISTFILTER_H

#include <QSortFilterProxyModel>
#include <QStringList>

// A list of chats or contacts narrowed by a search field: the rows whose names or Threema ID hold
// every word of `query`, in the source's order. Any case and order, accents or none. A row is looked
// at through the roles displayName, firstName, lastName, nickname and identity, those it has. Rows
// come and go one by one, and the source's other users don't notice.
class ListFilter : public QSortFilterProxyModel
{
    Q_OBJECT
    Q_PROPERTY(QString query READ query WRITE setQuery NOTIFY queryChanged)
    Q_PROPERTY(int count READ rowCount NOTIFY countChanged)

public:
    explicit ListFilter(QObject *parent = nullptr);

    QString query() const { return m_query; }
    void setQuery(const QString &query);

    // Lower case with accents off, also those typed as a mark of their own.
    static QString folded(const QString &text);

signals:
    void queryChanged();
    void countChanged();

protected:
    bool filterAcceptsRow(int sourceRow, const QModelIndex &sourceParent) const override;

private:
    QString m_query;
    QStringList m_words;
};

#endif // LISTFILTER_H
