// SPDX-License-Identifier: AGPL-3.0-only
//! MP4 and QuickTime files, for videos we send as media: what the file says about where, when and
//! on what it was recorded goes before it leaves the device (photos lose their EXIF the same way),
//! and the size that is announced to the recipient comes from here.
//!
//! The file is only ever rewritten in place, keeping its length: whatever goes becomes a `free`
//! box of the same size, full of zeros. Nothing moves, so the sample offsets stay right and the
//! audio and video bytes stay exactly as they were. Only boxes a player needs are kept (a list of
//! what stays, not of what goes: vendors invent new places for a GPS position), and only sound and
//! picture tracks, as Android's re-encoding keeps them: a dashcam's position track or a camera's
//! time code goes with its samples. Not read: the codecs' own streams, which can carry user data
//! of their own, and what lies in an `mdat` outside any track's samples.
use std::{
    fs::File,
    io::{Read as _, Seek as _, SeekFrom},
    ops::Range,
    path::Path,
};

/// Not a file this can vouch for: another container, or a damaged one. It stays untouched.
#[derive(Debug, PartialEq, Eq)]
pub struct NotCleanable;

/// The most of a file's index read just to learn the video's size. A long 4K recording stays well
/// below it.
const MOOV_READ_MAX: u64 = 64 * 1024 * 1024;

/// No real video comes near this many boxes in one place, or things to blank. A file of nothing
/// but 8-byte boxes would otherwise take many times its own size in memory.
const MAX_BOXES: usize = 100_000;

const BLANK: &[u8; 4] = b"free";

#[derive(Clone, Copy, Debug)]
struct Mp4Box {
    kind: [u8; 4],
    start: usize,
    body: usize,
    end: usize,
}

/// The box at `at`, with the length its header declares (it may reach beyond the data).
fn header(data: &[u8], at: usize) -> Option<([u8; 4], usize, u64)> {
    let head = data.get(at..at.checked_add(8)?)?;
    let size = u32::from_be_bytes(head[..4].try_into().ok()?);
    let kind: [u8; 4] = head[4..8].try_into().ok()?;
    match size {
        // To the end of what contains it.
        0 => Some((kind, at + 8, 0)),
        1 => {
            let large = data.get(at + 8..at.checked_add(16)?)?;
            let size = u64::from_be_bytes(large.try_into().ok()?);
            (size >= 16).then_some((kind, at + 16, size))
        }
        2..8 => None,
        size => Some((kind, at + 8, u64::from(size))),
    }
}

/// The box at `at` within a region ending at `limit`, if it fits.
fn box_at(data: &[u8], at: usize, limit: usize) -> Option<Mp4Box> {
    let (kind, body, size) = header(data, at)?;
    let end = if size == 0 {
        limit
    } else {
        at.checked_add(usize::try_from(size).ok()?)?
    };
    (body <= end && end <= limit).then_some(Mp4Box {
        kind,
        start: at,
        body,
        end,
    })
}

fn children(data: &[u8], parent: Mp4Box) -> Result<Vec<Mp4Box>, NotCleanable> {
    let mut found = Vec::new();
    let mut at = parent.body;
    // Less than a header left: QuickTime's four zero bytes that end a list.
    while parent.end - at >= 8 {
        let child = box_at(data, at, parent.end).ok_or(NotCleanable)?;
        at = child.end;
        found.push(child);
        if found.len() > MAX_BOXES {
            return Err(NotCleanable);
        }
    }
    Ok(found)
}

fn find(boxes: &[Mp4Box], kind: &[u8; 4]) -> Option<Mp4Box> {
    boxes.iter().find(|found| &found.kind == kind).copied()
}

fn read_u32(data: &[u8], at: usize) -> Option<u32> {
    let bytes = data.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes(bytes.try_into().ok()?))
}

/// What is to change, applied only once all of the file was understood.
#[derive(Default)]
struct Edits {
    blank: Vec<Mp4Box>,
    zero: Vec<Range<usize>>,
    cut: Option<usize>,
}

impl Edits {
    fn apply(self, data: &mut Vec<u8>) {
        for found in self.blank {
            data[found.start + 4..found.start + 8].copy_from_slice(BLANK);
            data[found.body..found.end].fill(0);
        }
        for range in self.zero {
            data[range].fill(0);
        }
        if let Some(cut) = self.cut {
            data.truncate(cut);
        }
    }

    fn check_room(&self) -> Result<(), NotCleanable> {
        if self.blank.len() + self.zero.len() > MAX_BOXES {
            return Err(NotCleanable);
        }
        Ok(())
    }

    /// Whatever is not on the list goes; free space may hold what was there before.
    fn keep_only(&mut self, found: Mp4Box, kept: &[&[u8; 4]]) -> bool {
        if kept.contains(&&found.kind) {
            return true;
        }
        if matches!(&found.kind, b"free" | b"skip" | b"wide") {
            self.zero.push(found.body..found.end);
        } else {
            self.blank.push(found);
        }
        false
    }

    /// `mvhd`, `tkhd`, `mdhd`: creation and modification time follow the version and flags.
    fn clear_times(&mut self, data: &[u8], found: Mp4Box) -> Result<(), NotCleanable> {
        let length = match data.get(found.body) {
            Some(0) => 8,
            Some(1) => 16,
            _ => return Err(NotCleanable),
        };
        let times = found.body + 4..found.body + 4 + length;
        if times.end > found.end {
            return Err(NotCleanable);
        }
        self.zero.push(times);
        Ok(())
    }

    /// `hdlr`: the kind of track stays; the rest is the writer's name for it ("Core Media Video").
    fn clear_handler_name(&mut self, found: Mp4Box) {
        let name = found.body + 12;
        if name < found.end {
            self.zero.push(name..found.end);
        }
    }
}

fn handler_of(data: &[u8], mdia: &[Mp4Box]) -> Option<[u8; 4]> {
    let handler = find(mdia, b"hdlr")?;
    if handler.end < handler.body + 12 {
        return None;
    }
    data.get(handler.body + 8..handler.body + 12)?
        .try_into()
        .ok()
}

