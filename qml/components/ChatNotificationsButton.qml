// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "Format.js" as Fmt

// A chat's notifications, like Threema Android's per-chat setting: on, off for a while, off,
// and for groups only when we are mentioned. Applies at once (ComboBox, like Jolla Settings).
// `settings`: `{policy, until}` from the core.
ChoiceBox {
    id: root

    property string identity
    property bool group
    property var settings: ({})

    // Off for 1 hour, 8 hours, 1 day, 1 week (options 1–4).
    readonly property var durations: [3600000, 8 * 3600000, 24 * 3600000, 7 * 24 * 3600000]
    options: {
        var list = [
            //% "On"
            qsTrId("threesea-me-notifications-on"),
            //% "Off for 1 hour"
            qsTrId("threesea-me-notifications-1-hour"),
            //% "Off for 8 hours"
            qsTrId("threesea-me-notifications-8-hours"),
            //% "Off for 1 day"
            qsTrId("threesea-me-notifications-1-day"),
            //% "Off for 1 week"
            qsTrId("threesea-me-notifications-1-week"),
            //% "Off"
            qsTrId("threesea-me-notifications-off")
        ]
        //% "Only when I am mentioned"
        if (group) list.push(qsTrId("threesea-me-notifications-mentioned"))
        return list
    }

    // The option of the current setting; a timed one shows the next longer duration still left.
    choice: {
        var policy = settings ? settings.policy : null
        if (policy === "mentioned") return 6
        if (policy === "never" && settings.until) {
            var left = settings.until - Date.now()
            for (var i = 0; i < durations.length; ++i) {
                if (durations[i] >= left) return i + 1
            }
            return durations.length
        }
        return policy === "never" ? 5 : 0
    }

    function apply(option) {
        var policy = option === 0 ? null : option === 6 ? "mentioned" : "never"
        var duration = option >= 1 && option <= 4 ? durations[option - 1] : 0
        backend.call("setChatNotifications", {
            identity: identity,
            policy: policy,
            until: duration ? Date.now() + duration : null
        }, function(ok, result) {
            if (ok) root.settings = result
        })
    }

    //% "Notifications"
    label: qsTrId("threesea-la-chat-notifications")
    valueText: settings && settings.policy === "never" && settings.until
               //% "Until %1"
               ? qsTrId("threesea-la-notifications-until").arg(Fmt.time(settings.until, Format, Formatter))
               : ""
    onChosen: apply(index)
}
