// SPDX-License-Identifier: AGPL-3.0-only
//! What a WebP file says beyond its picture. A RIFF file is a list of chunks: a four-letter name,
//! a little-endian size, the content and a padding byte after an odd size. EXIF and XMP are chunks
//! of their own, and readers skip what they don't know, so as for MP4 (ADR 0025) this keeps a list
//! of what stays, not of what goes. Animation stays.

use zeroize::Zeroizing;

/// The chunks that make up the picture, besides `VP8X` and `ANMF`, which take more care. `ICCP` is
/// how its colours are meant.
const KEPT: [&[u8; 4]; 5] = [b"ICCP", b"ANIM", b"ALPH", b"VP8 ", b"VP8L"];
/// The same within an animation's frame, after its header.
const KEPT_IN_FRAME: [&[u8; 4]; 3] = [b"ALPH", b"VP8 ", b"VP8L"];
const FRAME_HEADER: usize = 16;
/// `VP8X` flags: the file has EXIF, the file has XMP.
const FLAGS_METADATA: u8 = 0x08 | 0x04;

/// Not a well-formed WebP file: whatever it holds can't be vouched for.
#[derive(Debug, PartialEq, Eq)]
pub struct NotCleanable;

/// A chunk's name and content.
type Chunk<'a> = (&'a [u8; 4], &'a [u8]);

/// The chunks in `data`. Anything cut off or not adding up is an error.
fn chunks(mut data: &[u8]) -> Result<Vec<Chunk<'_>>, NotCleanable> {
    let mut found = Vec::new();
    while !data.is_empty() {
        let (name, rest) = data.split_first_chunk::<4>().ok_or(NotCleanable)?;
        let (size, rest) = rest.split_first_chunk::<4>().ok_or(NotCleanable)?;
        let size = usize::try_from(u32::from_le_bytes(*size)).map_err(|_| NotCleanable)?;
        let padded = size.checked_add(size & 1).ok_or(NotCleanable)?;
        if padded > rest.len() {
            return Err(NotCleanable);
        }
        found.push((name, &rest[..size]));
        data = &rest[padded..];
    }
    Ok(found)
}

/// The top level chunks of a WebP file. What follows the end the file gives for itself is outside
/// of them.
fn top_level(data: &[u8]) -> Result<Vec<Chunk<'_>>, NotCleanable> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err(NotCleanable);
    }
    let size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let end = usize::try_from(size)
        .ok()
        .and_then(|size| size.checked_add(8))
        .filter(|end| (12..=data.len()).contains(end))
        .ok_or(NotCleanable)?;
    chunks(&data[12..end])
}

fn write_chunk(out: &mut Vec<u8>, name: &[u8; 4], parts: &[&[u8]]) -> Result<(), NotCleanable> {
    let size: usize = parts.iter().map(|part| part.len()).sum();
    out.extend_from_slice(name);
    out.extend_from_slice(&u32::try_from(size).map_err(|_| NotCleanable)?.to_le_bytes());
    for part in parts {
        out.extend_from_slice(part);
    }
    if size % 2 == 1 {
        out.push(0);
    }
    Ok(())
}

/// Takes out of a WebP file in memory everything but its picture, and tells the size of a picture
/// whose file says it in plain view (an extended one). Doing it twice changes nothing more. On an
/// error `data` is as it was; the old content is wiped.
pub fn clean(data: &mut Vec<u8>) -> Result<Option<(u32, u32)>, NotCleanable> {
    let top = top_level(data)?;
    let mut out = Zeroizing::new(Vec::with_capacity(data.len()));
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let mut size = None;
    let mut pictures = 0;
    for (at, (name, content)) in top.iter().enumerate() {
        if *name == b"VP8X" {
            // Its place is first, and its fields are what the rest is read by.
            if at != 0 || content.len() != 10 {
                return Err(NotCleanable);
            }
            let side = |at: usize| {
                u32::from_le_bytes([content[at], content[at + 1], content[at + 2], 0]) + 1
            };
            size = Some((side(4), side(7)));
            let mut header = [0; 10];
            header.copy_from_slice(content);
            header[0] &= !FLAGS_METADATA;
            write_chunk(&mut out, name, &[&header])?;
        } else if *name == b"ANMF" {
            if content.len() < FRAME_HEADER {
                return Err(NotCleanable);
            }
            let (header, frame) = content.split_at(FRAME_HEADER);
            let mut kept = Zeroizing::new(Vec::with_capacity(frame.len()));
            let mut has_picture = false;
            for (name, content) in chunks(frame)? {
                if KEPT_IN_FRAME.contains(&name) {
                    has_picture |= name != b"ALPH";
                    write_chunk(&mut kept, name, &[content])?;
                }
            }
            if !has_picture {
                return Err(NotCleanable);
            }
            write_chunk(&mut out, name, &[header, &kept])?;
            pictures += 1;
        } else if KEPT.contains(name) {
            if matches!(*name, b"VP8 " | b"VP8L") {
                pictures += 1;
            }
            write_chunk(&mut out, name, &[content])?;
        }
    }
    if pictures == 0 {
        return Err(NotCleanable);
    }
    let riff = u32::try_from(out.len() - 8).map_err(|_| NotCleanable)?;
    out[4..8].copy_from_slice(&riff.to_le_bytes());
    drop(Zeroizing::new(std::mem::take(data)));
    data.extend_from_slice(&out);
    Ok(size)
}

/// Whether a WebP file is an animation.
#[must_use]
pub fn is_animated(data: &[u8]) -> bool {
    top_level(data).is_ok_and(|chunks| chunks.iter().any(|(name, _)| *name == b"ANIM"))
}

