// SPDX-License-Identifier: AGPL-3.0-only
//! What a WebP file says beyond its picture. A RIFF file is a list of chunks: a four-letter name,
//! a little-endian size, the content and a padding byte after an odd size. EXIF and XMP are chunks
//! of their own, and readers skip what they don't know, so as for MP4 (ADR 0025) this keeps what a
//! decoder needs and nothing else. An ICC colour profile goes too: it holds free text (device,
//! maker, copyright), and our other pictures lose theirs as well. Animation stays.

use zeroize::Zeroizing;

const FRAME_HEADER: usize = 16;
const ANIM_SIZE: usize = 6;
/// `VP8X`'s flags that stay: alpha and animation. Those of ICC, EXIF and XMP go, and the bits that
/// mean nothing.
const FLAGS_KEPT: u8 = 0x10 | 0x02;

/// Not a well-formed WebP file: whatever it holds can't be vouched for.
#[derive(Debug, PartialEq, Eq)]
pub struct NotCleanable;

/// A chunk's name and content.
type Chunk<'a> = (&'a [u8; 4], &'a [u8]);

/// The chunks in a piece of a file, one by one, in no more memory than that. Anything cut off or
/// not adding up is an error, and the last thing it gives.
struct Chunks<'a>(&'a [u8]);

impl<'a> Chunks<'a> {
    fn next_chunk(&mut self) -> Result<Chunk<'a>, NotCleanable> {
        let (name, rest) = self.0.split_first_chunk::<4>().ok_or(NotCleanable)?;
        let (size, rest) = rest.split_first_chunk::<4>().ok_or(NotCleanable)?;
        let size = usize::try_from(u32::from_le_bytes(*size)).map_err(|_| NotCleanable)?;
        let padded = size.checked_add(size & 1).ok_or(NotCleanable)?;
        if padded > rest.len() {
            return Err(NotCleanable);
        }
        self.0 = &rest[padded..];
        Ok((name, &rest[..size]))
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Result<Chunk<'a>, NotCleanable>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.0.is_empty() {
            return None;
        }
        let chunk = self.next_chunk();
        if chunk.is_err() {
            self.0 = &[];
        }
        Some(chunk)
    }
}

/// The top level chunks of a WebP file. What follows the end the file gives for itself is outside
/// of them.
fn top_level(data: &[u8]) -> Result<Chunks<'_>, NotCleanable> {
    if data.len() < 12 || &data[..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return Err(NotCleanable);
    }
    let size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let end = usize::try_from(size)
        .ok()
        .and_then(|size| size.checked_add(8))
        .filter(|end| (12..=data.len()).contains(end))
        .ok_or(NotCleanable)?;
    Ok(Chunks(&data[12..end]))
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

/// An animation's frame as it stays: its header, the first alpha before its first picture.
fn write_frame(out: &mut Vec<u8>, content: &[u8]) -> Result<(), NotCleanable> {
    if content.len() < FRAME_HEADER {
        return Err(NotCleanable);
    }
    let (header, frame) = content.split_at(FRAME_HEADER);
    let mut kept = Zeroizing::new(Vec::with_capacity(frame.len()));
    let mut alpha = false;
    let mut picture = false;
    for chunk in Chunks(frame) {
        let (name, content) = chunk?;
        match name {
            b"ALPH" if !alpha => {
                alpha = true;
                write_chunk(&mut kept, name, &[content])?;
            }
            b"VP8 " | b"VP8L" => {
                picture = true;
                write_chunk(&mut kept, name, &[content])?;
                break;
            }
            _ => {}
        }
    }
    if !picture {
        return Err(NotCleanable);
    }
    write_chunk(out, b"ANMF", &[header, &kept])
}

