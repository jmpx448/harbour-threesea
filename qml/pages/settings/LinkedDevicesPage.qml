// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import harbour.threesea 1.0
import "../../components/Errors.js" as Errors
import "../../components/Format.js" as Fmt

// Settings → Linked devices (multi-device, ADR 0011): this device and the linked ones (Threema
// for Desktop). Pull-down: link a device, remove all; long press a device: remove it.
Page {
    id: page
    // The window tells of a failed removal unless this page is on top to do it.
    objectName: "linkedDevices"
    allowedOrientations: Orientation.All

    property var devices: []
    property bool active: false
    property bool loading: true
    // The list couldn't be fetched for lack of a connection, or the connection didn't answer.
    property bool offline: false
    // This device's label, as the other devices show it.
    property string label: ""
    // Messages received on another device that this one didn't keep (no room to wait for a contact).
    property int leftOut: 0

    // Keyed, so a reload keeps the rows, and a running remorse with them.
    JsonListModel {
        id: deviceRows
        keyField: "id"
        roles: ["id", "current", "label", "alias", "platform", "platformDetails", "connectedSince", "lastDisconnectAt"]
    }
    onDevicesChanged: deviceRows.update(devices)

    function load() {
        backend.call("multiDevice", {}, function(ok, result) {
            if (!ok) {
                page.loading = false
                return
            }
            page.active = result.active
            page.leftOut = result.reflectedLeftOut || 0
            if (!result.active) {
                page.offline = false
                page.devices = []
                page.loading = false
                return
            }
            backend.call("linkedDevices", {}, function(ok, result, kind, message) {
                page.loading = false
                page.offline = !ok && kind === "network"
                if (ok) {
                    page.devices = result.devices
                    page.label = result.label || ""
                } else {
                    // No list, rather than an old one that may be wrong.
                    page.devices = []
                    if (page.offline) retry.restart()
                    else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    // Here, not in the context menu: its scope is gone when the remorse runs out.
    function remove(item, id) {
        //% "Removing"
        item.remorseAction(qsTrId("threesea-la-removing-device"), function() {
            backend.call("dropDevices", { ids: [id] }, function(ok, result, kind, message) {
                if (ok) {
                    page.devices = result.devices
                    page.active = result.active
                } else if (pageStack.currentPage === page) {
                    Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                }
            })
        })
    }

    // Every other device, then this one; on a failure the choice what next.
    function removeAll() {
        page.loading = true
        backend.call("deactivateMultiDevice", {}, function(ok) {
            page.load()
            if (!ok) page.offerLeaving()
        })
    }

    // Removing them all didn't finish: remove them first (again), or leave here alone, after
    // which this device can't remove them any more. With another page on top the window tells
    // of it, and this page offers it on the next failure.
    function offerLeaving() {
        if (pageStack.currentPage !== page) return
        var dialog = pageStack.push(Qt.resolvedUrl("LeaveDeviceGroupDialog.qml"))
        dialog.accepted.connect(function() {
            if (!dialog.leave) {
                page.removeAll()
                return
            }
            page.loading = true
            backend.call("deactivateMultiDevice", { leaveLocally: true }, function(ok, result, kind, message) {
                if (!ok) Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                page.load()
            })
        })
    }

    // This device: the label the others see (sent with the next connection). Another device:
    // a name for our lists only.
    function rename(device) {
        var dialog = pageStack.push(Qt.resolvedUrl("DeviceNameDialog.qml"),
                                    { name: device.current ? page.label : (device.alias || device.label || ""),
                                      current: device.current === true })
        dialog.accepted.connect(function() {
            var command = device.current ? "setDeviceLabel" : "setDeviceAlias"
            var args = device.current ? { label: dialog.name }
                                      : { device: device.id, alias: dialog.name }
            backend.call(command, args, function(ok, result, kind, message) {
                if (ok) page.load()
                else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
            })
        })
    }

    Component.onCompleted: load()
    onStatusChanged: if (status === PageStatus.Active && !loading) load()

    Connections {
        target: backend
        onCoreEvent: if (name === "multiDeviceChanged") page.load()
        // Back online: the list that couldn't be fetched.
        onConnectionStateChanged: if (backend.connectionState === "connected" && page.offline) page.load()
    }

    // Connected, and the list still timed out (a long backlog after a time offline): no state
    // change will bring it.
    Timer {
        id: retry
        interval: 15000
        onTriggered: if (page.offline && page.status === PageStatus.Active && backend.connectionState === "connected") page.load()
    }

    SilicaListView {
        id: list
        anchors.fill: parent
        model: deviceRows

        PullDownMenu {
            MenuItem {
                visible: page.active && page.leftOut > 0
                //% "Clear count of messages not kept"
                text: qsTrId("threesea-me-clear-reflected-left-out")
                onClicked: backend.call("clearReflectedLeftOut", {}, function(ok, result, kind, message) {
                    if (ok) page.leftOut = result.reflectedLeftOut
                    else Notices.show(Errors.text(kind, message), Notice.Long, Notice.Center)
                })
            }
            MenuItem {
                visible: page.active
                //% "Remove all linked devices"
                text: qsTrId("threesea-me-remove-all-devices")
                //% "Removing all linked devices"
                onClicked: Remorse.popupAction(page, qsTrId("threesea-la-removing-all-devices"), page.removeAll)
            }
            MenuItem {
                //% "Link device"
                text: qsTrId("threesea-me-link-device")
                onClicked: pageStack.push(Qt.resolvedUrl("LinkDeviceDialog.qml"))
            }
        }

        header: Column {
            width: list.width

            PageHeader {
                title: qsTrId("threesea-se-linked-devices")
            }
            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * x
                visible: page.active && page.leftOut > 0
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
                //% "%n message(s) received on your other devices weren't kept on this phone."
                text: qsTrId("threesea-la-reflected-left-out", page.leftOut)
            }
        }

        delegate: ListItem {
            id: item
            contentHeight: Theme.itemSizeMedium
            menu: deviceMenu

            Image {
                id: icon
                x: Theme.horizontalPageMargin
                anchors.verticalCenter: parent.verticalCenter
                source: "image://theme/" + (model.platform === "desktop" || model.platform === "web"
                                            ? "icon-m-computer" : model.current || model.platform === "android"
                                                                  || model.platform === "ios"
                                                                  ? "icon-m-phone" : "icon-m-device")
                        + (item.highlighted ? "?" + Theme.highlightColor : "")
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
                    text: model.alias || model.label || model.platformDetails
                          //% "Device"
                          || qsTrId("threesea-la-device")
                    color: item.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
                Label {
                    width: parent.width
                    truncationMode: TruncationMode.Fade
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: item.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    text: model.current
                          //% "This device"
                          ? qsTrId("threesea-la-this-device")
                          : model.connectedSince
                            //% "Connected"
                            ? qsTrId("threesea-la-device-connected")
                            : model.lastDisconnectAt
                              //% "Last active %1"
                              ? qsTrId("threesea-la-device-last-active").arg(
                                    Fmt.time(model.lastDisconnectAt, Format, Formatter))
                              : ""
                }
            }

            Component {
                id: deviceMenu
                ContextMenu {
                    MenuItem {
                        //% "Rename"
                        text: qsTrId("threesea-me-rename-device")
                        onClicked: page.rename(deviceRows.get(model.index))
                    }
                    MenuItem {
                        visible: !model.current
                        //% "Remove"
                        text: qsTrId("threesea-me-remove-device")
                        onClicked: page.remove(item, model.id)
                    }
                }
            }
        }

        ViewPlaceholder {
            enabled: !page.loading && !page.active
            //% "No linked devices"
            text: qsTrId("threesea-la-no-linked-devices")
            //% "With Threema for Desktop you can read and write your chats on a computer. Open it there and pull down here to link it."
            hintText: qsTrId("threesea-la-no-linked-devices-hint")
        }

        ViewPlaceholder {
            enabled: !page.loading && page.active && page.devices.length === 0
            text: page.offline
                  //% "Waiting for the connection"
                  ? qsTrId("threesea-la-linked-devices-offline")
                  //% "The list of linked devices did not load"
                  : qsTrId("threesea-la-linked-devices-failed")
            hintText: page.offline
                      //% "The linked devices appear here once the server answers."
                      ? qsTrId("threesea-la-linked-devices-offline-hint")
                      //% "Open this page again to load it."
                      : qsTrId("threesea-la-linked-devices-failed-hint")
        }

        BusyIndicator {
            anchors.centerIn: parent
            size: BusyIndicatorSize.Large
            running: page.loading
        }

        VerticalScrollDecorator { }
    }
}