/// A track's sample sizes (`stsz`, or the compact `stz2`).
enum SampleSizes {
    Uniform(u64),
    Table { first: usize, bits: usize },
}

impl SampleSizes {
    fn read(data: &[u8], stbl: &[Mp4Box]) -> Result<(Self, u64), NotCleanable> {
        let (sizes, bits) = match (find(stbl, b"stsz"), find(stbl, b"stz2")) {
            (Some(sizes), None) => (sizes, 32),
            (None, Some(sizes)) => {
                let bits = usize::from(*data.get(sizes.body + 7).ok_or(NotCleanable)?);
                if !matches!(bits, 4 | 8 | 16) {
                    return Err(NotCleanable);
                }
                (sizes, bits)
            }
            _ => return Err(NotCleanable),
        };
        let count = read_u32(data, sizes.body + 8).ok_or(NotCleanable)?;
        if bits == 32 {
            let uniform = read_u32(data, sizes.body + 4).ok_or(NotCleanable)?;
            if uniform > 0 {
                return Ok((Self::Uniform(u64::from(uniform)), u64::from(count)));
            }
        }
        let first = sizes.body + 12;
        let bytes = (u64::from(count) * bits as u64).div_ceil(8);
        if first as u64 + bytes > sizes.end as u64 {
            return Err(NotCleanable);
        }
        Ok((Self::Table { first, bits }, u64::from(count)))
    }

    fn total(&self, data: &[u8], samples: Range<u64>) -> Result<u64, NotCleanable> {
        match *self {
            Self::Uniform(size) => (samples.end - samples.start)
                .checked_mul(size)
                .ok_or(NotCleanable),
            Self::Table { first, bits } => {
                let mut total = 0;
                for index in samples {
                    let at = usize::try_from(index * bits as u64 / 8).map_err(|_| NotCleanable)?;
                    let at = first + at;
                    let byte = |at: usize| data.get(at).copied().ok_or(NotCleanable);
                    total += match bits {
                        32 => u64::from(read_u32(data, at).ok_or(NotCleanable)?),
                        16 => u64::from(byte(at)?) << 8 | u64::from(byte(at + 1)?),
                        8 => u64::from(byte(at)?),
                        _ if index % 2 == 0 => u64::from(byte(at)? >> 4),
                        _ => u64::from(byte(at)? & 0x0f),
                    };
                }
                Ok(total)
            }
        }
    }
}

/// Goes through where a track's samples lie, chunk by chunk, each of them inside an `mdat` that
/// stays, or the table lies and nothing here can be trusted. `mdats` in file order. Tells whether
/// there are any samples.
fn track_chunks(
    data: &[u8],
    mdia: &[Mp4Box],
    mdats: &[Range<usize>],
    mut chunk: impl FnMut(Range<usize>) -> Result<(), NotCleanable>,
) -> Result<bool, NotCleanable> {
    let inner = |parent: Option<Mp4Box>| -> Result<Vec<Mp4Box>, NotCleanable> {
        parent.map_or(Ok(Vec::new()), |parent| children(data, parent))
    };
    let minf = inner(find(mdia, b"minf"))?;
    let stbl = inner(find(&minf, b"stbl"))?;
    let (offsets, wide) = match (find(&stbl, b"stco"), find(&stbl, b"co64")) {
        (Some(table), None) => (table, false),
        (None, Some(table)) => (table, true),
        (None, None) => return Ok(false),
        (Some(_), Some(_)) => return Err(NotCleanable),
    };
    let table = |found: Mp4Box, width: usize| -> Option<(usize, usize)> {
        let count = usize::try_from(read_u32(data, found.body + 4)?).ok()?;
        let first = found.body + 8;
        (count.checked_mul(width)?.checked_add(first)? <= found.end).then_some((count, first))
    };
    let (chunks, chunk_table) = table(offsets, if wide { 8 } else { 4 }).ok_or(NotCleanable)?;
    // A fragmented file has its samples elsewhere.
    if chunks == 0 {
        return Ok(false);
    }
    let per_chunk = find(&stbl, b"stsc").ok_or(NotCleanable)?;
    let (runs, run_table) = table(per_chunk, 12).ok_or(NotCleanable)?;
    let (sizes, samples) = SampleSizes::read(data, &stbl)?;

    let mut sample = 0_u64;
    let mut total = 0_u64;
    let mut run = 0;
    let mut any = false;
    for index in 0..chunks {
        while run + 1 < runs {
            let next_first = read_u32(data, run_table + (run + 1) * 12).ok_or(NotCleanable)?;
            if u64::from(next_first) > index as u64 + 1 {
                break;
            }
            run += 1;
        }
        let wanted = if runs == 0 {
            0
        } else {
            u64::from(read_u32(data, run_table + run * 12 + 4).ok_or(NotCleanable)?)
        };
        let count = wanted.min(samples - sample);
        let bytes = sizes.total(data, sample..sample + count)?;
        sample += count;
        total = total.checked_add(bytes).ok_or(NotCleanable)?;
        if total > data.len() as u64 {
            return Err(NotCleanable);
        }
        if bytes == 0 {
            continue;
        }
        any = true;
        let offset = if wide {
            let at = chunk_table + index * 8;
            let bytes: [u8; 8] = data
                .get(at..at + 8)
                .and_then(|bytes| bytes.try_into().ok())
                .ok_or(NotCleanable)?;
            u64::from_be_bytes(bytes)
        } else {
            u64::from(read_u32(data, chunk_table + index * 4).ok_or(NotCleanable)?)
        };
        let start = usize::try_from(offset).map_err(|_| NotCleanable)?;
        let end = start
            .checked_add(usize::try_from(bytes).map_err(|_| NotCleanable)?)
            .ok_or(NotCleanable)?;
        let holder = mdats.partition_point(|mdat| mdat.start <= start);
        if holder == 0 || end > mdats[holder - 1].end {
            return Err(NotCleanable);
        }
        chunk(start..end)?;
    }
    Ok(any)
}

