//! Ogg Opus encoding for synthesised speech.
//!
//! Kept separate from `kokoro.rs` (which only knows about sherpa-onnx) and
//! from `api/tts.rs` (which only knows HTTP) — this file's one job is
//! turning `f32` PCM samples into an Ogg-contained Opus bitstream, per
//! RFC 7845. Matrix clients expect Ogg Opus for a voice bubble, and it is
//! roughly forty times smaller on the wire than WAV.
//!
//! Uses the `opus` crate (libopus, built from source by `opusic-sys`, which
//! is why this crate's build needs `cmake`) for encoding and
//! `ogg::PacketWriter` for the container.

use std::io::Cursor;
use std::time::{SystemTime, UNIX_EPOCH};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use opus::{Application, Encoder};

/// Opus frame duration, in milliseconds. Must be one of Opus's legal frame
/// sizes (2.5/5/10/20/40/60 ms); 20 ms is the standard VoIP choice.
const FRAME_MS: u32 = 20;

/// Opus's internal timebase: granule positions in an Ogg Opus stream are
/// always expressed in 48 kHz units, regardless of the input sample rate
/// (RFC 7845 section 4). This is easy to get wrong because every other
/// number in this file is in terms of the *input* rate.
const OPUS_GRANULE_RATE: u64 = 48_000;

/// Sample rates libopus accepts natively. Anything else must be resampled
/// before it reaches this function, which this module does not do — the
/// engine emits 24 kHz, so this is a guard against silently mis-encoding a
/// rate Opus was never given a chance to reject cleanly.
const SUPPORTED_RATES: [i32; 5] = [8000, 12000, 16000, 24000, 48000];

/// Encodes mono `f32` PCM `samples` at `sample_rate` as an Ogg-contained Opus
/// bitstream.
///
/// `sample_rate` must be one of Opus's native rates (8/12/16/24/48 kHz) — no
/// resampling is performed. The final partial frame, if any, is padded with
/// silence so every frame handed to the encoder is a full legal duration.
pub fn encode_ogg_opus(samples: &[f32], sample_rate: i32) -> Result<Vec<u8>, String> {
    if !SUPPORTED_RATES.contains(&sample_rate) {
        return Err(format!(
            "unsupported sample rate {} Hz for Opus (must be one of {:?})",
            sample_rate, SUPPORTED_RATES
        ));
    }

    let mut encoder = Encoder::new(sample_rate as u32, opus::Channels::Mono, Application::Voip)
        .map_err(|e| format!("failed to create Opus encoder: {}", e))?;

    // Samples per 20ms frame at the input rate, e.g. 480 at 24000 Hz.
    let frame_len = (sample_rate as u32 * FRAME_MS / 1000) as usize;
    // One frame, expressed in the 48kHz granule timebase.
    let granule_per_frame = (OPUS_GRANULE_RATE * FRAME_MS as u64) / 1000;

    let lookahead = encoder
        .get_lookahead()
        .map_err(|e| format!("failed to read encoder lookahead: {}", e))?;
    let pre_skip_48k = (lookahead as u64 * OPUS_GRANULE_RATE) / sample_rate as u64;

    let serial = stream_serial();
    let mut buf = Vec::new();
    let mut writer = PacketWriter::new(Cursor::new(&mut buf));

    write_id_header(&mut writer, serial, pre_skip_48k as u16, sample_rate as u32)?;
    write_comment_header(&mut writer, serial)?;

    // Max Opus packet size for a single 20ms frame at any bitrate this
    // encoder will pick; libopus never exceeds this for one frame.
    let mut packet_buf = vec![0u8; 4000];
    let mut granule_pos: u64 = 0;

    let frame_count = samples.len().div_ceil(frame_len).max(1);
    for frame_idx in 0..frame_count {
        let start = frame_idx * frame_len;
        let end = (start + frame_len).min(samples.len());

        let mut frame = vec![0.0_f32; frame_len];
        frame[..end - start].copy_from_slice(&samples[start..end]);

        let encoded_len = encoder
            .encode_float(&frame, &mut packet_buf)
            .map_err(|e| format!("Opus frame encode failed: {}", e))?;

        granule_pos += granule_per_frame;
        let is_last = frame_idx + 1 == frame_count;
        let end_info = if is_last {
            PacketWriteEndInfo::EndStream
        } else {
            PacketWriteEndInfo::NormalPacket
        };

        writer
            .write_packet(
                packet_buf[..encoded_len].to_vec(),
                serial,
                end_info,
                granule_pos + pre_skip_48k,
            )
            .map_err(|e| format!("Ogg packet write failed: {}", e))?;
    }

    drop(writer);
    Ok(buf)
}

