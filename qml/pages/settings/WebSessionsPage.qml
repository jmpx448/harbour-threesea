// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components/Errors.js" as Errors
import "../../components/Format.js" as Fmt

// Threema Web (ADR 0013): the browsers this phone serves, like Linked devices. Pull down to scan
// the code web.threema.ch shows; tap a session to connect or disconnect it; long press to remove.
Page {
    id: page
    allowedOrientations: Orientation.All

    property var sessions: []

    function load(done) {
        backend.call("webSessions", {}, function(ok, result) {
            if (ok) page.sessions = result.sessions
            if (done) done()
        })
    }

    // A session the browser never stored is gone from the list by now.
    function showDisconnected(id) {
        var kept = page.sessions.some(function(session) { return session.id === id })
        Notices.show(kept
                     //% "Threema Web disconnected. Tap the browser to connect it again."
                     ? qsTrId("threesea-la-web-disconnected-kept")
                     //% "Threema Web disconnected."
                     : qsTrId("threesea-la-web-disconnected"), Notice.Long, Notice.Center)
    }

    function stateText(session) {
        switch (session.state) {
        case "connecting":
            //% "Connecting…"
            return qsTrId("threesea-la-web-connecting")
        case "waiting":
            //% "Waiting for the browser"
            return qsTrId("threesea-la-web-waiting")
        case "connected":
            //% "Connected"
            return qsTrId("threesea-la-web-connected")
        }
        return session.lastConnection
                //% "Last used %1"
                ? qsTrId("threesea-la-web-last-used").arg(Fmt.time(session.lastConnection, Format, Formatter))
                //% "Not connected"
                : qsTrId("threesea-la-web-not-connected")
    }

    Component.onCompleted: load()

    Connections {
        target: backend
        onCoreEvent: {
            if (name === "webSession") {
                // `error` is a code for the log's sake; a changed certificate has its own dialog.
                var failed = data.state === "stopped" && data.error && data.error !== "certificateChanged"
                page.load(failed ? function() { page.showDisconnected(data.id) } : undefined)
            } else if (name === "webCertificateChanged") {
                var dialog = pageStack.push(Qt.resolvedUrl("WebHostDialog.qml"), { host: data.host })
                dialog.accepted.connect(function() {
                    backend.call("webAcceptCertificate", { session: data.id }, function(ok, result, kind, message) {
                        if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                    })
                })
            }
        }
    }

    SilicaListView {
        id: list
        anchors.fill: parent
        model: page.sessions

        PullDownMenu {
            MenuItem {
                //% "Scan Threema Web code"
                text: qsTrId("threesea-me-scan-web-code")
                onClicked: {
                    var scan = pageStack.push(Qt.resolvedUrl("../ScanQrPage.qml"), { webPair: true })
                    scan.webPaired.connect(page.load)
                }
            }
        }

        header: PageHeader {
            //% "Threema Web"
            title: qsTrId("threesea-he-threema-web")
        }

        ViewPlaceholder {
            enabled: list.count === 0
            //% "No browsers"
            text: qsTrId("threesea-la-web-none")
            //% "Open web.threema.ch in a browser and pull down to scan its code"
            hintText: qsTrId("threesea-la-web-none-hint")
        }

        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeMedium
            menu: ContextMenu {
                MenuItem {
                    //% "Remove"
                    text: qsTrId("threesea-me-web-remove")
                    onClicked: item.remorseDelete(function() {
                        backend.call("webDelete", { session: modelData.id }, function() { page.load() })
                    })
                }
            }
            onClicked: backend.call(modelData.state === "stopped" ? "webStart" : "webStop",
                                    { session: modelData.id }, function(ok, result, kind, message) {
                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                page.load()
            })

            Icon {
                id: icon
                x: Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                source: "image://theme/icon-m-website"
                highlighted: item.highlighted || modelData.state === "connected"
            }
            Column {
                anchors {
                    left: icon.right
                    leftMargin: Theme.paddingLarge
                    right: parent.right
                    rightMargin: Theme.horizontalPageMargin
                    verticalCenter: parent.verticalCenter
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                    text: modelData.client
                          ? modelData.client.charAt(0).toUpperCase() + modelData.client.slice(1)
                          //% "Browser"
                          : qsTrId("threesea-la-web-browser")
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    text: page.stateText(modelData) + (modelData.selfHosted ? " · " + modelData.host : "")
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
