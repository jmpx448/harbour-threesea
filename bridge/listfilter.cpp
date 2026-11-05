// SPDX-License-Identifier: AGPL-3.0-only
#include "listfilter.h"

ListFilter::ListFilter(QObject *parent)
    : QSortFilterProxyModel(parent)
{
    connect(this, &QAbstractItemModel::rowsInserted, this, &ListFilter::countChanged);
    connect(this, &QAbstractItemModel::rowsRemoved, this, &ListFilter::countChanged);
    connect(this, &QAbstractItemModel::modelReset, this, &ListFilter::countChanged);
    connect(this, &QAbstractItemModel::layoutChanged, this, &ListFilter::countChanged);
}

void ListFilter::setQuery(const QString &query)
{
    if (query == m_query) {
        return;
    }
    m_query = query;
    m_words = folded(query).split(QLatin1Char(' '), QString::SkipEmptyParts);
    invalidateFilter();
    emit queryChanged();
}

QString ListFilter::folded(const QString &text)
{
    // Letters that don't come apart into a base letter and a mark.
    static const struct { ushort letter; const char *base; } Whole[] = {
        { 0x00df, "ss" }, { 0x00e6, "ae" }, { 0x00f0, "d" }, { 0x00f8, "o" }, { 0x00fe, "th" },
        { 0x0111, "d" }, { 0x0127, "h" }, { 0x0131, "i" }, { 0x0142, "l" }, { 0x014b, "n" },
        { 0x0153, "oe" },
    };
    QString out;
    out.reserve(text.size());
    const QString decomposed = text.normalized(QString::NormalizationForm_D).toLower();
    for (const QChar letter : decomposed) {
        if (letter.category() == QChar::Mark_NonSpacing) {
            continue;
        }
        const char *base = nullptr;
        for (const auto &whole : Whole) {
            if (whole.letter == letter.unicode()) {
                base = whole.base;
                break;
            }
        }
        if (base) {
            out += QLatin1String(base);
        } else {
            out += letter.isSpace() ? QChar(QLatin1Char(' ')) : letter;
        }
    }
    return out;
}

bool ListFilter::filterAcceptsRow(int sourceRow, const QModelIndex &sourceParent) const
{
    if (m_words.isEmpty()) {
        return true;
    }
    const QAbstractItemModel *source = sourceModel();
    const QModelIndex row = source->index(sourceRow, 0, sourceParent);
    const QHash<int, QByteArray> roles = source->roleNames();
    QStringList names;
    for (auto role = roles.constBegin(); role != roles.constEnd(); ++role) {
        const QByteArray &name = role.value();
        if (name != "displayName" && name != "firstName" && name != "lastName" && name != "nickname"
                && name != "identity") {
            continue;
        }
        const QString value = source->data(row, role.key()).toString();
        // A group's or list's key is nothing anyone would type: their name alone counts.
        if (name == "identity" && value.contains(QLatin1Char(':'))) {
            continue;
        }
        names << folded(value);
    }
    for (const QString &word : m_words) {
        bool found = false;
        for (const QString &name : names) {
            if (name.contains(word)) {
                found = true;
                break;
            }
        }
        if (!found) {
            return false;
        }
    }
    return true;
}