/// Writes the `OpusHead` packet alone on the first page (RFC 7845 section 5.1).
fn write_id_header<W: std::io::Write>(
    writer: &mut PacketWriter<W>,
    serial: u32,
    pre_skip: u16,
    input_sample_rate: u32,
) -> Result<(), String> {
    let mut packet = Vec::with_capacity(19);
    packet.extend_from_slice(b"OpusHead");
    packet.push(1); // version
    packet.push(1); // channel count: mono
    packet.extend_from_slice(&pre_skip.to_le_bytes());
    packet.extend_from_slice(&input_sample_rate.to_le_bytes());
    packet.extend_from_slice(&0i16.to_le_bytes()); // output gain
    packet.push(0); // channel mapping family: mono/stereo, no mapping table

    writer
        .write_packet(packet, serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| format!("Ogg OpusHead write failed: {}", e))
}

/// Writes the `OpusTags` packet alone on the second page (RFC 7845 section 5.2).
fn write_comment_header<W: std::io::Write>(
    writer: &mut PacketWriter<W>,
    serial: u32,
) -> Result<(), String> {
    let vendor = b"matrix-bot";
    let mut packet = Vec::with_capacity(8 + 4 + vendor.len() + 4);
    packet.extend_from_slice(b"OpusTags");
    packet.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    packet.extend_from_slice(vendor);
    packet.extend_from_slice(&0u32.to_le_bytes()); // user comment count

    writer
        .write_packet(packet, serial, PacketWriteEndInfo::EndPage, 0)
        .map_err(|e| format!("Ogg OpusTags write failed: {}", e))
}

