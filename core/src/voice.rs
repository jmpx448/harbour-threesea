// SPDX-License-Identifier: AGPL-3.0-only
//! Voice messages in Opus (Ogg), for phones without a platform AAC encoder: the bridge records
//! raw audio, the core encodes it here, and decodes Opus for playback where no platform player
//! exists. Opus is royalty-free; Threema Android plays `audio/ogg` voice messages.

use opus_pure::{
    Application, MAX_PACKET_BYTES, MAX_PACKET_SAMPLES, OggOpusReader, OggOpusWriter, OpusEncoder,
    OpusHead, Trim,
};

/// The bridge records (and plays) 16-bit mono PCM at this rate.
pub const SAMPLE_RATE: i32 = 48_000;
/// Speech at Threema Android's AAC bitrate is more than enough for Opus.
const BITRATE: i32 = 24_000;

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("opus: {0}")]
    Opus(#[from] opus_pure::Error),
    #[error("no audio")]
    Empty,
    #[error("longer than we decode")]
    TooLong,
}

/// Longest voice message decoded (to PCM in memory: 20 minutes are 115 MB). Opus expands
/// about a hundredfold, so the file's size doesn't bound it (security audit 2026-10-27, S06).
const MAX_DECODED_SECONDS: usize = 20 * 60;

/// Encode 16-bit mono PCM at [`SAMPLE_RATE`] (little endian bytes) into an Ogg Opus file;
/// returns the file and the duration in seconds. Gapless: decodes back to exactly the input.
pub fn encode(pcm_le: &[u8]) -> Result<(Vec<u8>, f64), VoiceError> {
    let pcm: Vec<f32> = pcm_le
        .chunks_exact(2)
        .map(|s| f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0)
        .collect();
    if pcm.is_empty() {
        return Err(VoiceError::Empty);
    }
    let frame = usize::try_from(SAMPLE_RATE / 50).unwrap_or(960); // 20 ms
    let mut encoder = OpusEncoder::new(SAMPLE_RATE, 1, Application::Voip)?;
    encoder.bitrate_bps = BITRATE;
    let head = OpusHead::for_encoder(&encoder, SAMPLE_RATE.unsigned_abs());
    let total = pcm.len();
    // Feed the encoder delay as silence, then state where the audio ends (RFC 7845 §4.4).
    let frames = (total + usize::from(head.pre_skip)).div_ceil(frame);
    let final_granule = u64::from(head.pre_skip) + total as u64;
    let mut writer = OggOpusWriter::new(Vec::new(), head)?;
    let mut packet = vec![0_u8; MAX_PACKET_BYTES];
    let mut block = vec![0.0_f32; frame];
    for i in 0..frames {
        let start = (i * frame).min(total);
        let end = (start + frame).min(total);
        block[..end - start].copy_from_slice(&pcm[start..end]);
        block[end - start..].fill(0.0);
        let n = encoder.encode(&block, frame, &mut packet)?;
        if i + 1 == frames {
            let written = u64::try_from(writer.granule()).unwrap_or(0);
            let duration = u32::try_from(final_granule.saturating_sub(written)).unwrap_or(0);
            writer.write_packet_with_duration(&packet[..n], duration)?;
        } else {
            writer.write_packet(&packet[..n])?;
        }
    }
    let file = writer.finish()?;
    Ok((file, total as f64 / f64::from(SAMPLE_RATE)))
}

/// Decode an Ogg Opus file to 16-bit mono PCM at [`SAMPLE_RATE`] (little endian bytes).
pub fn decode(file: &[u8]) -> Result<Vec<u8>, VoiceError> {
    let mut reader = OggOpusReader::new(std::io::Cursor::new(file))?;
    let channels = usize::from(reader.head().channel_count).max(1);
    let mut decoder = reader.head().decoder(SAMPLE_RATE)?;
    let mut trim = Trim::new(reader.head(), SAMPLE_RATE, channels)?;
    let mut block = vec![0.0_f32; MAX_PACKET_SAMPLES * channels];
    let mut out = Vec::new();
    while let Some(packet) = reader.read_packet()? {
        let n = decoder.decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)?;
        for frame in trim
            .keep(&packet, &block[..n * channels])
            .chunks_exact(channels)
        {
            // Down-mix to mono.
            let sample = frame.iter().sum::<f32>() / channels as f32;
            #[expect(clippy::cast_possible_truncation, reason = "clamped to the i16 range")]
            let sample = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
            out.extend_from_slice(&sample.to_le_bytes());
        }
        if out.len() > MAX_DECODED_SECONDS * SAMPLE_RATE as usize * 2 {
            return Err(VoiceError::TooLong);
        }
    }
    if out.is_empty() {
        return Err(VoiceError::Empty);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_keeps_length_and_sound() {
        // 1.3 s of a 440 Hz tone.
        let samples = 62_400;
        let pcm: Vec<u8> = (0..samples)
            .flat_map(|i| {
                let t = f64::from(i) / f64::from(SAMPLE_RATE);
                #[expect(clippy::cast_possible_truncation, reason = "test tone")]
                let s = ((t * 440.0 * std::f64::consts::TAU).sin() * 12_000.0) as i16;
                s.to_le_bytes()
            })
            .collect();
        let (file, duration) = encode(&pcm).unwrap();
        assert!((duration - 1.3).abs() < 1e-9);
        assert_eq!(&file[..4], b"OggS");
        // About 24 kbit/s.
        assert!(file.len() < 6_000, "{} bytes", file.len());
        let back = decode(&file).unwrap();
        assert_eq!(back.len(), pcm.len(), "gapless");
        let energy = |b: &[u8]| {
            b.chunks_exact(2)
                .map(|s| f64::from(i16::from_le_bytes([s[0], s[1]])).powi(2))
                .sum::<f64>()
        };
        let ratio = energy(&back) / energy(&pcm);
        assert!((0.5..1.5).contains(&ratio), "energy ratio {ratio}");
    }

    #[test]
    fn rejects_garbage_and_empty() {
        assert!(encode(&[]).is_err());
        assert!(decode(b"not an ogg file").is_err());
    }
}

#[cfg(test)]
mod fuzz_seeds {
    /// Writes core/fuzz/seeds/voice_decode/: a voice message as the app sends it, so the
    /// fuzzer starts inside the Ogg/Opus decoder instead of at its header check.
    /// `cargo test -p threesea-core --lib write_voice_seed -- --ignored`
    #[test]
    #[ignore = "writes fuzzing seeds"]
    fn write_voice_seed() {
        let pcm: Vec<u8> = (0..48_000_u32 / 2)
            .flat_map(|i| {
                let sample = (f64::from(i) * 0.05).sin() * 8000.0;
                #[expect(clippy::cast_possible_truncation, reason = "a test tone")]
                (sample as i16).to_le_bytes()
            })
            .collect();
        let (ogg, _) = super::encode(&pcm).unwrap();
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/seeds/voice_decode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("half-second-tone.ogg"), ogg).unwrap();
    }
}
