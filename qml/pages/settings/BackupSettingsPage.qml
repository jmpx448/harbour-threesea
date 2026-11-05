// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0
import "../../components"

// Settings → Backup: Threema Safe and the ID backup.
Page {
    id: page
    allowedOrientations: Orientation.All

    // Back from the page above, however it went.
    onStatusChanged: if (status === PageStatus.Active) safeItem.refresh()

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width

            PageHeader {
                //% "Backup"
                title: qsTrId("threesea-se-backup")
            }

            BackgroundItem {
                id: safeItem
                width: parent.width
                height: Theme.itemSizeMedium
                onClicked: window.pushWithPin(Qt.resolvedUrl("../SafePage.qml"))

                property bool safeEnabled: false
                property bool safeFailing: false
                Component.onCompleted: refresh()
                function refresh() {
                    backend.call("safeStatus", {}, function(ok, result) {
                        if (ok) {
                            safeItem.safeEnabled = result.enabled
                            safeItem.safeFailing = result.enabled && !!result.lastError
                        }
                    })
                }
                Connections {
                    target: backend
                    onCoreEvent: if (name === "safeChanged") safeItem.refresh()
                }

                Column {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    Label {
                        width: parent.width
                        //% "Threema Safe"
                        text: qsTrId("threesea-la-threema-safe")
                        color: safeItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                    }
                    Label {
                        width: parent.width
                        text: safeItem.safeFailing
                              //% "On, the last attempt failed"
                              ? qsTrId("threesea-la-safe-on-failing")
                              : safeItem.safeEnabled
                                //% "On"
                                ? qsTrId("threesea-la-on")
                                //% "Off"
                                : qsTrId("threesea-la-off")
                        font.pixelSize: Theme.fontSizeSmall
                        color: safeItem.safeFailing ? Theme.errorColor
                               : safeItem.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
                    }
                }
            }

            BackgroundItem {
                width: parent.width
                onClicked: window.pushWithPin(Qt.resolvedUrl("../ExportBackupPage.qml"))

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    //% "Export ID backup"
                    text: qsTrId("threesea-la-export-backup")
                    color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }

            BackgroundItem {
                width: parent.width
                onClicked: window.pushWithPin(Qt.resolvedUrl("../DataBackupPage.qml"))

                Label {
                    x: Theme.horizontalPageMargin
                    width: parent.width - 2 * Theme.horizontalPageMargin
                    anchors.verticalCenter: parent.verticalCenter
                    //% "Data backup"
                    text: qsTrId("threesea-he-data-backup")
                    color: parent.highlighted ? Theme.highlightColor : Theme.primaryColor
                }
            }
        }

        VerticalScrollDecorator { }
    }
}
