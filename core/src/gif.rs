// SPDX-License-Identifier: AGPL-3.0-only
//! What a GIF file says beyond its picture. A GIF is a header and colours, then a list of blocks;
//! comments and application data (XMP among them) are blocks of their own, and readers skip what
//! they don't know, so as for MP4 and WebP (ADR 0025) this keeps what a decoder needs and nothing
//! else: the frames, what says how long they show, and the block that says how often the animation
//! runs. Plain text blocks go too: they hold free text, and hardly anything draws them.

use zeroize::Zeroizing;

const HEADER: usize = 13;
const EXTENSION: u8 = 0x21;
const IMAGE: u8 = 0x2c;
const TRAILER: u8 = 0x3b;
const GRAPHIC_CONTROL: u8 = 0xf9;
const APPLICATION: u8 = 0xff;
const PLAIN_TEXT: u8 = 0x01;
const IMAGE_HEADER: usize = 10;

/// Not a well-formed GIF file: whatever it holds can't be vouched for.
#[derive(Debug, PartialEq, Eq)]
pub struct NotCleanable;

/// Whether these bytes say they are a GIF.
#[must_use]
pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// A colour table's size from the flags byte in front of it.
fn colours(flags: u8) -> usize {
    if flags & 0x80 == 0 {
        0
    } else {
        3 << ((flags & 0x07) + 1)
    }
}

/// Where the sub-blocks starting at `at` end, past their empty last one.
fn past_sub_blocks(bytes: &[u8], mut at: usize) -> Result<usize, NotCleanable> {
    loop {
        let size = usize::from(*bytes.get(at).ok_or(NotCleanable)?);
        at += 1;
        if size == 0 {
            return Ok(at);
        }
        at += size;
        if at > bytes.len() {
            return Err(NotCleanable);
        }
    }
}

/// An application block saying how often the animation runs, and nothing else: its name, then one
/// sub-block of the loop count.
fn is_loop_count(block: &[u8]) -> bool {
    block.len() == 2 + 12 + 5
        && (&block[3..14] == b"NETSCAPE2.0" || &block[3..14] == b"ANIMEXTS1.0")
        && block[2] == 11
        && block[14] == 3
        && block[15] == 1
        && block[18] == 0
}

/// The parts of the file that stay, in order. Touches nothing.
fn kept_parts(bytes: &[u8]) -> Result<Vec<(usize, usize)>, NotCleanable> {
    if !is_gif(bytes) || bytes.len() < HEADER {
        return Err(NotCleanable);
    }
    let mut at = HEADER + colours(bytes[10]);
    if at > bytes.len() {
        return Err(NotCleanable);
    }
    let mut kept = vec![(0, at)];
    // A graphic control block belongs to what follows it: it goes if that does.
    let mut control: Option<(usize, usize)> = None;
    let mut frames = 0_usize;
    loop {
        let start = at;
        let Some(&kind) = bytes.get(at) else {
            // Without a last byte, which real files do: the frames are all there.
            break;
        };
        match kind {
            TRAILER => {
                kept.push((at, at + 1));
                break;
            }
            IMAGE => {
                let flags = *bytes.get(at + IMAGE_HEADER - 1).ok_or(NotCleanable)?;
                // The header, the colours and the LZW minimum code size.
                at += IMAGE_HEADER + colours(flags) + 1;
                if at > bytes.len() {
                    return Err(NotCleanable);
                }
                at = past_sub_blocks(bytes, at)?;
                kept.extend(control.take());
                kept.push((start, at));
                frames += 1;
            }
            EXTENSION => {
                let label = *bytes.get(at + 1).ok_or(NotCleanable)?;
                at = past_sub_blocks(bytes, at + 2)?;
                match label {
                    GRAPHIC_CONTROL => control = Some((start, at)),
                    APPLICATION if is_loop_count(&bytes[start..at]) => kept.push((start, at)),
                    PLAIN_TEXT => control = None,
                    _ => {}
                }
            }
            _ => return Err(NotCleanable),
        }
    }
    if frames == 0 {
        return Err(NotCleanable);
    }
    Ok(kept)
}

/// Takes out all but the picture, in place, and says its size. A file that doesn't add up is left
/// as it was.
pub fn clean(data: &mut Zeroizing<Vec<u8>>) -> Result<(u32, u32), NotCleanable> {
    let kept = kept_parts(data)?;
    let mut to = 0;
    for (start, end) in kept {
        data.copy_within(start..end, to);
        to += end - start;
    }
    // Nothing of what went stays in the buffer behind the end.
    data[to..].fill(0);
    data.truncate(to);
    Ok((
        u32::from(u16::from_le_bytes([data[6], data[7]])),
        u32::from(u16::from_le_bytes([data[8], data[9]])),
    ))
}

