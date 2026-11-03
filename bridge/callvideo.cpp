// SPDX-License-Identifier: AGPL-3.0-only
#include "callvideo.h"

#include <QQuickWindow>
#include <QSGGeometryNode>
#include <QSGTextureMaterial>
#include <QtMath>

CallVideo::CallVideo(QQuickItem *parent)
    : QQuickItem(parent)
{
    setFlag(ItemHasContents, true);
    connect(this, &CallVideo::shapeChanged, this, &QQuickItem::update);
}

void CallVideo::setRadius(qreal radius)
{
    if (!qFuzzyCompare(radius, m_radius)) {
        m_radius = radius;
        emit shapeChanged();
    }
}

void CallVideo::setManager(QObject *manager)
{
    if (manager == m_manager) {
        return;
    }
    if (m_manager) {
        disconnect(m_manager, nullptr, this, nullptr);
    }
    m_manager = manager;
    connectManager();
    emit managerChanged();
}

void CallVideo::setRemote(bool remote)
{
    if (remote == m_remote) {
        return;
    }
    m_remote = remote;
    connectManager();
    emit remoteChanged();
}

void CallVideo::setParticipant(int participant)
{
    if (participant == m_participant) {
        return;
    }
    m_participant = participant;
    connectManager();
    emit remoteChanged();
}

void CallVideo::setMirror(bool mirror)
{
    if (mirror == m_mirror) {
        return;
    }
    m_mirror = mirror;
    update();
    emit mirrorChanged();
}

void CallVideo::connectManager()
{
    if (!m_manager) {
        return;
    }
    disconnect(m_manager, nullptr, this, nullptr);
    if (m_participant >= 0) {
        connect(m_manager, SIGNAL(participantFrame(quint32,QImage)), this, SLOT(onParticipantFrame(quint32,QImage)));
    } else if (m_manager->metaObject()->indexOfSignal(m_remote ? "remoteFrame(QImage)" : "localFrame(QImage)") >= 0) {
        // (Group calls have no single remote picture; QML may set `participant` after this.)
        connect(m_manager, m_remote ? SIGNAL(remoteFrame(QImage)) : SIGNAL(localFrame(QImage)), this,
                SLOT(onFrame(QImage)));
        connect(m_manager, m_remote ? SIGNAL(remoteVideoStopped()) : SIGNAL(localVideoStopped()), this,
                SLOT(clear()));
    }
    clear();
}

void CallVideo::onFrame(const QImage &image)
{
    const bool first = m_image.isNull();
    m_image = image;
    m_newImage = true;
    update();
    if (first) {
        emit hasFrameChanged();
    }
}

void CallVideo::onParticipantFrame(quint32 participant, const QImage &image)
{
    if (static_cast<int>(participant) == m_participant) {
        onFrame(image);
    }
}

void CallVideo::clear()
{
    if (m_image.isNull()) {
        return;
    }
    m_image = QImage();
    m_newImage = true;
    update();
    emit hasFrameChanged();
}

namespace {

// A fan around the centre with the rounded rectangle's outline (convex, so a fan covers it):
// each corner a quarter circle or a single point.
QVector<QPointF> outline(const QRectF &rect, qreal radius, const bool round[4])
{
    const int steps = 8;
    // Corners clockwise from top left: centre of its arc, start angle (degrees).
    const QPointF centres[4] = {
        {rect.left() + radius, rect.top() + radius},
        {rect.right() - radius, rect.top() + radius},
        {rect.right() - radius, rect.bottom() - radius},
        {rect.left() + radius, rect.bottom() - radius},
    };
    const QPointF points[4] = {rect.topLeft(), rect.topRight(), rect.bottomRight(), rect.bottomLeft()};
    QVector<QPointF> result;
    for (int corner = 0; corner < 4; ++corner) {
        if (!round[corner] || radius <= 0) {
            result << points[corner];
            continue;
        }
        const qreal start = 180 + 90 * corner;
        for (int i = 0; i <= steps; ++i) {
            const qreal angle = qDegreesToRadians(start + 90.0 * i / steps);
            result << centres[corner] + QPointF(radius * qCos(angle), radius * qSin(angle));
        }
    }
    return result;
}

} // namespace

QSGNode *CallVideo::updatePaintNode(QSGNode *oldNode, UpdatePaintNodeData *)
{
    auto *node = static_cast<QSGGeometryNode *>(oldNode);
    if (m_image.isNull() || width() <= 0 || height() <= 0) {
        if (node) {
            // The material doesn't own its texture.
            delete static_cast<QSGOpaqueTextureMaterial *>(node->material())->texture();
            delete node;
        }
        return nullptr;
    }
    if (!node) {
        node = new QSGGeometryNode;
        auto *geometry = new QSGGeometry(QSGGeometry::defaultAttributes_TexturedPoint2D(), 0);
        geometry->setDrawingMode(GL_TRIANGLE_FAN);
        node->setGeometry(geometry);
        node->setFlag(QSGNode::OwnsGeometry);
        node->setMaterial(new QSGOpaqueTextureMaterial);
        node->setFlag(QSGNode::OwnsMaterial);
    }
    auto *material = static_cast<QSGOpaqueTextureMaterial *>(node->material());
    if (m_newImage || !material->texture()) {
        delete material->texture();
        material->setTexture(window()->createTextureFromImage(m_image));
        material->setFiltering(QSGTexture::Linear);
        m_newImage = false;
    }
    // Fill the item, cropping what doesn't fit (the middle stays).
    const qreal scale = qMax(width() / m_image.width(), height() / m_image.height());
    const qreal sourceWidth = width() / scale / m_image.width();
    const qreal sourceHeight = height() / scale / m_image.height();
    const QRectF source((1 - sourceWidth) / 2, (1 - sourceHeight) / 2, sourceWidth, sourceHeight);
    const QRectF rect = boundingRect();
    const bool round[4] = {m_roundTopLeft, m_roundTopRight, m_roundBottomRight, m_roundBottomLeft};
    const qreal radius = qMin(m_radius, qMin(width(), height()) / 2);
    QVector<QPointF> points = outline(rect, radius, round);
    points.prepend(rect.center());
    points.append(points.at(1)); // close the fan
    QSGGeometry *geometry = node->geometry();
    geometry->allocate(points.size());
    QSGGeometry::TexturedPoint2D *vertices = geometry->vertexDataAsTexturedPoint2D();
    for (int i = 0; i < points.size(); ++i) {
        const QPointF &p = points.at(i);
        qreal tx = (p.x() - rect.left()) / rect.width();
        if (m_mirror) {
            tx = 1 - tx;
        }
        const qreal ty = (p.y() - rect.top()) / rect.height();
        vertices[i].set(float(p.x()), float(p.y()), float(source.left() + tx * source.width()),
                        float(source.top() + ty * source.height()));
    }
    node->markDirty(QSGNode::DirtyGeometry | QSGNode::DirtyMaterial);
    return node;
}
