// SPDX-License-Identifier: AGPL-3.0-only
//! What a received file is, as far as it matters on this phone. A sender states a media type and
//! a name independently, and the system picks the app for a file by its name alone. So its name
//! decides, one checked type for the icon and for handing it to other apps, whatever the media
//! type claims.

use std::path::Path;

use serde_json::Value;

/// A kind of file, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileClass {
    Image,
    Audio,
    Video,
    Pdf,
    Document,
    Contact,
    Archive,
    Other,
}

impl FileClass {
    /// `body.fileType` in the UI's message JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Pdf => "pdf",
            Self::Document => "document",
            Self::Contact => "contact",
            Self::Archive => "archive",
            Self::Other => "other",
        }
    }

    fn opens_externally(self) -> bool {
        !matches!(self, Self::Archive | Self::Other)
    }
}

/// Known files: extension, media type, kind. The first extension of a media type is the one a
/// name without any gets.
const TYPES: &[(&str, &str, FileClass)] = &[
    ("jpg", "image/jpeg", FileClass::Image),
    ("jpeg", "image/jpeg", FileClass::Image),
    // Some Android devices label JPEGs like this (upstream's MimeUtil keeps it too).
    ("jpg", "image/jpg", FileClass::Image),
    ("png", "image/png", FileClass::Image),
    ("gif", "image/gif", FileClass::Image),
    ("webp", "image/webp", FileClass::Image),
    ("heic", "image/heic", FileClass::Image),
    ("heif", "image/heif", FileClass::Image),
    ("bmp", "image/bmp", FileClass::Image),
    ("tif", "image/tiff", FileClass::Image),
    ("tiff", "image/tiff", FileClass::Image),
    ("mp3", "audio/mpeg", FileClass::Audio),
    ("m4a", "audio/mp4", FileClass::Audio),
    ("m4a", "audio/x-m4a", FileClass::Audio),
    ("aac", "audio/aac", FileClass::Audio),
    ("ogg", "audio/ogg", FileClass::Audio),
    ("oga", "audio/ogg", FileClass::Audio),
    ("opus", "audio/opus", FileClass::Audio),
    ("wav", "audio/wav", FileClass::Audio),
    ("flac", "audio/flac", FileClass::Audio),
    ("flac", "audio/x-flac", FileClass::Audio),
    ("amr", "audio/amr", FileClass::Audio),
    ("mid", "audio/midi", FileClass::Audio),
    ("midi", "audio/midi", FileClass::Audio),
    ("mp4", "video/mp4", FileClass::Video),
    ("m4v", "video/x-m4v", FileClass::Video),
    ("mov", "video/quicktime", FileClass::Video),
    ("webm", "video/webm", FileClass::Video),
    ("mkv", "video/x-matroska", FileClass::Video),
    ("3gp", "video/3gpp", FileClass::Video),
    ("avi", "video/x-msvideo", FileClass::Video),
    ("mpg", "video/mpeg", FileClass::Video),
    ("mpeg", "video/mpeg", FileClass::Video),
    ("pdf", "application/pdf", FileClass::Pdf),
    ("txt", "text/plain", FileClass::Document),
    ("csv", "text/csv", FileClass::Document),
    ("rtf", "application/rtf", FileClass::Document),
    ("ics", "text/calendar", FileClass::Document),
    ("epub", "application/epub+zip", FileClass::Document),
    (
        "odg",
        "application/vnd.oasis.opendocument.graphics",
        FileClass::Document,
    ),
    (
        "odt",
        "application/vnd.oasis.opendocument.text",
        FileClass::Document,
    ),
    (
        "ods",
        "application/vnd.oasis.opendocument.spreadsheet",
        FileClass::Document,
    ),
    (
        "odp",
        "application/vnd.oasis.opendocument.presentation",
        FileClass::Document,
    ),
    ("doc", "application/msword", FileClass::Document),
    (
        "docx",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        FileClass::Document,
    ),
    ("xls", "application/vnd.ms-excel", FileClass::Document),
    (
        "xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        FileClass::Document,
    ),
    ("ppt", "application/vnd.ms-powerpoint", FileClass::Document),
    (
        "pptx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        FileClass::Document,
    ),
    // Upstream's MimeUtil knows both labels.
    ("vcf", "text/x-vcard", FileClass::Contact),
    ("vcf", "text/vcard", FileClass::Contact),
    ("vcard", "text/x-vcard", FileClass::Contact),
    ("zip", "application/zip", FileClass::Archive),
    ("gz", "application/gzip", FileClass::Archive),
    ("tgz", "application/x-gtar", FileClass::Archive),
    ("7z", "application/x-7z-compressed", FileClass::Archive),
    ("rar", "application/vnd.rar", FileClass::Archive),
    ("tar", "application/x-tar", FileClass::Archive),
    ("bz2", "application/x-bzip2", FileClass::Archive),
    ("xz", "application/x-xz", FileClass::Archive),
];