/// Files to test with: here too for the fuzz seeds and other modules' tests.
#[cfg(test)]
pub(crate) mod fixture {
    /// Header and a two-colour table for one pixel.
    fn head() -> Vec<u8> {
        let mut file = b"GIF89a\x01\x00\x01\x00\x80\x00\x00".to_vec();
        file.extend([0, 0, 0, 255, 255, 255]);
        file
    }

    pub(crate) const LOOP_COUNT: &[u8] = b"\x21\xff\x0bNETSCAPE2.0\x03\x01\x00\x00\x00";
    pub(crate) const CONTROL: &[u8] = b"\x21\xf9\x04\x00\x0a\x00\x00\x00";
    pub(crate) const FRAME: &[u8] = b"\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00";

    /// A comment, XMP and a plain text block with its own control block around an animation.
    pub(crate) fn with_metadata() -> Vec<u8> {
        let mut file = head();
        file.extend(b"\x21\xfe\x08GPS here\x00");
        file.extend(LOOP_COUNT);
        file.extend(b"\x21\xff\x0bXMP DataXMP\x08GPS here\x00");
        file.extend(CONTROL);
        file.extend(FRAME);
        file.extend(CONTROL);
        file.extend(
            b"\x21\x01\x0c\x00\x00\x00\x00\x02\x00\x01\x00\x01\x01\x00\x01\x08GPS here\x00",
        );
        file.extend(CONTROL);
        file.extend(FRAME);
        file.push(0x3b);
        file.extend(b"GPS here");
        file
    }

    /// The same as it is to leave.
    pub(crate) fn clean() -> Vec<u8> {
        let mut file = head();
        file.extend(LOOP_COUNT);
        file.extend(CONTROL);
        file.extend(FRAME);
        file.extend(CONTROL);
        file.extend(FRAME);
        file.push(0x3b);
        file
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(file: &[u8]) -> Result<Vec<u8>, NotCleanable> {
        let mut data = Zeroizing::new(file.to_vec());
        clean(&mut data).map(|_| data.to_vec())
    }

    #[test]
    fn only_the_animation_stays() {
        let mut data = Zeroizing::new(fixture::with_metadata());
        assert_eq!(clean(&mut data), Ok((1, 1)));
        assert_eq!(data.as_slice(), fixture::clean());
    }

    #[test]
    fn cleaning_twice_changes_nothing() {
        assert_eq!(cleaned(&fixture::clean()).unwrap(), fixture::clean());
    }

    #[test]
    fn a_loop_block_with_more_in_it_goes() {
        let mut file = fixture::clean();
        let at = file
            .windows(11)
            .position(|part| part == b"NETSCAPE2.0")
            .unwrap();
        // A second sub-block after the loop count.
        file.splice(at + 15..at + 15, *b"\x08GPS here");
        let out = cleaned(&file).unwrap();
        assert!(!out.windows(8).any(|part| part == b"GPS here"));
        assert!(!out.windows(11).any(|part| part == b"NETSCAPE2.0"));
    }

    #[test]
    fn real_files_without_a_last_byte_pass() {
        let mut file = fixture::clean();
        file.pop();
        assert_eq!(cleaned(&file).unwrap(), file);
    }

    #[test]
    fn what_doesnt_add_up_is_refused_and_left_as_it_was() {
        let good = fixture::clean();
        for (why, bad) in [
            ("no GIF", b"not one".to_vec()),
            ("only a head", good[..6].to_vec()),
            ("no colours", good[..14].to_vec()),
            ("no frames", [&good[..19], b"\x3b"].concat()),
            ("cut off in a frame", good[..good.len() - 5].to_vec()),
            ("cut off in a block", good[..25].to_vec()),
            ("a block of no kind", [&good[..19], b"\x99"].concat()),
        ] {
            let mut data = Zeroizing::new(bad.clone());
            assert_eq!(clean(&mut data), Err(NotCleanable), "{why}");
            assert_eq!(data.as_slice(), bad, "{why}");
        }
    }

    #[test]
    #[ignore = "writes the fuzz seeds"]
    fn write_gif_seeds() {
        let seeds = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/gif_clean");
        std::fs::create_dir_all(&seeds).unwrap();
        std::fs::write(seeds.join("metadata.gif"), fixture::with_metadata()).unwrap();
        std::fs::write(seeds.join("clean.gif"), fixture::clean()).unwrap();
    }
}
