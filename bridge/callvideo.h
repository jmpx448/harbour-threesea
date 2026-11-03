// SPDX-License-Identifier: AGPL-3.0-only
#ifndef CALLVIDEO_H
#define CALLVIDEO_H

#include <QImage>
#include <QPointer>
#include <QQuickItem>

// A call's video in QML (`CallVideo { manager: calls; remote: true }`): the latest frame from
// CallManager, filling the item (cropped, like Android's call screen). No frame yet: nothing is
// drawn.
class CallVideo : public QQuickItem
{
    Q_OBJECT
    Q_PROPERTY(QObject *manager READ manager WRITE setManager NOTIFY managerChanged)
    // The peer's video (else our camera's preview).
    Q_PROPERTY(bool remote READ remote WRITE setRemote NOTIFY remoteChanged)
    // Group calls: this participant's picture (`groupCalls.participantFrame`); -1: `remote` decides.
    Q_PROPERTY(int participant READ participant WRITE setParticipant NOTIFY remoteChanged)
    // Shown mirrored (the front camera's preview, like a mirror).
    Q_PROPERTY(bool mirror READ mirror WRITE setMirror NOTIFY mirrorChanged)
    // Rounded corners like the chat bubbles (Sailfish has no per-corner radius in Qt 5.6).
    Q_PROPERTY(qreal radius READ radius WRITE setRadius NOTIFY shapeChanged)
    Q_PROPERTY(bool roundTopLeft MEMBER m_roundTopLeft NOTIFY shapeChanged)
    Q_PROPERTY(bool roundTopRight MEMBER m_roundTopRight NOTIFY shapeChanged)
    Q_PROPERTY(bool roundBottomLeft MEMBER m_roundBottomLeft NOTIFY shapeChanged)
    Q_PROPERTY(bool roundBottomRight MEMBER m_roundBottomRight NOTIFY shapeChanged)
    // A frame arrived since the item was shown or the stream restarted.
    Q_PROPERTY(bool hasFrame READ hasFrame NOTIFY hasFrameChanged)
public:
    explicit CallVideo(QQuickItem *parent = nullptr);

    QObject *manager() const { return m_manager; }
    void setManager(QObject *manager);
    bool remote() const { return m_remote; }
    void setRemote(bool remote);
    int participant() const { return m_participant; }
    void setParticipant(int participant);
    bool hasFrame() const { return !m_image.isNull(); }
    bool mirror() const { return m_mirror; }
    qreal radius() const { return m_radius; }
    void setRadius(qreal radius);
    void setMirror(bool mirror);

signals:
    void managerChanged();
    void remoteChanged();
    void hasFrameChanged();
    void mirrorChanged();
    void shapeChanged();

protected:
    QSGNode *updatePaintNode(QSGNode *node, UpdatePaintNodeData *) override;

private slots:
    void onFrame(const QImage &image);
    void onParticipantFrame(quint32 participant, const QImage &image);
    void clear();

private:
    void connectManager();

    QPointer<QObject> m_manager;
    bool m_remote = true;
    int m_participant = -1;
    bool m_mirror = false;
    qreal m_radius = 0;
    bool m_roundTopLeft = true;
    bool m_roundTopRight = true;
    bool m_roundBottomLeft = true;
    bool m_roundBottomRight = true;
    QImage m_image;
    bool m_newImage = false;
};

#endif // CALLVIDEO_H
