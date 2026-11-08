// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6

// For a list view whose header changes height after the first layout (a Column's first height
// lacks what comes later: a DialogHeader's own height, a row that appears when its data arrives).
// ListView leaves contentY alone when its header grows, so the list comes to rest past its top: a
// pull-down menu only opens exactly at the top, and the header's first lines are out of sight. A
// list that was at its top stays there; one that was scrolled stays where it is.
Connections {
    // Where the top was before this change; nothing known yet counts as at the top.
    property real _top: NaN

    onOriginYChanged: {
        var wasAtTop = isNaN(_top) || Math.abs(target.contentY - _top) < 1
        _top = target.originY
        if (wasAtTop && !target.moving) target.contentY = _top
    }
}
