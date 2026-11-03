// SPDX-License-Identifier: AGPL-3.0-only
#ifndef OFFLINENETWORK_H
#define OFFLINENETWORK_H

#include <QNetworkAccessManager>
#include <QNetworkRequest>
#include <QQmlNetworkAccessManagerFactory>

// The UI never goes online: all network traffic is the core's, to Threema's servers only
// (CLAUDE.md §7.6). This stops QML from fetching anything remote, e.g. an <img src="http://…">
// in text that a Text item renders as rich text, which would work as a tracking pixel.
class OfflineNetworkAccessManager : public QNetworkAccessManager
{
public:
    using QNetworkAccessManager::QNetworkAccessManager;

protected:
    QNetworkReply *createRequest(Operation op, const QNetworkRequest &request, QIODevice *outgoingData) override
    {
        const QString scheme = request.url().scheme();
        if (scheme == QLatin1String("file") || scheme == QLatin1String("qrc") || scheme == QLatin1String("data")) {
            return QNetworkAccessManager::createRequest(op, request, outgoingData);
        }
        // An empty URL fails with ProtocolUnknownError without touching the network.
        return QNetworkAccessManager::createRequest(op, QNetworkRequest(), outgoingData);
    }
};

class OfflineNetworkFactory : public QQmlNetworkAccessManagerFactory
{
public:
    QNetworkAccessManager *create(QObject *parent) override
    {
        return new OfflineNetworkAccessManager(parent);
    }
};

#endif // OFFLINENETWORK_H