/// The entries of `stsd` and `dref`, behind their version, flags and count.
fn entries(data: &[u8], list: Mp4Box) -> Result<Vec<Mp4Box>, NotCleanable> {
    if list.end < list.body + 8 {
        return Err(NotCleanable);
    }
    children(
        data,
        Mp4Box {
            body: list.body + 8,
            ..list
        },
    )
}

/// `dinf`: samples in this file (flag 1), and no name or address of anywhere else, which QuickTime
/// can keep there. A track whose samples are elsewhere isn't sent anyway.
fn clean_references(data: &[u8], dinf: Mp4Box, edits: &mut Edits) -> Result<(), NotCleanable> {
    for child in children(data, dinf)? {
        if edits.keep_only(child, &[b"dref"]) {
            for entry in entries(data, child)? {
                let flags = read_u32(data, entry.body).ok_or(NotCleanable)?;
                if flags & 1 == 0 {
                    return Err(NotCleanable);
                }
                edits.zero.push(entry.body + 4..entry.end);
            }
        }
    }
    Ok(())
}

/// `stbl`: the tables of samples stay; in the sample descriptions, not who made the encoder (an
/// audio entry has other fields where a picture's has that name).
fn clean_sample_table(
    data: &[u8],
    stbl: Mp4Box,
    picture: bool,
    edits: &mut Edits,
) -> Result<(), NotCleanable> {
    const VENDOR: Range<usize> = 12..16;
    const COMPRESSOR_NAME: Range<usize> = 42..74;
    let kept = [
        b"stsd", b"stts", b"ctts", b"cslg", b"stsc", b"stsz", b"stz2", b"stco", b"co64", b"stss",
        b"stsh", b"stdp", b"sdtp", b"sbgp", b"sgpd", b"subs", b"saiz", b"saio", b"padb",
    ];
    for child in children(data, stbl)? {
        if edits.keep_only(child, &kept) && &child.kind == b"stsd" {
            for entry in entries(data, child)? {
                let mut clear = |field: &Range<usize>| {
                    if entry.body + field.end <= entry.end {
                        edits
                            .zero
                            .push(entry.body + field.start..entry.body + field.end);
                    }
                };
                clear(&VENDOR);
                if picture {
                    clear(&COMPRESSOR_NAME);
                }
            }
        }
    }
    Ok(())
}

