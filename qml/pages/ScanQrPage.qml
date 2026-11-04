// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import QtMultimedia 5.6
import Sailfish.Silica 1.0
import Sailfish.Pickers 1.0
import "../components/Errors.js" as Errors

// Scan a contact's QR code to verify them in person (and add them if new), or (deviceLink)
// the code Threema for Desktop shows to link it, or (webPair) the code of Threema Web, or
// (anyCode, from the contacts page) whichever it is, a link to a Threema ID too. Looks at the
// camera a few times a second; a picture (e.g. a screenshot someone sent) works too.
//
// Everything goes one way: the core says what a code is (qrKind), refusal() says what this
// scanner can't do with it, and what connects a browser or adds from a link asks first.
Page {
    id: page
    allowedOrientations: Orientation.Portrait

    // Only accept this contact's code (when verifying a known contact), else anyone's.
    property string expectedIdentity
    property bool busy: false
    // The camera runs only while the page can be seen.
    readonly property bool shown: Qt.application.state === Qt.ApplicationActive && status === PageStatus.Active
    // A frame is being looked at: the next one can wait.
    property bool decoding: false
    // A grab asked for while the window went away may never come back.
    onShownChanged: decoding = false

    // Linking Threema for Desktop: only its code continues to the linking page.
    property bool deviceLink

    // Threema Web: pair the browser whose code this is.
    property bool webPair

    // Any of them: contacts are verified here, Threema Web pairs and then shows its sessions,
    // Threema for Desktop continues to the linking dialog, a link adds the contact.
    property bool anyCode

    signal verified(var contact)
    signal webPaired()

    // A code refused or declined stays quiet while the camera still sees it: no notice twice a
    // second, no dialog coming back. Frames fail to decode now and then, so it takes several in
    // a row without it to count as gone, and only frames of a running camera: it restarts when a
    // dialog closes. After a failure that may pass, retryAt tries it again.
    property string restingCode
    property double retryAt
    property int framesWithoutCode

    function rest(text, mayPass) {
        restingCode = text
        retryAt = mayPass ? Date.now() + 5000 : 0
        framesWithoutCode = 0
    }

    function cameraFrame(text) {
        if (busy || !shown) return
        if (!text) {
            if (restingCode && camera.cameraStatus === Camera.ActiveStatus && ++framesWithoutCode >= 5) {
                restingCode = ""
            }
            return
        }
        if (text === restingCode) {
            framesWithoutCode = 0
            if (retryAt === 0 || Date.now() < retryAt) return
        }
        restingCode = ""
        check(text, false)
    }

    // Not this code. A picture was chosen and always gets an answer; there is nothing to rest.
    function refuse(message, text, fromPicture, mayPass) {
        busy = false
        if (!fromPicture) rest(text, mayPass === true)
        Notices.show(message, Notice.Long, Notice.Center)
    }

    function fail(kind, message, text, fromPicture) {
        refuse(Errors.text(kind, message), text, fromPicture, kind === "network" || kind === "server")
    }

    function decline(text, fromPicture) {
        busy = false
        if (!fromPicture) rest(text, false)
    }

    // Why this scanner won't go on with the code, or nothing.
    function refusal(code, fromPicture) {
        var someones = code.kind === "contact" || code.kind === "link"
        if (webPair && code.kind !== "web") {
            return someones
                    //% "This is a contact's code. Scan the code web.threema.ch shows."
                    ? qsTrId("threesea-la-qr-not-web")
                    : Errors.text("webCodeInvalid")
        }
        if (deviceLink && code.kind !== "device") {
            return someones
                    //% "This is a contact's code. Scan the code on the computer."
                    ? qsTrId("threesea-la-qr-not-device")
                    : Errors.text(code.kind === "web" ? "linkWebCode" : "linkUnknownCode")
        }
        var verifying = !anyCode && !webPair && !deviceLink
        if (verifying && code.kind === "link") {
            //% "This is a link to a Threema ID. To verify it, scan the QR code in the other person's app."
            return qsTrId("threesea-la-qr-link-verify")
        }
        if (verifying && code.kind !== "contact") return Errors.text("qrInvalid")
        if (code.error) return Errors.text(code.error)
        if (code.kind === "unknown") {
            return fromPicture
                    //% "No Threema QR code found in this picture."
                    ? qsTrId("threesea-la-qr-not-found")
                    //% "This is not a Threema QR code."
                    : qsTrId("threesea-la-qr-unknown")
        }
        if (verifying && expectedIdentity && code.identity !== expectedIdentity) {
            //% "This is someone else's code."
            return qsTrId("threesea-la-qr-other-contact")
        }
        return ""
    }

    function check(text, fromPicture) {
        if (busy) return
        busy = true
        backend.call("qrKind", { text: text }, function(ok, code) {
            if (!ok) code = { kind: "unknown" }
            var message = page.refusal(code, fromPicture)
            if (message) page.refuse(message, text, fromPicture)
            else if (code.kind === "web") page.askWeb(code, text, fromPicture)
            else if (code.kind === "device") page.continueLinking(text)
            else if (code.kind === "link") page.askAddContact(code.identity, text, fromPicture)
            else page.verify(text, fromPicture)
        })
    }

    function askWeb(code, text, fromPicture) {
        whenOnTop.run(function() {
            var dialog = pageStack.push(Qt.resolvedUrl("settings/WebPairDialog.qml"),
                                        { host: code.confirmHost ? code.host : "" })
            dialog.accepted.connect(function() { page.pairWeb(code, text, fromPicture) })
            dialog.rejected.connect(function() { page.decline(text, fromPicture) })
        })
    }

    function pairWeb(code, text, fromPicture) {
        backend.call("webPair", { qr: text, allowHost: code.confirmHost === true }, function(ok, result, kind, message) {
            if (!ok) {
                page.fail(kind, message, text, fromPicture)
            } else if (result.confirmHost) {
                // The server's permission is gone since the question.
                code.confirmHost = true
                code.host = result.confirmHost
                page.askWeb(code, text, fromPicture)
            } else {
                page.webPaired()
                if (page.anyCode) {
                    whenOnTop.run(function() { pageStack.replace(Qt.resolvedUrl("settings/WebSessionsPage.qml")) })
                } else {
                    closeWhenIdle.start()
                }
            }
        })
    }

    function continueLinking(text) {
        whenOnTop.run(function() {
            // From the contacts page: what linking means first, like Linked devices does.
            if (page.anyCode) pageStack.replace(Qt.resolvedUrl("settings/LinkDeviceDialog.qml"), { code: text })
            else pageStack.replace(Qt.resolvedUrl("settings/LinkDevicePage.qml"), { code: text })
        })
    }

    function askAddContact(identity, text, fromPicture) {
        whenOnTop.run(function() {
            var dialog = pageStack.push(Qt.resolvedUrl("AddScannedContactDialog.qml"), { identity: identity })
            dialog.accepted.connect(function() { page.addContact(identity, text, fromPicture) })
            dialog.rejected.connect(function() { page.decline(text, fromPicture) })
        })
    }

    function addContact(identity, text, fromPicture) {
        backend.call("addContact", { identity: identity }, function(ok, result, kind, message) {
            if (!ok) {
                page.fail(kind, message, text, fromPicture)
                return
            }
            backend.refreshContacts()
            whenOnTop.run(function() {
                pageStack.replace(Qt.resolvedUrl("ContactPage.qml"), { identity: identity })
            })
        })
    }

    function verify(text, fromPicture) {
        backend.call("verifyQrCode", { text: text }, function(ok, result, kind, message) {
            if (ok) {
                page.busy = false
                //% "%1 is verified"
                Notices.show(qsTrId("threesea-la-qr-verified").arg(result.displayName), Notice.Short, Notice.Center)
                backend.refreshContacts()
                page.verified(result)
                closeWhenIdle.start()
            } else {
                page.fail(kind, message, text, fromPicture)
            }
        })
    }

    // Once the picker (if any) and any transition have gone, with this page on top again.
    Timer {
        id: whenOnTop
        interval: 100
        repeat: true
        property var action
        function run(what) {
            action = what
            start()
        }
        onTriggered: {
            if (pageStack.busy || window.stackHeld) return
            if (pageStack.currentPage !== page) {
                pageStack.pop(page, PageStackAction.Immediate)
                return
            }
            stop()
            action()
        }
    }

    // Back to the contact once the picker (if any) has gone.
    Timer {
        id: closeWhenIdle
        interval: 100
        repeat: true
        onTriggered: {
            if (pageStack.busy || window.stackHeld) return
            stop()
            pageStack.pop(page, PageStackAction.Immediate)
            pageStack.pop()
        }
    }

    Camera {
        id: camera
        // Only while on screen: the link dialog creates this page in advance, and a camera
        // defaults to running. Unloaded rather than Loaded, which would still hold the device.
        cameraState: page.shown ? Camera.ActiveState : Camera.UnloadedState
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
                  : page.webPair
                  //% "Point the camera at the QR code Threema Web shows."
                  ? qsTrId("threesea-la-scan-hint-web")
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
        running: viewfinder.visible && page.shown && !page.busy && !page.decoding
        onTriggered: {
            page.decoding = viewfinder.grabToImage(function(result) {
                backend.decodeQr(result.image, function(text) {
                    page.decoding = false
                    page.cameraFrame(text)
                })
            })
        }
    }

    Component {
        id: picturePicker
        ImagePickerPage {
            onSelectedContentPropertiesChanged: {
                var text = backend.decodeQrFile(selectedContentProperties.filePath)
                if (text) page.check(text, true)
                else Notices.show(qsTrId("threesea-la-qr-not-found"), Notice.Long, Notice.Center)
            }
        }
    }
}