/// WebP files for other modules' tests.
#[cfg(test)]
pub(crate) mod fixture {
    use super::{FRAME_HEADER, write_chunk};

    pub(crate) fn chunk(name: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        write_chunk(&mut out, name, &[content]).unwrap();
        out
    }

    pub(crate) fn file(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body = chunks.concat();
        let size = u32::try_from(body.len() + 4).unwrap();
        [b"RIFF".as_slice(), &size.to_le_bytes(), b"WEBP", &body].concat()
    }

    /// An animation for 320 x 240, with EXIF.
    pub(crate) fn animated() -> Vec<u8> {
        file(&[
            chunk(b"VP8X", &[0x02 | 0x08, 0, 0, 0, 63, 1, 0, 239, 0, 0]),
            chunk(b"ANIM", b"loop"),
            chunk(
                b"ANMF",
                &[&[7; FRAME_HEADER][..], &chunk(b"VP8 ", b"one")].concat(),
            ),
            chunk(b"EXIF", b"GPS here"),
        ])
    }

    /// A still picture, with EXIF.
    pub(crate) fn still() -> Vec<u8> {
        file(&[
            chunk(b"VP8X", &[0x08, 0, 0, 0, 63, 1, 0, 239, 0, 0]),
            chunk(b"VP8 ", b"PICTURE"),
            chunk(b"EXIF", b"GPS here"),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        fixture::{chunk, file},
        *,
    };

    /// `VP8X` with these flags, for a 320 x 240 picture.
    fn extended(flags: u8) -> Vec<u8> {
        chunk(b"VP8X", &[flags, 0, 0, 0, 63, 1, 0, 239, 0, 0])
    }

    #[test]
    fn metadata_goes_and_the_picture_stays() {
        let picture = chunk(b"VP8 ", b"PICTURE"); // odd: padded
        let mut data = file(&[
            extended(0x20 | 0x10 | 0x08 | 0x04),
            chunk(b"ICCP", b"colours"),
            chunk(b"ALPH", b"alpha"),
            picture.clone(),
            chunk(b"EXIF", b"GPS here"),
            chunk(b"XMP ", b"who, odd"),
            chunk(b"JUNK", b"anything"),
        ]);
        assert_eq!(clean(&mut data), Ok(Some((320, 240))));
        assert_eq!(
            data,
            file(&[
                extended(0x20 | 0x10),
                chunk(b"ICCP", b"colours"),
                chunk(b"ALPH", b"alpha"),
                picture,
            ])
        );
        let riff = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(riff as usize, data.len() - 8, "the file's size follows");
        let before = data.clone();
        clean(&mut data).unwrap();
        assert_eq!(data, before, "twice changes nothing");
    }

    #[test]
    fn a_simple_file_has_no_flags_to_change() {
        let mut data = file(&[chunk(b"VP8L", b"lossless")]);
        let before = data.clone();
        assert_eq!(clean(&mut data), Ok(None));
        assert_eq!(data, before);
    }

    #[test]
    fn an_animation_stays_one_without_what_its_frames_carry() {
        let header = [7; FRAME_HEADER];
        let frame = |parts: &[Vec<u8>]| chunk(b"ANMF", &[&header[..], &parts.concat()].concat());
        let mut data = file(&[
            extended(0x02 | 0x08),
            chunk(b"ANIM", b"loop"),
            frame(&[
                chunk(b"ALPH", b"a"),
                chunk(b"VP8 ", b"one"),
                chunk(b"EXIF", b"in a frame"),
            ]),
            frame(&[chunk(b"VP8L", b"two!")]),
        ]);
        assert!(is_animated(&data));
        clean(&mut data).unwrap();
        assert_eq!(
            data,
            file(&[
                extended(0x02),
                chunk(b"ANIM", b"loop"),
                frame(&[chunk(b"ALPH", b"a"), chunk(b"VP8 ", b"one")]),
                frame(&[chunk(b"VP8L", b"two!")]),
            ])
        );
        assert!(is_animated(&data));
        assert!(!is_animated(&file(&[chunk(b"VP8 ", b"one")])));
    }

    #[test]
    fn what_follows_the_files_end_goes() {
        let clean_file = file(&[chunk(b"VP8 ", b"pict")]);
        let mut data = [clean_file.as_slice(), b"Exif, outside any chunk"].concat();
        clean(&mut data).unwrap();
        assert_eq!(data, clean_file);
    }

    #[test]
    fn what_doesnt_add_up_is_refused_and_left_as_it_was() {
        let good = file(&[extended(0x08), chunk(b"VP8 ", b"PICTURE")]);
        for (why, bad) in [
            ("cut off in a chunk", good[..good.len() - 3].to_vec()),
            ("cut off at the padding", good[..good.len() - 1].to_vec()),
            ("cut off in a chunk's head", good[..12 + 18 + 3].to_vec()),
            ("no head", b"RIFF".to_vec()),
            ("another RIFF", [&good[..8], b"WAVE", &good[12..]].concat()),
            ("no picture", file(&[extended(0), chunk(b"EXIF", b"only")])),
            (
                "size past the end",
                [b"RIFF".as_slice(), &u32::MAX.to_le_bytes(), &good[8..]].concat(),
            ),
            (
                "VP8X not first",
                file(&[chunk(b"VP8 ", b"pict"), extended(0)]),
            ),
            ("frame too short", file(&[chunk(b"ANMF", b"short")])),
            (
                "frame without a picture",
                file(&[chunk(b"ANMF", &[0; FRAME_HEADER])]),
            ),
        ] {
            let mut data = bad.clone();
            assert_eq!(clean(&mut data), Err(NotCleanable), "{why}");
            assert_eq!(data, bad, "{why}");
        }
    }
}
