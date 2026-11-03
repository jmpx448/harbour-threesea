// SPDX-License-Identifier: AGPL-3.0-only
import QtQuick 2.6
import Sailfish.Silica 1.0

// Settings → About: version, the European Commission's "AI generated" label, the disclaimer,
// credits and the license (the same as the README).
Page {
    allowedOrientations: Orientation.All

    SilicaFlickable {
        anchors.fill: parent
        contentHeight: column.height + Theme.paddingLarge

        Column {
            id: column
            width: parent.width
            spacing: Theme.paddingMedium

            PageHeader {
                //% "About"
                title: qsTrId("threesea-he-about")
            }

            DetailItem {
                //% "Version"
                label: qsTrId("threesea-la-version")
                value: backend.coreVersion
            }

            // The EU icon in the colour that stands out on the ambience.
            Image {
                anchors.horizontalCenter: parent.horizontalCenter
                height: Theme.itemSizeExtraSmall
                fillMode: Image.PreserveAspectFit
                sourceSize.height: height
                source: Theme.colorScheme === Theme.LightOnDark ? "../icons/ai-generated-white.png"
                                                                : "../icons/ai-generated-black.png"
            }

            SectionHeader {
                //% "Disclaimer"
                text: qsTrId("threesea-se-disclaimer")
            }

            Repeater {
                //% "Threesea was developed with Claude Code (Anthropic) and audited with OpenAI Codex, directed and tested by a human maintainer. It is a research and experimentation project."
                model: [qsTrId("threesea-la-disclaimer-ai"),
                        //% "AI makes mistakes. The code, including its security-relevant parts, can contain errors nobody noticed. The developers give no warranty of any kind; you use the app at your own risk."
                        qsTrId("threesea-la-disclaimer-warranty"),
                        //% "Threesea is based on Threema Libre's open-source code and tries to play fair with Threema's APIs and licensing. Threema told us it doesn't actively block third-party apps that cause no stability issues or high server load, are clearly marked as unofficial without using its branding, and don't circumvent the licensing. Threesea aims to meet that. This is not a permission, and Threesea is not affiliated with, endorsed or supported by Threema AG. As with any Threema app, keep a backup of your ID."
                        qsTrId("threesea-la-disclaimer-threema")]
                Label {
                    x: Theme.horizontalPageMargin
                    width: column.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeSmall
                    color: Theme.highlightColor
                    text: modelData
                }
            }

            SectionHeader {
                //% "Credits"
                text: qsTrId("threesea-se-credits")
            }

            Repeater {
                //% "Threema AG: threema-android and libthreema (AGPL-3.0), the protocols of Threema, Threema for Desktop and Threema Web."
                model: [qsTrId("threesea-la-credit-threema"),
                        // Required attribution (CC-BY 4.0), as in Threema Libre's licence page.
                        //% "Emoji graphics by Twemoji (github.com/jdecked/twemoji), licensed under CC-BY 4.0."
                        qsTrId("threesea-la-twemoji-credit"),
                        //% "opus-pure by Stephen Berry, from Rust ports of libopus by the Xiph.Org Foundation and others (BSD-3-Clause): voice messages."
                        qsTrId("threesea-la-credit-opus"),
                        //% "webrtc-audio-processing, from Google's WebRTC (BSD-3-Clause): echo cancellation in calls."
                        qsTrId("threesea-la-credit-webrtc"),
                        //% "Jolla and Sailfish OS: Silica and its icons; Jolla's Messages, People, Settings and Gallery as the model for the design."
                        qsTrId("threesea-la-credit-jolla"),
                        //% "Whisperfish, the Signal client for Sailfish OS: a reference for building such an app."
                        qsTrId("threesea-la-credit-whisperfish"),
                        //% "SQLCipher, GStreamer, Qt and many Rust libraries (tokio, rustls, reqwest, rusqlite, RustCrypto and others)."
                        qsTrId("threesea-la-credit-libraries"),
                        //% "The “AI generated” label is the European Commission's icon for AI-generated content."
                        qsTrId("threesea-la-credit-eu-icon")]
                Label {
                    x: Theme.horizontalPageMargin
                    width: column.width - 2 * Theme.horizontalPageMargin
                    wrapMode: Text.Wrap
                    font.pixelSize: Theme.fontSizeExtraSmall
                    color: Theme.secondaryHighlightColor
                    text: modelData
                }
            }

            SectionHeader {
                //% "License"
                text: qsTrId("threesea-se-license")
            }

            Label {
                x: Theme.horizontalPageMargin
                width: parent.width - 2 * Theme.horizontalPageMargin
                wrapMode: Text.Wrap
                font.pixelSize: Theme.fontSizeSmall
                color: Theme.highlightColor
                //% "An unofficial Threema client for Sailfish OS, licensed under the GNU AGPL v3. Uses libthreema by Threema AG. Requires a Threema Libre license from the Threema Shop. Not affiliated with or supported by Threema AG."
                text: qsTrId("threesea-la-license-text")
            }
        }

        VerticalScrollDecorator { }
    }
}