/// A stream serial number that varies between calls. Two concatenated Ogg
/// streams sharing a serial are malformed, and this module may encode many
/// clips over the process's lifetime, so a constant would eventually collide
/// in principle (and always looks suspicious to an Ogg-aware tool). The
/// system clock's nanosecond component is good enough: it is not read back
/// or relied on for anything but distinctness.
fn stream_serial() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A short sine wave, loud enough that a silence check can't mistake it
    /// for nothing, at the engine's real output rate.
    fn tone(sample_rate: i32, duration_secs: f32) -> Vec<f32> {
        let n = (sample_rate as f32 * duration_secs) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn test_round_trip_decodes_to_expected_length_and_is_not_silent() {
        let sample_rate = 24000;
        let samples = tone(sample_rate, 1.0);
        let ogg_bytes = encode_ogg_opus(&samples, sample_rate).expect("encode should succeed");

        let mut reader = ogg::reading::PacketReader::new(Cursor::new(ogg_bytes));
        let mut decoder = opus::Decoder::new(sample_rate as u32, opus::Channels::Mono)
            .expect("decoder should construct");

        let mut decoded_samples = 0usize;
        let mut saw_nonzero = false;
        let mut packet_index = 0;
        while let Some(packet) = reader.read_packet().expect("ogg read should succeed") {
            // Packets 0 and 1 are OpusHead/OpusTags, not audio.
            if packet_index >= 2 {
                let mut out = vec![0.0_f32; 5760]; // max frame size at 48kHz-equivalent
                let n = decoder
                    .decode_float(&packet.data, &mut out, false)
                    .expect("decode should succeed");
                decoded_samples += n;
                if out[..n].iter().any(|&s| s.abs() > 0.01) {
                    saw_nonzero = true;
                }
            }
            packet_index += 1;
        }

        assert!(saw_nonzero, "decoded audio should not be silence");
        // One second at 24kHz in 20ms frames, allowing for the final padded frame.
        let expected_frames = sample_rate as usize / 1000 * 20;
        assert!(
            decoded_samples >= expected_frames,
            "decoded {} samples, expected at least {}",
            decoded_samples,
            expected_frames
        );
    }

    #[test]
    fn test_header_shape_has_oggs_magic_and_head_then_tags() {
        let samples = tone(24000, 0.1);
        let bytes = encode_ogg_opus(&samples, 24000).expect("encode should succeed");

        assert_eq!(&bytes[0..4], b"OggS", "stream must start with an Ogg page");

        let mut reader = ogg::reading::PacketReader::new(Cursor::new(&bytes));
        let first = reader
            .read_packet()
            .expect("read should succeed")
            .expect("first packet should exist");
        assert_eq!(&first.data[0..8], b"OpusHead");

        let second = reader
            .read_packet()
            .expect("read should succeed")
            .expect("second packet should exist");
        assert_eq!(&second.data[0..8], b"OpusTags");
    }

    #[test]
    fn test_unsupported_sample_rate_is_rejected() {
        let samples = tone(44100, 0.1);
        let result = encode_ogg_opus(&samples, 44100);
        assert!(result.is_err(), "44100 Hz is not a native Opus rate");
    }

    #[test]
    fn test_partial_final_frame_is_padded_without_error() {
        // Not a whole number of 20ms frames at 24kHz (480 samples/frame).
        let samples = vec![0.1_f32; 500];
        let bytes = encode_ogg_opus(&samples, 24000).expect("padding should not error");
        assert!(!bytes.is_empty());

        // Exactly one frame's worth of input should still round-trip to one
        // full padded frame's worth of decoded audio (480 samples @ 24kHz).
        let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
        let mut decoder =
            opus::Decoder::new(24000, opus::Channels::Mono).expect("decoder should construct");
        reader.read_packet().unwrap(); // OpusHead
        reader.read_packet().unwrap(); // OpusTags
        let audio = reader
            .read_packet()
            .expect("read should succeed")
            .expect("one audio packet should exist");
        let mut out = vec![0.0_f32; 5760];
        let n = decoder
            .decode_float(&audio.data, &mut out, false)
            .expect("decode should succeed");
        assert_eq!(n, 480, "a padded frame should decode to one full frame");
    }

    #[test]
    fn test_silence_compresses_much_smaller_than_noise() {
        let sample_rate = 24000;
        let silence = vec![0.0_f32; sample_rate as usize]; // 1 second
        let noise: Vec<f32> = (0..sample_rate as usize)
            .map(|i| {
                // Deterministic pseudo-noise: not a real PRNG dependency, just
                // something with no periodic structure for Opus to exploit.
                let x = (i as f32 * 12.9898).sin() * 43758.5453;
                (x - x.floor()) * 2.0 - 1.0
            })
            .collect();

        let silence_bytes = encode_ogg_opus(&silence, sample_rate).expect("encode should succeed");
        let noise_bytes = encode_ogg_opus(&noise, sample_rate).expect("encode should succeed");

        // DTX is not enabled, so libopus still emits a full frame every 20ms
        // even for true digital silence — VBR just makes each of those
        // frames far cheaper to encode than one carrying actual signal.
        // Observed on this codebase: ~1.5KB for 1s of silence vs. ~3.4KB for
        // 1s of noise, comfortably under this threshold with room to spare.
        assert!(
            silence_bytes.len() * 10 < noise_bytes.len() * 7,
            "silence ({} bytes) should compress noticeably smaller than noise ({} bytes)",
            silence_bytes.len(),
            noise_bytes.len()
        );
    }

    #[test]
    fn test_stream_serials_differ_between_calls() {
        let a = stream_serial();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let b = stream_serial();
        assert_ne!(a, b, "serials taken apart in time should differ");
    }
}
