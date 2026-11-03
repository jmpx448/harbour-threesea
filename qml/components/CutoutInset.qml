// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// The room a display cutout ("notch") needs above custom top content. Silica already shrinks
// landscape pages away from the cutout (CutoutMode.AvoidLandscapeCutout is the default) and
// pads its own PageHeader/DialogHeader, pulley menus and remorse popups in upright portrait
// (SFOS 5.1 Page.qml/PageHeader.qml: the public Screen.topCutout); a layout without a standard
// header adds `inset` to its top spacing itself. 0 without a cutout, in every other orientation,
// and outside a page.
Item {
    visible: false
    width: 0
    height: 0

    readonly property real inset: _page && _page.orientation === Orientation.Portrait
                                  ? Screen.topCutout.height : 0

    // The page above us, found like Silica's own Util.findPage (the Page type's marker
    // property; no private import). The tree above is complete by onCompleted.
    property Item _page
    Component.onCompleted: {
        var item = parent
        while (item && !item.hasOwnProperty("__silica_page")) item = item.parent
        _page = item
    }
}
