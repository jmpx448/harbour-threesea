// SPDX-License-Identifier: AGPL-3.0-only
#ifndef FILEPREPARER_H
#define FILEPREPARER_H

#include <QByteArray>
#include <QImage>
#include <QObject>
#include <QRunnable>
#include <QString>
#include <QVariantMap>

// Prepares a picked file for sending, on a worker thread: photos are re-encoded (at most
// 2048 px, JPEG; PNG if they have transparency) which also drops their EXIF data (GPS!), and get
// a 512 px JPEG thumbnail. Videos larger than the setting asks for are scaled down where the
// phone can do it (Full flavour, VideoTranscoder); the core drops their metadata when it reads
// them. Other files are only described (the core reads them itself).
class FilePreparer : public QObject, public QRunnable
{
    Q_OBJECT
public:
    // `scratchPath`: where a scaled-down video may be written (the cache's capture folder,
    // emptied at every start); empty: send videos as they are.
    FilePreparer(const QString &path, const QVariantMap &request, const QString &scratchPath = QString());
    // A video: its first frame (from the thumbnailer) becomes the thumbnail, if there is one.
    void setVideoFrame(const QImage &frame) { m_videoFrame = frame; }
    void run() override;

signals:
    // `request` completed with mediaType, name, rendering, width and height for pictures (and
    // path for files);
    // `data` empty for files the core reads from `path`. `error` non-empty on failure.
    void prepared(const QVariantMap &request, const QByteArray &data, const QByteArray &thumbnail,
                  const QString &error);

private:
    QString m_path;
    QVariantMap m_request;
    QString m_scratchPath;
    QImage m_videoFrame;
};

#endif
