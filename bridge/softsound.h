// SPDX-License-Identifier: AGPL-3.0-only
#ifndef SOFTSOUND_H
#define SOFTSOUND_H

#include <QAudioOutput>
#include <QBuffer>
#include <QByteArray>
#include <QObject>

// A short, soft bell generated in memory (no asset, nothing from the theme fits): the
// separator between two voice messages played one after the other, and the quiet feedback for
// a message arriving in the chat the user is reading (Android's "in-app sounds").
class SoftSound : public QObject
{
    Q_OBJECT

public:
    explicit SoftSound(QObject *parent = nullptr);

    // The bell as 48 kHz mono S16 PCM, `gain` times as loud as generated.
    static QByteArray samples(qreal gain = 1.0);
    // Plays it through the loudspeaker (media volume); done() once it has sounded, also when
    // it could not be played at all.
    Q_INVOKABLE void play();

signals:
    void done();

private slots:
    void cleanUp();

private:
    QAudioOutput *m_output = nullptr;
    QBuffer *m_buffer = nullptr;
    QByteArray m_data;
};

#endif
