// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components/Errors.js" as Errors
import "../../components/Format.js" as Fmt

// Threema Web (ADR 0013): the switch and the browsers this phone serves, like Linked devices. Pull
// down to scan the code web.threema.ch shows; tap a session to connect or disconnect it; long press
// to remove.
Page {
    id: page
    // The window opens this page from its notifications, unless it is on top already.
    objectName: "webSessions"
    allowedOrientations: Orientation.All

    // The core's switch; off, nothing connects.
    property bool webOn: true
    property var sessions: []
    // The self-hosted servers the user allowed.
    property var servers: []
    // New certificates of those servers, waiting for the user's word.
    property var certificates: []
    property bool reviewing: false

    function load(done) {
        backend.call("webSessions", {}, function(ok, result) {
            if (ok) {
                page.webOn = result.enabled
                page.sessions = result.sessions
                page.servers = result.servers
                page.certificates = result.certificates
            }
            if (done) done()
        })
    }

    function reviewCertificate() {
        var certificate = certificates[0]
        reviewing = true
        window.webCertificateSeen()
        var answered = function() {
            page.reviewing = false
            page.load()
        }
        var dialog = pageStack.push(Qt.resolvedUrl("WebHostDialog.qml"), { host: certificate.host })
        dialog.accepted.connect(function() {
            backend.call("webAcceptCertificate", { session: certificate.id }, function(ok, result, kind, message) {
                if (!ok) {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                } else if (result.scanAgain) {
                    //% "The new certificate is trusted. Scan the code in the browser again to connect it."
                    Notices.show(qsTrId("threesea-la-web-scan-again"), Notice.Long, Notice.Center)
                }
                answered()
            })
        })
        dialog.rejected.connect(function() {
            backend.call("webDeclineCertificate", { session: certificate.id }, answered)
        })
    }
    // One at a time, and only once this page is on top: a push may have woken the session, or
    // the scanner may still be closing.
    Timer {
        running: page.certificates.length > 0 && !page.reviewing
                 && page.status === PageStatus.Active && !pageStack.busy
        interval: 100
        onTriggered: page.reviewCertificate()
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

    // Here, not in the context menu: its scope is gone when the remorse runs out.
    function removeServer(item, host) {
        //% "Removing"
        item.remorseAction(qsTrId("threesea-la-web-removing"), function() {
            backend.call("webRemoveServer", { host: host }, function(ok, result, kind, message) {
                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                page.load()
            })
        })
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
                page.load()
            }
        }
    }

    SilicaListView {
        id: list
        anchors.fill: parent
        model: page.sessions

        PullDownMenu {
            visible: page.webOn
            MenuItem {
                //% "Scan Threema Web code"
                text: qsTrId("threesea-me-scan-web-code")
                onClicked: {
                    var scan = pageStack.push(Qt.resolvedUrl("../ScanQrPage.qml"), { webPair: true })
                    scan.webPaired.connect(page.load)
                }
            }
        }

        header: Column {
            width: parent.width

            PageHeader {
                //% "Threema Web"
                title: qsTrId("threesea-he-threema-web")
            }
            TextSwitch {
                automaticCheck: false
                checked: page.webOn
                //% "Threema Web"
                text: qsTrId("threesea-la-web-enabled")
                //% "Lets the browsers listed here connect to this phone and use your chats. Turning it off disconnects them and keeps them in the list."
                description: qsTrId("threesea-la-web-enabled-description")
                onClicked: backend.call("webSetEnabled", { enabled: !checked }, function(ok, result, kind, message) {
                    if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                    page.load()
                })
            }
        }

        ViewPlaceholder {
            enabled: page.webOn && list.count === 0 && page.servers.length === 0
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

        footer: Column {
            width: parent.width
            visible: page.servers.length > 0

            SectionHeader {
                //% "Allowed servers"
                text: qsTrId("threesea-se-web-servers")
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeExtraSmall
                color: Theme.secondaryHighlightColor
                //% "Self-hosted Threema Web servers you allowed. Removing one also removes its browsers, and the next code naming it asks again."
                text: qsTrId("threesea-la-web-servers-description")
            }
            Repeater {
                model: page.servers
                ListItem {
                    id: server
                    contentHeight: Theme.itemSizeSmall
                    menu: ContextMenu {
                        MenuItem {
                            text: qsTrId("threesea-me-web-remove")
                            onClicked: page.removeServer(server, modelData)
                        }
                    }
                    Label {
                        x: Theme.horizontalPageMargin
                        width: parent.width - 2 * x
                        anchors.verticalCenter: parent.verticalCenter
                        truncationMode: TruncationMode.Fade
                        color: server.highlighted ? Theme.highlightColor : Theme.primaryColor
                        text: modelData
                    }
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