/// Takes out of a WebP file in memory everything but its picture or its animation, and tells the
/// size of a picture whose file says it in plain view (an extended one). Doing it twice changes
/// nothing more. On an error `data` is as it was; the old content is wiped.
pub fn clean(data: &mut Vec<u8>) -> Result<Option<(u32, u32)>, NotCleanable> {
    let mut out = Zeroizing::new(Vec::with_capacity(data.len()));
    out.extend_from_slice(b"RIFF\0\0\0\0WEBP");
    let mut size = None;
    let mut first = true;
    let mut alpha = false;
    let mut animation = false;
    let mut frames = false;
    let mut still = false;
    for chunk in top_level(data)? {
        let (name, content) = chunk?;
        match name {
            b"VP8X" => {
                // Its place is first, and its fields are what the rest is read by.
                if !first || content.len() != 10 {
                    return Err(NotCleanable);
                }
                let side = |at: usize| {
                    u32::from_le_bytes([content[at], content[at + 1], content[at + 2], 0]) + 1
                };
                size = Some((side(4), side(7)));
                let mut header = [0; 10];
                header[0] = content[0] & FLAGS_KEPT;
                header[4..].copy_from_slice(&content[4..]);
                write_chunk(&mut out, name, &[&header])?;
            }
            b"ANIM" if !animation => {
                if content.len() != ANIM_SIZE {
                    return Err(NotCleanable);
                }
                animation = true;
                write_chunk(&mut out, name, &[content])?;
            }
            b"ANMF" => {
                if !animation {
                    return Err(NotCleanable);
                }
                frames = true;
                write_frame(&mut out, content)?;
            }
            b"ALPH" if !alpha && !animation => {
                alpha = true;
                write_chunk(&mut out, name, &[content])?;
            }
            b"VP8 " | b"VP8L" if !frames => {
                // A still picture, once it has been said what it is, ends the file.
                if animation {
                    return Err(NotCleanable);
                }
                still = true;
                write_chunk(&mut out, name, &[content])?;
                break;
            }
            _ => {}
        }
        first = false;
    }
    if !still && !frames {
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
    let Ok(mut chunks) = top_level(data) else {
        return false;
    };
    chunks.any(|chunk| matches!(chunk, Ok((b"ANIM", _))))
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
            chunk(b"ANIM", b"loop.."),
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
            extended(0x20 | 0x10 | 0x08 | 0x04 | 0x01),
            chunk(b"ICCP", b"my phone's profile"),
            chunk(b"ALPH", b"alpha"),
            chunk(b"ALPH", b"more"),
            chunk(b"JUNK", b"anything"),
            picture.clone(),
            chunk(b"EXIF", b"GPS here"),
            chunk(b"XMP ", b"who, odd"),
            chunk(b"VP8L", b"another picture"),
            chunk(b"ANMF", &[0; FRAME_HEADER]),
        ]);
        assert_eq!(clean(&mut data), Ok(Some((320, 240))));
        assert_eq!(
            data,
            file(&[extended(0x10), chunk(b"ALPH", b"alpha"), picture])
        );
        let riff = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        assert_eq!(riff as usize, data.len() - 8, "the file's size follows");
        let before = data.clone();
        clean(&mut data).unwrap();
        assert_eq!(data, before, "twice changes nothing");
    }

    #[test]
    fn what_the_extended_header_could_hide_goes() {
        let mut data = file(&[
            chunk(b"VP8X", &[0, 1, 2, 3, 63, 1, 0, 239, 0, 0]),
            chunk(b"VP8 ", b"pict"),
        ]);
        clean(&mut data).unwrap();
        assert_eq!(data, file(&[extended(0), chunk(b"VP8 ", b"pict")]));
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
        let anim = chunk(b"ANIM", b"loop..");
        let mut data = file(&[
            extended(0x02 | 0x08),
            anim.clone(),
            chunk(b"ANIM", b"second"),
            frame(&[
                chunk(b"ALPH", b"a"),
                chunk(b"ALPH", b"b"),
                chunk(b"VP8 ", b"one"),
                chunk(b"VP8L", b"more of it"),
                chunk(b"EXIF", b"in a frame"),
            ]),
            chunk(b"ALPH", b"outside"),
            chunk(b"VP8 ", b"outside"),
            frame(&[chunk(b"VP8L", b"two!")]),
        ]);
        assert!(is_animated(&data));
        clean(&mut data).unwrap();
        assert_eq!(
            data,
            file(&[
                extended(0x02),
                anim,
                frame(&[chunk(b"ALPH", b"a"), chunk(b"VP8 ", b"one")]),
                frame(&[chunk(b"VP8L", b"two!")]),
            ])
        );
        assert!(is_animated(&data));
        assert!(!is_animated(&file(&[chunk(b"VP8 ", b"one")])));
    }

    #[test]
    fn looking_for_an_animation_stops_at_it_and_at_what_doesnt_add_up() {
        let anim = chunk(b"ANIM", b"loop..");
        let mut data = file(&[anim, chunk(b"VP8 ", b"one")]);
        assert!(is_animated(&data));
        data.truncate(12);
        assert!(!is_animated(&data));
        assert!(!is_animated(b"not a webp"));
    }

    #[test]
    fn what_follows_the_files_end_goes() {
        let clean_file = file(&[chunk(b"VP8 ", b"pict")]);
        let mut data = [clean_file.as_slice(), b"Exif, outside any chunk"].concat();
        clean(&mut data).unwrap();
        assert_eq!(data, clean_file);
    }

    /// Fuzzing seed: `cargo test -p threesea-core --lib write_webp_seeds -- --ignored`
    #[test]
    #[ignore = "writes fuzzing seeds"]
    fn write_webp_seeds() {
        let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/webp_clean");
        std::fs::create_dir_all(&seeds).unwrap();
        std::fs::write(seeds.join("still.webp"), super::fixture::still()).unwrap();
        std::fs::write(seeds.join("animated.webp"), super::fixture::animated()).unwrap();
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
                file(&[
                    chunk(b"JUNK", b"pict"),
                    extended(0),
                    chunk(b"VP8 ", b"pict"),
                ]),
            ),
            (
                "frame too short",
                file(&[chunk(b"ANIM", b"loop.."), chunk(b"ANMF", b"short")]),
            ),
            (
                "frame without a picture",
                file(&[
                    chunk(b"ANIM", b"loop.."),
                    chunk(b"ANMF", &[0; FRAME_HEADER]),
                ]),
            ),
            (
                "frame without an animation",
                file(&[chunk(
                    b"ANMF",
                    &[[7; FRAME_HEADER].as_slice(), &chunk(b"VP8 ", b"one")].concat(),
                )]),
            ),
            (
                "animation of another size",
                file(&[
                    chunk(b"ANIM", b"long and free text"),
                    chunk(b"VP8 ", b"one"),
                ]),
            ),
            (
                "animation of a still picture",
                file(&[chunk(b"ANIM", b"loop.."), chunk(b"VP8 ", b"one")]),
            ),
        ] {
            let mut data = bad.clone();
            assert_eq!(clean(&mut data), Err(NotCleanable), "{why}");
            assert_eq!(data, bad, "{why}");
        }
    }
}
