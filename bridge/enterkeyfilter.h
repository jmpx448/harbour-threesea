// SPDX-License-Identifier: AGPL-3.0-only
#ifndef ENTERKEYFILTER_H
#define ENTERKEYFILTER_H

#include <QObject>
#include <QPointer>

// "Enter sends" for a Silica TextArea: its key events go straight to the inner TextEdit
// (`_editor`), so QML's Keys handlers on the TextArea never see Return. This filter sits on that
// editor and, when Enter (or Shift+Enter, `sendWithShift`) is pressed, swallows it and emits
// sendRequested() instead of letting it insert a new line. The on-screen keyboard's Enter has no
// Shift.
class EnterKeyFilter : public QObject
{
    Q_OBJECT
    // The TextArea's `_editor`.
    Q_PROPERTY(QObject *editor READ editor WRITE setEditor NOTIFY editorChanged)
    Q_PROPERTY(bool enabled MEMBER m_enabled NOTIFY enabledChanged)
    // true: Shift+Enter sends and Enter makes a new line; false: the other way round.
    Q_PROPERTY(bool sendWithShift MEMBER m_sendWithShift NOTIFY sendWithShiftChanged)

public:
    explicit EnterKeyFilter(QObject *parent = nullptr) : QObject(parent) {}

    QObject *editor() const { return m_editor; }
    void setEditor(QObject *editor);

signals:
    void editorChanged();
    void enabledChanged();
    void sendWithShiftChanged();
    void sendRequested();

protected:
    bool eventFilter(QObject *watched, QEvent *event) override;

private:
    QPointer<QObject> m_editor;
    bool m_enabled = true;
    bool m_sendWithShift = false;
};

#endif // ENTERKEYFILTER_H
