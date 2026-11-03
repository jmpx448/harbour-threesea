// SPDX-License-Identifier: AGPL-3.0-only
#include "enterkeyfilter.h"

#include <QKeyEvent>

void EnterKeyFilter::setEditor(QObject *editor)
{
    if (editor == m_editor) {
        return;
    }
    if (m_editor) {
        m_editor->removeEventFilter(this);
    }
    m_editor = editor;
    if (m_editor) {
        m_editor->installEventFilter(this);
    }
    emit editorChanged();
}

bool EnterKeyFilter::eventFilter(QObject *watched, QEvent *event)
{
    if (m_enabled && event->type() == QEvent::KeyPress) {
        auto *key = static_cast<QKeyEvent *>(event);
        if (key->key() == Qt::Key_Return || key->key() == Qt::Key_Enter) {
            const bool shift = key->modifiers() & Qt::ShiftModifier;
            if (shift == m_sendWithShift) {
                emit sendRequested();
                return true;
            }
        }
    }
    return QObject::eventFilter(watched, event);
}