fn by_media_type(media_type: &str) -> Option<(&'static str, FileClass)> {
    let media_type = media_type.split(';').next().unwrap_or_default().trim();
    TYPES
        .iter()
        .find(|(_, known, _)| known.eq_ignore_ascii_case(media_type))
        .map(|&(extension, _, class)| (extension, class))
}

/// The extension a file of this media type gets if its name has none.
pub(crate) fn extension_for(media_type: &str) -> Option<&'static str> {
    by_media_type(media_type).map(|(extension, _)| extension)
}

fn extension_of(name: &str) -> &str {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
}

/// The kind of file that `name` makes it.
pub fn class_of_name(name: &str) -> FileClass {
    let extension = extension_of(name);
    TYPES
        .iter()
        .find(|(known, _, _)| known.eq_ignore_ascii_case(extension))
        .map_or(FileClass::Other, |&(_, _, class)| class)
}

/// May a saved file of this name go to the system's app for it? Documents, media and contact
/// cards; everything else (programs, launchers, packages, scripts, web pages) stays saved only.
pub fn opens_externally(name: &str) -> bool {
    class_of_name(name).opens_externally()
}

/// `name` without the characters that reorder how text reads (U+202E shows `fdp.desktop` as
/// `potksed.pdf`). No file name needs them; the text direction of the name's letters remains.
pub fn without_bidi_controls(name: &str) -> String {
    name.chars()
        .filter(|c| {
            !matches!(c,
                '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// A stored file message's body as the UI gets it: the name as it reads on the disk, and the
/// checked `fileType` for its icon.
pub(crate) fn shown_body(mut body: Value) -> Value {
    if !body.is_object() {
        return body;
    }
    let saved_name = crate::app_media::safe_file_name(
        body["name"].as_str(),
        body["mediaType"].as_str().unwrap_or_default(),
    );
    body["fileType"] = class_of_name(&saved_name).as_str().into();
    if let Some(body) = body.as_object_mut() {
        body.remove(crate::app_media::LEGACY_NONCE);
    }
    if let Some(name) = body["name"].as_str().map(without_bidi_controls) {
        body["name"] = name.into();
    }
    body
}

/// Bitmaps and TIFFs show in the app's file row only: they aren't saved to the gallery.
fn kept_from_gallery(extension: &str) -> bool {
    ["bmp", "tif", "tiff"]
        .iter()
        .any(|kept| kept.eq_ignore_ascii_case(extension))
}

/// The name for a photo or video saved to the gallery without anyone asking, whose folder other
/// programs index: it says what the media type says, or the media type's extension is added.
/// `None` for anything but known pictures and videos.
pub(crate) fn gallery_file_name(name: Option<&str>, media_type: &str) -> Option<String> {
    let (extension, class) = by_media_type(media_type)?;
    if !matches!(class, FileClass::Image | FileClass::Video) || kept_from_gallery(extension) {
        return None;
    }
    let safe = crate::app_media::safe_file_name(name, media_type);
    Some(
        if class_of_name(&safe) == class && !kept_from_gallery(extension_of(&safe)) {
            safe
        } else {
            format!("{safe}.{extension}")
        },
    )
}

/// Do the bytes start like a picture or video of the formats [`gallery_file_name`] lets through?
/// A mislabelled file stays out of the indexed folder.
pub(crate) fn looks_like_gallery_media(data: &[u8]) -> bool {
    data.starts_with(&[0xff, 0xd8, 0xff])
        || data.starts_with(b"\x89PNG\r\n\x1a\n")
        || data.starts_with(b"GIF87a")
        || data.starts_with(b"GIF89a")
        || (data.starts_with(b"RIFF") && matches!(data.get(8..12), Some(b"WEBP" | b"AVI ")))
        // ISO media (mp4, mov, 3gp, HEIC), Matroska (mkv, webm), MPEG program and elementary
        // streams.
        || matches!(
            data.get(4..8),
            Some(b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot")
        )
        || data.starts_with(&[0x1a, 0x45, 0xdf, 0xa3])
        || data.starts_with(&[0x00, 0x00, 0x01, 0xba])
        || data.starts_with(&[0x00, 0x00, 0x01, 0xb3])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_name_decides_what_a_file_is() {
        for (name, class) in [
            ("a.pdf", FileClass::Pdf),
            ("A.PDF", FileClass::Pdf),
            ("a.odt", FileClass::Document),
            ("a.ics", FileClass::Document),
            ("a.vcf", FileClass::Contact),
            ("a.heic", FileClass::Image),
            ("a.bmp", FileClass::Image),
            ("a.tar.gz", FileClass::Archive),
            ("a.pdf.desktop", FileClass::Other),
            ("a.desktop", FileClass::Other),
            ("a.sh", FileClass::Other),
            ("a.rpm", FileClass::Other),
            ("a.html", FileClass::Other),
            ("a.svg", FileClass::Other),
            ("pdf", FileClass::Other),
            ("a.pd\u{200B}f", FileClass::Other),
        ] {
            assert_eq!(class_of_name(name), class, "{name}");
        }
    }

    #[test]
    fn only_documents_and_media_open_in_other_apps() {
        for name in [
            "a.pdf",
            "a.jpg",
            "a.mp3",
            "a.mp4",
            "a.txt",
            "a (1).docx",
            "a.vcf",
            "a.ics",
            "a.epub",
            "a.odg",
            "a.tiff",
            "a.heic",
            "a.avi",
            "a.mpeg",
            "a.midi",
            "a.amr",
        ] {
            assert!(opens_externally(name), "{name}");
        }
        for name in [
            "a.desktop",
            "a.sh",
            "a.rpm",
            "a.zip",
            "a.html",
            "a",
            "a.exe",
        ] {
            assert!(!opens_externally(name), "{name}");
        }
    }

    /// The bubble of `invoice<U+202E>fdp.desktop`, sent as a PDF: it reads as what it is, and has
    /// no PDF icon.
    #[test]
    fn the_ui_gets_a_checked_type_and_a_clean_name() {
        let shown = shown_body(json!({
            "mediaType": "application/pdf",
            "name": "invoice\u{202E}fdp.desktop",
        }));
        assert_eq!(shown["name"], "invoicefdp.desktop");
        assert_eq!(shown["fileType"], "other");

        let named = shown_body(json!({"mediaType": "application/pdf", "name": "report"}));
        assert_eq!(named["fileType"], "pdf");
        assert_eq!(named["name"], "report");

        let unnamed = shown_body(json!({"mediaType": "image/jpeg"}));
        assert_eq!(unnamed["fileType"], "image");
        assert!(unnamed["name"].is_null());

        assert_eq!(shown_body(Value::Null), Value::Null);
    }

    #[test]
    fn aliases_of_a_type_get_its_extension() {
        for (media_type, extension) in [
            ("text/x-vcard", "vcf"),
            ("text/vcard", "vcf"),
            ("audio/x-m4a", "m4a"),
            ("audio/x-flac", "flac"),
            ("image/jpg", "jpg"),
            ("Text/X-VCard; charset=utf-8", "vcf"),
        ] {
            assert_eq!(extension_for(media_type), Some(extension), "{media_type}");
        }
    }

    #[test]
    fn bidi_controls_go_and_letters_stay() {
        let controls = "a\u{061C}\u{200E}\u{200F}\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}\
                        \u{2066}\u{2067}\u{2068}\u{2069}b";
        assert_eq!(without_bidi_controls(controls), "ab");
        assert_eq!(without_bidi_controls("שלום.pdf"), "שלום.pdf");
    }

    #[test]
    fn the_gallery_gets_what_the_media_type_says() {
        assert_eq!(
            gallery_file_name(Some("x.jpg"), "image/jpeg").as_deref(),
            Some("x.jpg")
        );
        assert_eq!(
            gallery_file_name(Some("x.desktop"), "image/jpeg").as_deref(),
            Some("x.desktop.jpg")
        );
        assert_eq!(
            gallery_file_name(Some("x.mp4"), "image/jpeg").as_deref(),
            Some("x.mp4.jpg")
        );
        assert_eq!(
            gallery_file_name(Some("x.mp4"), "video/mp4").as_deref(),
            Some("x.mp4")
        );
        assert_eq!(
            gallery_file_name(None, "IMAGE/PNG").as_deref(),
            Some("threesea-file.png")
        );
        // A real JPEG with Android's other label.
        assert_eq!(
            gallery_file_name(None, "image/jpg").as_deref(),
            Some("threesea-file.jpg")
        );
        assert_eq!(
            gallery_file_name(Some("x.heic"), "image/heic").as_deref(),
            Some("x.heic")
        );
        assert_eq!(
            gallery_file_name(Some("x.tiff"), "image/jpeg").as_deref(),
            Some("x.tiff.jpg")
        );
        assert_eq!(gallery_file_name(Some("x.tiff"), "image/tiff"), None);
        assert_eq!(gallery_file_name(Some("x.bmp"), "image/bmp"), None);
        assert_eq!(gallery_file_name(Some("x.jpg"), "application/pdf"), None);
    }

    #[test]
    fn gallery_bytes_are_recognised() {
        assert!(looks_like_gallery_media(b"\xff\xd8\xff\xe0"));
        assert!(looks_like_gallery_media(b"\x89PNG\r\n\x1a\n"));
        assert!(looks_like_gallery_media(b"GIF89a"));
        assert!(looks_like_gallery_media(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_gallery_media(b"\0\0\0\x18ftypmp42"));
        assert!(looks_like_gallery_media(b"\x1a\x45\xdf\xa3"));
        assert!(looks_like_gallery_media(b"\0\0\0\x18ftypheic"));
        for atom in [b"free", b"skip", b"pnot", b"moov", b"mdat", b"wide"] {
            let mut mov = b"\0\0\0\x08".to_vec();
            mov.extend_from_slice(atom);
            assert!(looks_like_gallery_media(&mov));
        }
        assert!(looks_like_gallery_media(b"RIFF\0\0\0\0AVI LIST"));
        assert!(looks_like_gallery_media(b"\0\0\x01\xba"));
        assert!(looks_like_gallery_media(b"\0\0\x01\xb3"));
        assert!(!looks_like_gallery_media(b"BM\0\0\0\0"), "BMP");
        assert!(!looks_like_gallery_media(b"RIFF\0\0\0\0WAVE"));
        assert!(!looks_like_gallery_media(b"II*\0"), "TIFF");
        assert!(!looks_like_gallery_media(b"[Desktop Entry]"));
        assert!(!looks_like_gallery_media(b""));
    }
}
