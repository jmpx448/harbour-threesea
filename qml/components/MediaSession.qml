// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Amber.Mpris 1.0

// What is playing, announced to the system (MPRIS): the lock screen's and top menu's media
// controls, headset buttons, Bluetooth. Exists only while something is loaded, so the controls
// go away with the message.
Loader {
    readonly property var player: videoPlayer.messageId >= 0 ? videoPlayer : voicePlayer
    readonly property bool isVideo: videoPlayer.messageId >= 0

    active: isVideo || voicePlayer.active
    sourceComponent: MprisPlayer {
        id: mpris
        serviceName: "harbour-threesea"
        identity: "Threesea"
        desktopEntry: "harbour-threesea"
        canControl: true
        canPlay: true
        canPause: true
        canSeek: player.messageId >= 0
        canRaise: true
        playbackStatus: player.playing || (!isVideo && voicePlayer.chiming) ? Mpris.Playing : Mpris.Paused
        minimumRate: 1
        maximumRate: isVideo ? 1 : 2
        rate: isVideo ? 1 : voicePlayer.speed
        volume: 1

        metaData {
            trackId: "/org/threesea/message/" + Math.max(0, player.messageId)
            title: isVideo ? qsTrId("threesea-la-video") : qsTrId("threesea-la-voice-message")
            // This shows on the lock screen: same rule as the name on the cover.
            contributingArtist: !isVideo && !window.locked && window.appSettings.disableScreenshots !== true
                                ? window.voiceFrom : ""
            duration: player.duration
        }

        onPlayRequested: player.resume()
        onPauseRequested: player.pause()
        onPlayPauseRequested: player.playing ? player.pause() : player.resume()
        onStopRequested: player.stop()
        onRaiseRequested: window.activate()
        onRateRequested: if (!isVideo) voicePlayer.speed = rate
        // Amber's side of all of these is milliseconds, like the players; it converts for D-Bus.
        onPositionRequested: position = player.position
        onSeekRequested: player.seek(Math.max(0, Math.min(player.duration, player.position + offset)))
        onSetPositionRequested: player.seek(Math.max(0, Math.min(player.duration, position)))

        property var seekedSource: Connections {
            target: player
            onSeeked: mpris.seeked(position)
        }
    }
}
