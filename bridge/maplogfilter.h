// SPDX-License-Identifier: AGPL-3.0-only
#ifndef MAPLOGFILTER_H
#define MAPLOGFILTER_H

#include <QString>

// The map renderer's records reach Qt's message handler. Our patch to MapLibre
// (scripts/maplibre/patches/0007) leaves only "MapLibre: <severity> <event>"; a record in
// MapLibre's own form ("[ ERROR ]  \"{thread}[Event]: text\"") can name a URL or a tile id, which
// says where a map was looked at (CLAUDE.md §7.3), so if one ever comes through unpatched its
// text goes no further than here. tests/mapview checks both.
inline QString withoutMapDetails(const QString &message)
{
    static const char *const severities[] = { "[ DEBUG ]", "[ INFO ]", "[ WARNING ]", "[ ERROR ]" };
    for (const char *severity : severities) {
        if (message.startsWith(QLatin1String(severity))) {
            const QString name = QString::fromLatin1(severity);
            return QStringLiteral("MapLibre: ") + name.mid(2, name.size() - 4)
                   + QStringLiteral(" (text withheld)");
        }
    }
    return message;
}

#endif