/// Returns whether the track shows a picture, with samples of its own in a file that isn't
/// fragmented.
fn clean_track(
    data: &[u8],
    trak: Mp4Box,
    mdats: &[Range<usize>],
    fragmented: bool,
    edits: &mut Edits,
) -> Result<bool, NotCleanable> {
    let inner = children(data, trak)?;
    let mdia = children(data, find(&inner, b"mdia").ok_or(NotCleanable)?)?;
    let handler = handler_of(data, &mdia);
    if !matches!(handler.as_ref(), Some(b"vide" | b"soun")) {
        // Its samples in fragments would stay behind.
        if fragmented {
            return Err(NotCleanable);
        }
        track_chunks(data, &mdia, mdats, |chunk| {
            edits.zero.push(chunk);
            edits.check_room()
        })?;
        edits.blank.push(trak);
        return Ok(false);
    }
    let samples = track_chunks(data, &mdia, mdats, |_| Ok(()))?;
    for child in inner {
        if edits.keep_only(child, &[b"tkhd", b"mdia", b"edts", b"tapt"]) && &child.kind == b"tkhd" {
            edits.clear_times(data, child)?;
        }
    }
    for child in mdia {
        if edits.keep_only(child, &[b"mdhd", b"hdlr", b"minf", b"elng"]) {
            match &child.kind {
                b"mdhd" => edits.clear_times(data, child)?,
                b"hdlr" => edits.clear_handler_name(child),
                b"minf" => {
                    for part in children(data, child)? {
                        let kept = [b"vmhd", b"smhd", b"gmhd", b"dinf", b"stbl", b"hdlr"];
                        if edits.keep_only(part, &kept) {
                            match &part.kind {
                                b"hdlr" => edits.clear_handler_name(part),
                                b"dinf" => clean_references(data, part, edits)?,
                                b"stbl" => clean_sample_table(
                                    data,
                                    part,
                                    handler.as_ref() == Some(b"vide"),
                                    edits,
                                )?,
                                _ => {}
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    edits.check_room()?;
    Ok(handler.as_ref() == Some(b"vide") && (samples || fragmented))
}

fn clean_fragment(data: &[u8], moof: Mp4Box, edits: &mut Edits) -> Result<(), NotCleanable> {
    for child in children(data, moof)? {
        if edits.keep_only(child, &[b"mfhd", b"traf"]) && &child.kind == b"traf" {
            for part in children(data, child)? {
                let kept = [
                    b"tfhd", b"tfdt", b"trun", b"sbgp", b"sgpd", b"subs", b"saiz", b"saio", b"senc",
                ];
                edits.keep_only(part, &kept);
            }
        }
    }
    Ok(())
}

/// The boxes of the whole file. What follows the last whole box (a vendor's trailer, a broken
/// tail) is no part of the video and is to be cut, and any samples said to lie in it won't be found
/// (`track_chunks`). An `mdat` cut short stays as far as it goes.
fn top_level(data: &[u8]) -> Result<(Vec<Mp4Box>, Option<usize>), NotCleanable> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < data.len() {
        if found.len() >= MAX_BOXES {
            return Err(NotCleanable);
        }
        match box_at(data, at, data.len()) {
            Some(top) => {
                at = top.end;
                found.push(top);
            }
            None => {
                let short = header(data, at).filter(|(kind, ..)| kind == b"mdat");
                if let Some((kind, body, _)) = short {
                    found.push(Mp4Box {
                        kind,
                        start: at,
                        body,
                        end: data.len(),
                    });
                    return Ok((found, None));
                }
                return Ok((found, Some(at)));
            }
        }
    }
    Ok((found, None))
}

/// Takes out of an MP4 or QuickTime file in memory what it says beyond its sound and picture, and
/// tells the size of its picture as shown, if known. Doing it twice changes nothing more. On an
/// error `data` is as it was.
pub fn clean(data: &mut Vec<u8>) -> Result<Option<(u32, u32)>, NotCleanable> {
    let (tops, cut) = top_level(data)?;
    let mut edits = Edits {
        cut,
        ..Edits::default()
    };
    let mut moov = tops.iter().filter(|top| &top.kind == b"moov");
    let (Some(&moov), None) = (moov.next(), moov.next()) else {
        return Err(NotCleanable);
    };
    let mdats: Vec<Range<usize>> = tops
        .iter()
        .filter(|top| &top.kind == b"mdat")
        .map(|top| top.body..top.end)
        .collect();
    let fragmented = tops.iter().any(|top| &top.kind == b"moof");
    // Nothing says what its fragments' samples were, and later fragments may be what is cut.
    if fragmented && cut.is_some() {
        return Err(NotCleanable);
    }

    let mut has_picture = false;
    let mut has_header = false;
    for child in children(data, moov)? {
        if edits.keep_only(child, &[b"mvhd", b"trak", b"mvex", b"iods"]) {
            match &child.kind {
                b"mvhd" => {
                    has_header = true;
                    edits.clear_times(data, child)?;
                }
                b"trak" => {
                    has_picture |= clean_track(data, child, &mdats, fragmented, &mut edits)?;
                }
                _ => {}
            }
        }
    }
    if !has_header || !has_picture {
        return Err(NotCleanable);
    }
    for &top in &tops {
        let kept = [
            b"ftyp", b"styp", b"moov", b"mdat", b"moof", b"sidx", b"ssix", b"mfra", b"pdin",
        ];
        if edits.keep_only(top, &kept) && &top.kind == b"moof" {
            clean_fragment(data, top, &mut edits)?;
        }
        edits.check_room()?;
    }
    let size = shown_size(data, moov);
    edits.apply(data);
    Ok(size)
}

/// The first picture track's size as a player shows it: `tkhd` gives it before the rotation in its
/// matrix, which is how phones store a video filmed upright. Some writers leave it out there.
fn shown_size(data: &[u8], moov: Mp4Box) -> Option<(u32, u32)> {
    for trak in children(data, moov).ok()? {
        if &trak.kind != b"trak" {
            continue;
        }
        let inner = children(data, trak).ok()?;
        let mdia = children(data, find(&inner, b"mdia")?).ok()?;
        if handler_of(data, &mdia).as_ref() != Some(b"vide") {
            continue;
        }
        let stored = find(&inner, b"tkhd").and_then(|tkhd| header_size(data, tkhd));
        return match stored {
            Some(_) => stored,
            None => sample_entry_size(data, &mdia),
        };
    }
    None
}

fn header_size(data: &[u8], tkhd: Mp4Box) -> Option<(u32, u32)> {
    let matrix = tkhd.body
        + match data.get(tkhd.body)? {
            0 => 40,
            1 => 52,
            _ => return None,
        };
    if tkhd.end < matrix + 44 {
        return None;
    }
    let field = |index: usize| read_u32(data, matrix + index * 4);
    let (a, b) = (field(0)?.cast_signed(), field(1)?.cast_signed());
    let width = field(9)? >> 16;
    let height = field(10)? >> 16;
    if width == 0 || height == 0 {
        return None;
    }
    // A quarter turn either way: the picture's first axis lies along the screen's second.
    Some(if a.unsigned_abs() < b.unsigned_abs() {
        (height, width)
    } else {
        (width, height)
    })
}

/// The coded size in the first sample description.
fn sample_entry_size(data: &[u8], mdia: &[Mp4Box]) -> Option<(u32, u32)> {
    let minf = children(data, find(mdia, b"minf")?).ok()?;
    let stbl = children(data, find(&minf, b"stbl")?).ok()?;
    let stsd = find(&stbl, b"stsd")?;
    let entry = box_at(data, stsd.body + 8, stsd.end)?;
    let at = entry.body + 24;
    if entry.end < at + 4 {
        return None;
    }
    let size = read_u32(data, at)?;
    let (width, height) = (size >> 16, size & 0xffff);
    (width > 0 && height > 0).then_some((width, height))
}

/// The size of the picture of the MP4 or QuickTime file at `path`, as shown. Reads only the file's
/// index, however large it is. `None`: another container, or nothing to be learned from it.
pub fn video_size_of_file(path: &Path) -> Option<(u32, u32)> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let mut at = 0_u64;
    let mut head = [0_u8; 16];
    while at.checked_add(8)? <= length {
        file.seek(SeekFrom::Start(at)).ok()?;
        let available = usize::try_from((length - at).min(16)).ok()?;
        file.read_exact(&mut head[..available]).ok()?;
        let (kind, _, size) = header(&head[..available], 0)?;
        let size = if size == 0 { length - at } else { size };
        let end = at.checked_add(size)?;
        if &kind == b"moov" {
            if end > length || size > MOOV_READ_MAX {
                return None;
            }
            let mut moov = vec![0_u8; usize::try_from(size).ok()?];
            file.seek(SeekFrom::Start(at)).ok()?;
            file.read_exact(&mut moov).ok()?;
            let found = box_at(&moov, 0, moov.len())?;
            return shown_size(&moov, found);
        }
        at = end;
    }
    None
}

/// Files that say a lot about themselves, for the tests of what sends them.
#[cfg(test)]
pub(crate) mod fixture {
    /// Text that must not leave the device.
    pub(crate) const SECRETS: [&[u8]; 11] = [
        b"+47.3686+008.5392/",
        b"SomePhoneModel",
        b"com.apple.quicktime.location",
        b"XmpInAUuid",
        b"TopLevelMeta",
        b"LeftInFreeSpace",
        b"VendorHandlerName",
        b"GpsTrackSample",
        b"VendorEncoderName",
        b"ACME",
        b"file:///home/me/clips",
    ];
    pub(crate) const PICTURE: &[u8] = b"PICTUREPICTUREPICTURE";
    pub(crate) const SOUND: &[u8] = b"SOUNDSOUND";
    pub(crate) const CREATED: [u8; 4] = [0xdb, 0x12, 0x34, 0x56];

    pub(crate) fn boxed(kind: &[u8; 4], parts: &[&[u8]]) -> Vec<u8> {
        let body: Vec<u8> = parts.concat();
        let mut data = u32::try_from(body.len() + 8)
            .unwrap()
            .to_be_bytes()
            .to_vec();
        data.extend_from_slice(kind);
        data.extend_from_slice(&body);
        data
    }

    pub(crate) fn times_v0() -> Vec<u8> {
        let mut body = vec![0; 4];
        body.extend_from_slice(&CREATED);
        body.extend_from_slice(&CREATED);
        body
    }

    /// `tkhd` version 0; `turned`: filmed upright, stored as a quarter turn.
    pub(crate) fn tkhd(width: u32, height: u32, turned: bool) -> Vec<u8> {
        let mut body = times_v0();
        body.extend_from_slice(&[0; 28]); // id, duration, layer, volume and what lies between
        let one = 0x0001_0000_u32.to_be_bytes();
        let minus_one = 0xffff_0000_u32.to_be_bytes();
        let zero = [0; 4];
        let matrix: [[u8; 4]; 4] = if turned {
            [zero, one, minus_one, zero]
        } else {
            [one, zero, zero, one]
        };
        for line in [&matrix[0], &matrix[1], &zero, &matrix[2], &matrix[3], &zero] {
            body.extend_from_slice(line);
        }
        body.extend_from_slice(&zero);
        body.extend_from_slice(&zero);
        body.extend_from_slice(&0x4000_0000_u32.to_be_bytes());
        body.extend_from_slice(&(width << 16).to_be_bytes());
        body.extend_from_slice(&(height << 16).to_be_bytes());
        boxed(b"tkhd", &[&body])
    }

    pub(crate) fn hdlr(kind: &[u8; 4], name: &[u8]) -> Vec<u8> {
        boxed(b"hdlr", &[&[0; 8], kind, &[0; 12], name])
    }

    /// Reads like a phone's: the encoder's name and its maker in the sample description, also for
    /// the sound (whose description ends before the place of that name).
    fn sample_table(offset: u32, size: u32, picture: bool) -> Vec<u8> {
        let word = |value: u32| value.to_be_bytes();
        let mut entry = vec![0; 12];
        entry.extend_from_slice(b"ACME");
        entry.extend_from_slice(&[0; 8]);
        if picture {
            entry.extend_from_slice(&[0, 9, 0, 9]);
            entry.extend_from_slice(&[0; 14]);
            entry.extend_from_slice(b"VendorEncoderName\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0");
            entry.extend_from_slice(&[0; 4]);
        }
        boxed(
            b"stbl",
            &[
                &boxed(b"stsd", &[&[0; 4], &word(1), &boxed(b"avc1", &[&entry])]),
                &boxed(b"stco", &[&[0; 4], &word(1), &word(offset)]),
                &boxed(b"stsc", &[&[0; 4], &word(1), &word(1), &word(1), &word(1)]),
                &boxed(b"stsz", &[&[0; 4], &word(0), &word(1), &word(size)]),
                &boxed(b"xtra", &[b"SomePhoneModel"]),
            ],
        )
    }

    pub(crate) fn data_info(flags: u32) -> Vec<u8> {
        let entry = boxed(b"url ", &[&flags.to_be_bytes(), b"file:///home/me/clips"]);
        let dref = boxed(b"dref", &[&[0; 4], &1_u32.to_be_bytes(), &entry]);
        boxed(b"dinf", &[&dref])
    }

    pub(crate) fn trak(
        tkhd: &[u8],
        handler: &[u8; 4],
        name: &[u8],
        offset: u32,
        size: u32,
    ) -> Vec<u8> {
        let mdia = boxed(
            b"mdia",
            &[
                &boxed(b"mdhd", &[&times_v0()]),
                &hdlr(handler, name),
                &boxed(
                    b"minf",
                    &[
                        &boxed(b"vmhd", &[&[0; 12]]),
                        &data_info(1),
                        &sample_table(offset, size, handler == b"vide"),
                    ],
                ),
            ],
        );
        boxed(
            b"trak",
            &[tkhd, &mdia, &boxed(b"udta", &[b"SomePhoneModel"])],
        )
    }

    /// A phone's recording with everything vendors do: `(file, where the samples start)`.
    pub(crate) fn video(width: u32, height: u32, turned: bool) -> (Vec<u8>, usize) {
        let ftyp = boxed(b"ftyp", &[b"isom\0\0\0\0isom"]);
        let samples = [PICTURE, SOUND, b"GpsTrackSample"].concat();
        let build = |start: u32| {
            let picture = u32::try_from(PICTURE.len()).unwrap();
            let sound = u32::try_from(SOUND.len()).unwrap();
            let moov = boxed(
                b"moov",
                &[
                    &boxed(b"mvhd", &[&times_v0(), &[0; 88]]),
                    &boxed(b"udta", &[&boxed(b"\xa9xyz", &[b"+47.3686+008.5392/"])]),
                    &boxed(b"meta", &[b"com.apple.quicktime.location"]),
                    &trak(
                        &tkhd(width, height, turned),
                        b"vide",
                        b"VendorHandlerName",
                        start,
                        picture,
                    ),
                    &trak(
                        &tkhd(0, 0, false),
                        b"soun",
                        b"VendorHandlerName",
                        start + picture,
                        sound,
                    ),
                    &trak(
                        &tkhd(0, 0, false),
                        b"meta",
                        b"VendorHandlerName",
                        start + picture + sound,
                        14,
                    ),
                ],
            );
            [
                ftyp.clone(),
                boxed(b"meta", &[b"TopLevelMeta"]),
                boxed(b"free", &[b"LeftInFreeSpace"]),
                boxed(b"uuid", &[b"XmpInAUuid"]),
                moov,
            ]
            .concat()
        };
        let head = build(0).len() + 8;
        let mut file = build(u32::try_from(head).unwrap());
        file.extend_from_slice(&boxed(b"mdat", &[&samples]));
        (file, head)
    }
}

#[cfg(test)]
mod tests {
    use super::{fixture::*, *};

    fn contains(data: &[u8], needle: &[u8]) -> bool {
        data.windows(needle.len()).any(|window| window == needle)
    }

    /// A little video with `before` in front of it: the samples are where the table says.
    fn little_video(before: &[u8], mvhd: &[u8]) -> Vec<u8> {
        let moov = |offset: usize| {
            let track = trak(
                &tkhd(9, 9, false),
                b"vide",
                b"",
                u32::try_from(offset).unwrap(),
                u32::try_from(PICTURE.len()).unwrap(),
            );
            boxed(b"moov", &[&boxed(b"mvhd", &[mvhd]), &track])
        };
        let samples = before.len() + moov(0).len() + 8;
        [before, &moov(samples), &boxed(b"mdat", &[PICTURE])].concat()
    }

    #[test]
    fn nothing_but_sound_and_picture_remains() {
        let (mut file, samples) = video(1920, 1080, false);
        let length = file.len();
        for secret in SECRETS {
            assert!(contains(&file, secret), "the fixture has it");
        }
        assert_eq!(clean(&mut file), Ok(Some((1920, 1080))));
        for secret in SECRETS {
            assert!(
                !contains(&file, secret),
                "still there: {}",
                String::from_utf8_lossy(secret)
            );
        }
        assert!(!contains(&file, &CREATED), "the time of recording");
        // Nothing moved, so the offsets in the sample tables still hold.
        assert_eq!(file.len(), length);
        assert_eq!(&file[samples..][..PICTURE.len()], PICTURE);
        assert_eq!(&file[samples + PICTURE.len()..][..SOUND.len()], SOUND);
    }

    #[test]
    fn what_stays_is_still_a_video() {
        let (mut file, _) = video(640, 360, false);
        clean(&mut file).unwrap();
        let (tops, cut) = top_level(&file).unwrap();
        assert_eq!(cut, None);
        let names: Vec<&[u8; 4]> = tops.iter().map(|top| &top.kind).collect();
        assert_eq!(
            names,
            [b"ftyp", b"free", b"free", b"free", b"moov", b"mdat"]
        );
        let moov = find(&tops, b"moov").unwrap();
        let tracks: Vec<Mp4Box> = children(&file, moov)
            .unwrap()
            .into_iter()
            .filter(|child| &child.kind == b"trak")
            .collect();
        // The position track went, sound and picture stayed with their kinds.
        assert_eq!(tracks.len(), 2);
        let mdia = children(
            &file,
            find(&children(&file, tracks[1]).unwrap(), b"mdia").unwrap(),
        )
        .unwrap();
        assert_eq!(handler_of(&file, &mdia), Some(*b"soun"));
        assert_eq!(clean(&mut file.clone()), Ok(Some((640, 360))));
    }

    #[test]
    fn cleaning_twice_changes_nothing_more() {
        let (mut file, _) = video(640, 360, false);
        clean(&mut file).unwrap();
        let once = file.clone();
        clean(&mut file).unwrap();
        assert_eq!(file, once);
    }

    #[test]
    fn a_video_filmed_upright_counts_as_upright() {
        let (mut file, _) = video(1920, 1080, true);
        assert_eq!(clean(&mut file), Ok(Some((1080, 1920))));
    }

    #[test]
    fn the_size_comes_from_the_file_without_reading_all_of_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mov");
        let (file, _) = video(1920, 1080, true);
        std::fs::write(&path, &file).unwrap();
        assert_eq!(video_size_of_file(&path), Some((1080, 1920)));
        // The index behind a large `mdat` in the 64-bit form, as cameras write it.
        let mut large = 24_u32.to_be_bytes().to_vec();
        large.extend_from_slice(b"ftypisom\0\0\0\0isomiso2");
        large.extend_from_slice(&1_u32.to_be_bytes());
        large.extend_from_slice(b"mdat");
        large.extend_from_slice(&(16_u64 + 5).to_be_bytes());
        large.extend_from_slice(b"12345");
        let (tops, _) = top_level(&file).unwrap();
        let moov = find(&tops, b"moov").unwrap();
        large.extend_from_slice(&file[moov.start..moov.end]);
        std::fs::write(&path, &large).unwrap();
        assert_eq!(video_size_of_file(&path), Some((1080, 1920)));
        // Others have none to tell.
        std::fs::write(&path, b"\x1aE\xdf\xa3 a webm file").unwrap();
        assert_eq!(video_size_of_file(&path), None);
        assert_eq!(video_size_of_file(&dir.path().join("gone")), None);
    }

    #[test]
    fn the_size_falls_back_to_the_sample_description() {
        let mut entry = vec![0; 24];
        entry.extend_from_slice(&1280_u16.to_be_bytes());
        entry.extend_from_slice(&720_u16.to_be_bytes());
        let stsd = boxed(b"stsd", &[&[0; 8], &boxed(b"avc1", &[&entry])]);
        let mdia = boxed(
            b"mdia",
            &[
                &hdlr(b"vide", b""),
                &boxed(b"minf", &[&boxed(b"stbl", &[&stsd])]),
            ],
        );
        let moov = boxed(b"moov", &[&boxed(b"trak", &[&tkhd(0, 0, false), &mdia])]);
        let found = box_at(&moov, 0, moov.len()).unwrap();
        assert_eq!(shown_size(&moov, found), Some((1280, 720)));
    }

    #[test]
    fn other_containers_stay_as_they_are() {
        for original in [
            b"\x1aE\xdf\xa3 a webm file, long enough to look at".to_vec(),
            b"RIFF\x10\0\0\0AVI LIST and so on".to_vec(),
            Vec::new(),
            boxed(b"ftyp", &[b"isom"]),
        ] {
            let mut file = original.clone();
            assert_eq!(clean(&mut file), Err(NotCleanable));
            assert_eq!(file, original);
        }
    }

    #[test]
    fn audio_alone_is_no_video() {
        let moov = boxed(
            b"moov",
            &[
                &boxed(b"mvhd", &[&times_v0()]),
                &trak(&tkhd(0, 0, false), b"soun", b"", 0, 0),
            ],
        );
        let mut file = [moov, boxed(b"mdat", &[b"x"])].concat();
        assert_eq!(clean(&mut file), Err(NotCleanable));
    }

    #[test]
    fn a_track_that_goes_takes_its_samples_along_or_nothing_happens() {
        // Samples said to lie outside the `mdat`: that table can't be believed, so the position
        // would stay in the file.
        let moov = boxed(
            b"moov",
            &[
                &boxed(b"mvhd", &[&times_v0()]),
                &trak(&tkhd(9, 9, false), b"vide", b"", 0, 0),
                &trak(&tkhd(0, 0, false), b"meta", b"", 4, 8),
            ],
        );
        let original = [moov.clone(), boxed(b"mdat", &[b"x"])].concat();
        let mut file = original.clone();
        assert_eq!(clean(&mut file), Err(NotCleanable));
        assert_eq!(file, original);
        // In fragments, its samples can't be told from the others'.
        let original = [
            moov,
            boxed(b"moof", &[&boxed(b"mfhd", &[&[0; 8]])]),
            boxed(b"mdat", &[b"x"]),
        ]
        .concat();
        let mut file = original.clone();
        assert_eq!(clean(&mut file), Err(NotCleanable));
        assert_eq!(file, original);
    }

    #[test]
    fn fragments_lose_their_metadata_too() {
        let moov = boxed(
            b"moov",
            &[
                &boxed(b"mvhd", &[&times_v0()]),
                &trak(&tkhd(9, 9, false), b"vide", b"", 0, 0),
                &boxed(b"mvex", &[&boxed(b"trex", &[&[0; 24]])]),
            ],
        );
        let moof = boxed(
            b"moof",
            &[
                &boxed(b"mfhd", &[&[0; 8]]),
                &boxed(b"meta", &[b"TopLevelMeta"]),
                &boxed(
                    b"traf",
                    &[
                        &boxed(b"tfhd", &[&[0; 8]]),
                        &boxed(b"uuid", &[b"XmpInAUuid"]),
                    ],
                ),
            ],
        );
        let mut file = [moov, moof, boxed(b"mdat", &[b"x"])].concat();
        assert_eq!(clean(&mut file), Ok(Some((9, 9))));
        assert!(!contains(&file, b"TopLevelMeta"));
        assert!(!contains(&file, b"XmpInAUuid"));
        assert!(contains(&file, b"tfhd"));
    }

    #[test]
    fn the_large_box_form_is_blanked_too() {
        let mut meta = 1_u32.to_be_bytes().to_vec();
        meta.extend_from_slice(b"meta");
        meta.extend_from_slice(&28_u64.to_be_bytes());
        meta.extend_from_slice(b"TopLevelMeta");
        let mut file = little_video(&meta, &times_v0());
        clean(&mut file).unwrap();
        assert!(!contains(&file, b"TopLevelMeta"));
        let (tops, cut) = top_level(&file).unwrap();
        assert_eq!(cut, None);
        assert_eq!(&tops[0].kind, b"free");
        assert_eq!(tops[0].end, 28);
    }

    #[test]
    fn times_of_the_longer_form_go_too() {
        let mut body = vec![1, 0, 0, 0];
        body.extend_from_slice(&[0xab; 16]);
        body.extend_from_slice(&[0; 8]);
        let mut file = little_video(b"", &body);
        clean(&mut file).unwrap();
        assert!(!contains(&file, &[0xab; 16]));
    }

    #[test]
    fn a_vendors_trailer_is_cut_and_a_short_mdat_stays() {
        let (mut file, _) = video(9, 9, false);
        let length = file.len();
        file.extend_from_slice(b"\xff\xff\xff\xffSomePhoneModel");
        clean(&mut file).unwrap();
        assert_eq!(file.len(), length);
        assert!(!contains(&file, b"SomePhoneModel"));

        // Declares more than it has, but what the table points at is all there.
        let mut file = little_video(b"", &times_v0());
        let mdat = file.len() - PICTURE.len() - 8;
        file[mdat..mdat + 4].copy_from_slice(&1000_u32.to_be_bytes());
        let length = file.len();
        assert_eq!(clean(&mut file), Ok(Some((9, 9))));
        assert_eq!(file.len(), length);
        assert!(contains(&file, PICTURE));
    }

    /// Cutting behind a broken box must not make a video without samples look cleaned.
    #[test]
    fn samples_that_are_not_there_are_no_video() {
        // The `mdat` behind a box that can't be read.
        let good = little_video(b"", &times_v0());
        let mdat = good.len() - PICTURE.len() - 8;
        let original = [
            &good[..mdat],
            &[0, 0, 0, 2, b'x', b'x', b'x', b'x'],
            &good[mdat..],
        ]
        .concat();
        let mut file = original.clone();
        assert_eq!(clean(&mut file), Err(NotCleanable));
        assert_eq!(file, original);
        // Cut off in the samples.
        let mut file = good[..good.len() - 2].to_vec();
        let original = file.clone();
        assert_eq!(clean(&mut file), Err(NotCleanable));
        assert_eq!(file, original);
        // None at all.
        let moov = boxed(
            b"moov",
            &[
                &boxed(b"mvhd", &[&times_v0()]),
                &trak(&tkhd(9, 9, false), b"vide", b"", 0, 0),
            ],
        );
        let mut file = [moov, boxed(b"mdat", &[b"x"])].concat();
        assert_eq!(clean(&mut file), Err(NotCleanable));
    }

    #[test]
    fn a_track_with_its_samples_elsewhere_is_refused() {
        let good = little_video(b"", &times_v0());
        let self_contained = data_info(1);
        let at = good
            .windows(self_contained.len())
            .position(|window| window == self_contained)
            .unwrap();
        let mut file = good.clone();
        file[at..at + self_contained.len()].copy_from_slice(&data_info(0));
        let original = file.clone();
        assert_eq!(clean(&mut file), Err(NotCleanable));
        assert_eq!(file, original);
    }

    #[test]
    fn compact_sample_sizes_count_too() {
        let word = |value: u32| value.to_be_bytes();
        let mut compact = vec![0; 7];
        compact.push(4);
        compact.extend_from_slice(&word(2));
        compact.push(0x35);
        let stbl = [
            boxed(b"stco", &[&[0; 4], &word(1), &word(20)]),
            boxed(b"stsc", &[&[0; 4], &word(1), &word(1), &word(2), &word(1)]),
            boxed(b"stz2", &[&compact]),
        ]
        .concat();
        let mdia = boxed(b"mdia", &[&boxed(b"minf", &[&boxed(b"stbl", &[&stbl])])]);
        let parts = children(&mdia, box_at(&mdia, 0, mdia.len()).unwrap()).unwrap();
        let mut seen = Vec::new();
        let holder = 0..100.max(mdia.len());
        let any = track_chunks(&mdia, &parts, std::slice::from_ref(&holder), |chunk| {
            seen.push(chunk);
            Ok(())
        });
        // 3 + 5 bytes at offset 20.
        assert_eq!(any, Ok(true));
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], 20..28);
    }

    /// A file of nothing but tiny boxes must not cost many times its size in memory.
    #[test]
    fn too_many_boxes_are_a_refusal() {
        let many = |count: usize| b"\0\0\0\x08free".repeat(count);
        let at_top = [little_video(b"", &times_v0()), many(MAX_BOXES + 1)].concat();
        assert_eq!(clean(&mut at_top.clone()), Err(NotCleanable));

        let in_moov = boxed(
            b"moov",
            &[
                &boxed(b"mvhd", &[&times_v0()]),
                &many(MAX_BOXES + 1),
                &trak(&tkhd(9, 9, false), b"vide", b"", 0, 0),
            ],
        );
        assert_eq!(
            clean(&mut [in_moov, boxed(b"mdat", &[b"x"])].concat()),
            Err(NotCleanable)
        );

        // Each list within the limit, all together beyond it.
        let track = boxed(
            b"trak",
            &[
                &boxed(b"mdia", &[&hdlr(b"vide", b"")]),
                &many(MAX_BOXES / 2 + 1),
            ],
        );
        let in_tracks = boxed(
            b"moov",
            &[&boxed(b"mvhd", &[&times_v0()]), &track, &track, &track],
        );
        assert_eq!(
            clean(&mut [in_tracks, boxed(b"mdat", &[b"x"])].concat()),
            Err(NotCleanable)
        );

        // A dropped track's chunks count as well.
        let with_chunks = |count: usize| {
            let word = |value: u32| value.to_be_bytes();
            let number = u32::try_from(count).unwrap();
            let moov = |samples: u32| {
                let table = boxed(
                    b"stbl",
                    &[
                        &boxed(
                            b"stco",
                            &[&[0; 4], &word(number), &word(samples).repeat(count)],
                        ),
                        &boxed(b"stsc", &[&[0; 4], &word(1), &word(1), &word(1), &word(1)]),
                        &boxed(b"stsz", &[&[0; 4], &word(1), &word(number)]),
                    ],
                );
                let positions = boxed(b"mdia", &[&hdlr(b"meta", b""), &boxed(b"minf", &[&table])]);
                boxed(
                    b"moov",
                    &[
                        &boxed(b"mvhd", &[&times_v0()]),
                        &trak(&tkhd(9, 9, false), b"vide", b"", samples, 1),
                        &boxed(b"trak", &[&positions]),
                    ],
                )
            };
            let samples = u32::try_from(moov(0).len() + 8).unwrap();
            [moov(samples), boxed(b"mdat", &[&vec![1; count + 1]])].concat()
        };
        assert_eq!(clean(&mut with_chunks(1000)), Ok(Some((9, 9))));
        assert_eq!(clean(&mut with_chunks(MAX_BOXES + 2)), Err(NotCleanable));
    }

    /// However a file is broken, it never panics and never reads or writes past the end.
    #[test]
    fn damage_is_never_more_than_a_refusal() {
        let (file, _) = video(640, 360, true);
        for length in 0..file.len() {
            let mut cut = file[..length].to_vec();
            if clean(&mut cut).is_ok() {
                let once = cut.clone();
                assert!(clean(&mut cut).is_ok(), "cut at {length}");
                assert_eq!(cut, once);
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("damaged");
        for at in 0..file.len() {
            for value in [0x00, 0x01, 0x7f, 0xff] {
                let mut damaged = file.clone();
                damaged[at] = value;
                let mut cleaned = damaged.clone();
                if clean(&mut cleaned).is_ok() {
                    let once = cleaned.clone();
                    assert!(clean(&mut cleaned).is_ok(), "{value} at {at}");
                    assert_eq!(cleaned, once);
                }
                if at % 16 == 0 {
                    std::fs::write(&path, &damaged).unwrap();
                    let _ = video_size_of_file(&path);
                }
            }
        }
    }

    /// Fuzzing seed: `cargo test -p threesea-core --lib write_video_seed -- --ignored`
    #[test]
    #[ignore = "writes a fuzzing seed"]
    fn write_video_seed() {
        let seeds = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/video_clean");
        std::fs::create_dir_all(&seeds).unwrap();
        std::fs::write(seeds.join("video.mp4"), video(640, 360, true).0).unwrap();
    }
}
