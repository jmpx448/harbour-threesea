// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../components/Errors.js" as Errors

// Scan a contact's QR code to verify them in person (and add them if new), or (deviceLink)
// the code Threema for Desktop shows to link it, or (webPair) the code of Threema Web, or
// (anyCode, from the contacts page) whichever of the three it is. Looks at the camera a few
// times a second; a picture (e.g. a screenshot someone sent) works too.
Page {
    id: page
    allowedOrientations: Orientation.Portrait

    // Only accept this contact's code (when verifying a known contact), else anyone's.
    property string expectedIdentity
    property bool busy: false

    // Linking Threema for Desktop: any code but a contact's continues to the linking page.
    property bool deviceLink
    property string scannedCode

    // Threema Web: pair the browser whose code this is.
    property bool webPair

    // Any of the three: contacts are verified here, Threema Web pairs and then shows its
    // sessions, Threema for Desktop continues to the linking dialog.
    property bool anyCode

    signal verified(var contact)
    signal webPaired()

    function pairWeb(text, allowHost) {
        backend.call("webPair", { qr: text, allowHost: allowHost }, function(ok, result, kind, message) {
            if (!ok) {
                page.busy = false
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                return
            }
            if (result.confirmHost) {
                var dialog = pageStack.push(Qt.resolvedUrl("settings/WebHostDialog.qml"), { host: result.confirmHost })
                dialog.accepted.connect(function() { page.pairWeb(text, true) })
                dialog.rejected.connect(function() { page.busy = false })
                return
            }
            page.webPaired()
            if (page.anyCode) showWebSessions.start()
            else closeWhenIdle.start()
        })
    }

    function check(text) {
        if (anyCode && text.indexOf("3mid:") !== 0) {
            if (busy) return false
            busy = true
            backend.call("qrKind", { text: text }, function(ok, result) {
                var kind = ok ? result.kind : "unknown"
                if (kind === "web") {
                    page.pairWeb(text, false)
                } else if (kind === "device") {
                    page.scannedCode = text
                    continueLinking.start()
                } else {
                    page.busy = false
                }
            })
            return true
        }
        if (webPair) {
            if (busy) return false
            if (text.indexOf("3mid:") === 0) {
                //% "This is a contact's code. Scan the code web.threema.ch shows."
                Notices.show(qsTrId("threesea-la-qr-not-web"), Notice.Short, Notice.Center)
                return true
            }
            busy = true
            pairWeb(text, false)
            return true
        }
        if (deviceLink) {
            if (busy) return false
            if (text.indexOf("3mid:") === 0) {
                //% "This is a contact's code. Scan the code on the computer."
                Notices.show(qsTrId("threesea-la-qr-not-device"), Notice.Short, Notice.Center)
                return true
            }
            busy = true
            scannedCode = text
            continueLinking.start()
            return true
        }
        if (busy || text.indexOf("3mid:") !== 0) return false
        var identity = text.substring(5, 13).toUpperCase()
        if (expectedIdentity && identity !== expectedIdentity) {
            //% "This is someone else's code."
            Notices.show(qsTrId("threesea-la-qr-other-contact"), Notice.Short, Notice.Center)
            return true
        }
        busy = true
        backend.call("verifyQrCode", { text: text }, function(ok, result, kind, message) {
            page.busy = false
            if (ok) {
                //% "%1 is verified"
                Notices.show(qsTrId("threesea-la-qr-verified").arg(result.displayName), Notice.Short, Notice.Center)
                backend.refreshContacts()
                page.verified(result)
                closeWhenIdle.start()
            } else {
                Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            }
        })
        return true
    }

    // On to the linking page once the picker (if any) has gone.
    Timer {
        id: continueLinking
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            if (pageStack.currentPage !== page) {
                pageStack.pop(page, PageStackAction.Immediate)
                return
            }
            stop()
            // From the contacts page: what linking means first, like Linked devices does.
            if (page.anyCode) pageStack.replace(Qt.resolvedUrl("settings/LinkDeviceDialog.qml"), { code: page.scannedCode })
            else pageStack.replace(Qt.resolvedUrl("settings/LinkDevicePage.qml"), { code: page.scannedCode })
        }
    }

    // Threema Web paired from the contacts page: its sessions, once the picker (if any) has gone.
    Timer {
        id: showWebSessions
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            if (pageStack.currentPage !== page) {
                pageStack.pop(page, PageStackAction.Immediate)
                return
            }
            stop()
            pageStack.replace(Qt.resolvedUrl("settings/WebSessionsPage.qml"))
        }
    }

    // Back to the contact once the picker (if any) has gone.
    Timer {
        id: closeWhenIdle
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy) return
            stop()
            pageStack.pop(page, PageStackAction.Immediate)
            pageStack.pop()
        }
    }

    Camera {
        id: camera
        captureMode: Camera.CaptureViewfinder
        focus.focusMode: Camera.FocusContinuous
    }

    SilicaFlickable {
        anchors.fill: parent

        PullDownMenu {
            MenuItem {
                //% "Scan from picture"
                text: qsTrId("threesea-me-scan-from-picture")
                onClicked: pageStack.push(picturePicker)
            }
        }

        PageHeader {
            id: header
            //% "Scan QR code"
            title: qsTrId("threesea-he-scan-qr")
        }

        VideoOutput {
            id: viewfinder
            anchors {
                top: header.bottom
                left: parent.left
                right: parent.right
                bottom: hint.top
                bottomMargin: Theme.paddingLarge
            }
            source: camera
            fillMode: VideoOutput.PreserveAspectCrop
            visible: camera.availability === Camera.Available
        }

        Label {
            anchors.centerIn: viewfinder
            width: parent.width - 2 * Theme.horizontalPageMargin
            visible: !viewfinder.visible
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            //% "No camera available. Pull down to scan a code from a picture."
            text: qsTrId("threesea-la-no-camera")
            color: Theme.secondaryHighlightColor
        }

        Label {
            id: hint
            anchors {
                bottom: parent.bottom
                bottomMargin: Theme.paddingLarge
            }
            x: Theme.horizontalPageMargin
            width: parent.width - 2 * Theme.horizontalPageMargin
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            text: page.deviceLink
                  //% "Point the camera at the QR code Threema for Desktop shows."
                  ? qsTrId("threesea-la-scan-hint-device")
                  : page.anyCode
                  //% "Point the camera at a contact's Threema QR code, or at the one Threema Web or Threema for Desktop shows."
                  ? qsTrId("threesea-la-scan-hint-any")
                  //% "Point the camera at the Threema QR code of the person next to you."
                  : qsTrId("threesea-la-scan-hint")
            color: Theme.secondaryHighlightColor
            font.pixelSize: Theme.fontSizeSmall
        }
    }

    Timer {
        interval: 600
        repeat: true
        running: viewfinder.visible && page.status === PageStatus.Active && !page.busy
        onTriggered: viewfinder.grabToImage(function(result) {
            var text = backend.decodeQr(result.image)
            if (text) page.check(text)
        })
    }

    Component {
        id: picturePicker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                var text = backend.decodeQrFile(selectedContentProperties.filePath)
                if (!text || !page.check(text)) {
                    //% "No Threema QR code found in this picture."
                    Notices.show(qsTrId("threesea-la-qr-not-found"), Notice.Long, Notice.Center)
                }
            }
        }
    }
}
