use std::io::Cursor;
use ogg::PacketReader;
use opus_decoder::OpusDecoder;
use symphonia::core::audio::{AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tracing::{debug, warn};

use super::models::VoiceError;

pub const TARGET_SAMPLE_RATE: u32 = 16_000;

/// Decodes an arbitrary audio byte slice (Matrix voice notes in Ogg Opus, WAV, MP3, AAC, FLAC)
/// into a mono 16kHz normalized `f32` sample buffer ready for SenseVoiceSmall inference.
pub fn decode_audio_to_16k_mono(bytes: &[u8]) -> Result<Vec<f32>, VoiceError> {
    if bytes.is_empty() {
        return Err(VoiceError::AudioDecode("Empty audio byte buffer".to_string()));
    }

    let decode_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Matrix voice notes (MSC3245) are typically OGG Opus (starts with magic "OggS")
        if is_ogg_container(bytes) {
            match decode_ogg_opus(bytes) {
                Ok(samples) if !samples.is_empty() => {
                    debug!("Decoded {} 16kHz mono samples via Ogg Opus decoder", samples.len());
                    return Ok(samples);
                }
                Ok(_) => {
                    debug!("Ogg Opus decoder returned 0 samples, trying generic fallback");
                }
                Err(err) => {
                    debug!("Ogg Opus decode attempt returned: {:?}, attempting fallback", err);
                }
            }
        }

        // Generic audio format probe (WAV, MP3, AAC, FLAC, Vorbis)
        decode_generic_audio(bytes)
    }));

    match decode_result {
        Ok(res) => res,
        Err(panic_err) => {
            warn!("Audio decoder panicked on input: {:?}", panic_err);
            Err(VoiceError::AudioDecode("Audio decoding failed due to an internal decoder error".to_string()))
        }
    }
}

fn is_ogg_container(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && &bytes[0..4] == b"OggS"
}

/// Demuxes an Ogg Opus stream and decodes directly to 16,000 Hz mono f32 samples.
pub fn decode_ogg_opus(bytes: &[u8]) -> Result<Vec<f32>, VoiceError> {
    let catch_res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cursor = Cursor::new(bytes);
        let mut reader = PacketReader::new(cursor);
        let mut decoder = OpusDecoder::new(TARGET_SAMPLE_RATE, 1)
            .map_err(|e| VoiceError::AudioDecode(format!("Opus decoder init error: {:?}", e)))?;

        let mut all_samples: Vec<f32> = Vec::new();
        let mut packet_idx = 0;

        // Up to 120ms frame at 16kHz = 1920 samples
        let mut pcm_buf = vec![0.0f32; 1920];

        loop {
            match reader.read_packet() {
                Ok(Some(packet)) => {
                    if packet_idx == 0 {
                        if !packet.data.starts_with(b"OpusHead") {
                            return Err(VoiceError::AudioDecode(
                                "Ogg stream does not contain OpusHead header".to_string(),
                            ));
                        }
                    } else if packet_idx == 1 {
                        // OpusTags comment header - skip
                    } else {
                        match decoder.decode_float(&packet.data, &mut pcm_buf, false) {
                            Ok(samples_decoded) => {
                                all_samples.extend_from_slice(&pcm_buf[..samples_decoded]);
                            }
                            Err(err) => {
                                warn!("Skipping damaged Opus audio packet: {:?}", err);
                            }
                        }
                    }
                    packet_idx += 1;
                }
                Ok(None) => break,
                Err(err) => {
                    return Err(VoiceError::AudioDecode(format!("Ogg packet error: {:?}", err)));
                }
            }
        }

        Ok(all_samples)
    }));

    match catch_res {
        Ok(res) => res,
        Err(panic_err) => {
            warn!("Opus decoding loop panicked: {:?}", panic_err);
            Err(VoiceError::AudioDecode("Internal Opus packet decoding error".to_string()))
        }
    }
}

/// Fallback decoder supporting WAV, MP3, AAC, FLAC, and OGG Vorbis via Symphonia.
pub fn decode_generic_audio(bytes: &[u8]) -> Result<Vec<f32>, VoiceError> {
    let cursor = Cursor::new(bytes.to_vec());
    let mss = MediaSourceStream::new(Box::new(cursor), Default::default());
    let hint = Hint::new();
    let meta_opts: MetadataOptions = Default::default();
    let fmt_opts: FormatOptions = Default::default();

    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &fmt_opts, &meta_opts)
        .map_err(|e| VoiceError::AudioDecode(format!("Audio format probe error: {:?}", e)))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or_else(|| VoiceError::AudioDecode("No supported audio track found".to_string()))?;

    let track_id = track.id;
    let sample_rate = track.codec_params.sample_rate.unwrap_or(TARGET_SAMPLE_RATE);
    let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(1);

    let dec_opts: DecoderOptions = Default::default();
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &dec_opts)
        .map_err(|e| VoiceError::AudioDecode(format!("Codec decoder init error: {:?}", e)))?;

    let mut raw_mono_samples: Vec<f32> = Vec::new();

    while let Ok(packet) = format.next_packet() {
        if packet.track_id() != track_id {
            continue;
        }

        match decoder.decode(&packet) {
            Ok(decoded) => match decoded {
                AudioBufferRef::F32(buf) => {
                    for i in 0..buf.frames() {
                        let mut sum = 0.0;
                        for ch in 0..channels {
                            sum += buf.chan(ch)[i];
                        }
                        raw_mono_samples.push(sum / channels as f32);
                    }
                }
                AudioBufferRef::S16(buf) => {
                    for i in 0..buf.frames() {
                        let mut sum = 0.0;
                        for ch in 0..channels {
                            sum += buf.chan(ch)[i] as f32 / 32768.0;
                        }
                        raw_mono_samples.push(sum / channels as f32);
                    }
                }
                AudioBufferRef::U8(buf) => {
                    for i in 0..buf.frames() {
                        let mut sum = 0.0;
                        for ch in 0..channels {
                            sum += (buf.chan(ch)[i] as f32 - 128.0) / 128.0;
                        }
                        raw_mono_samples.push(sum / channels as f32);
                    }
                }
                _ => {}
            },
            Err(SymphoniaError::IoError(_)) => break,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(VoiceError::AudioDecode(format!("Decoder error: {:?}", e))),
        }
    }

    if sample_rate == TARGET_SAMPLE_RATE || raw_mono_samples.is_empty() {
        Ok(raw_mono_samples)
    } else {
        Ok(resample_linear(&raw_mono_samples, sample_rate, TARGET_SAMPLE_RATE))
    }
}

/// Simple linear sample rate conversion for mono f32 signals.
pub fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = to_rate as f64 / from_rate as f64;
    let target_len = (samples.len() as f64 * ratio).round() as usize;
    let mut resampled = Vec::with_capacity(target_len);

    for i in 0..target_len {
        let src_idx = i as f64 / ratio;
        let idx0 = src_idx.floor() as usize;
        let idx1 = (idx0 + 1).min(samples.len() - 1);
        let frac = (src_idx - idx0 as f64) as f32;
        let s0 = samples[idx0];
        let s1 = samples[idx1];
        resampled.push(s0 + frac * (s1 - s0));
    }
    resampled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_empty_buffer_returns_error() {
        let res = decode_audio_to_16k_mono(&[]);
        assert!(res.is_err());
    }

    #[test]
    fn test_decode_invalid_ogg_returns_error() {
        let invalid_ogg = b"OggS\x00\x02\x00\x00\x00\x00\x00\x00";
        let res = decode_audio_to_16k_mono(invalid_ogg);
        assert!(res.is_err());
    }

    #[test]
    fn test_resample_linear_identity() {
        let original = vec![0.1, 0.2, 0.3, 0.4];
        let res = resample_linear(&original, 16000, 16000);
        assert_eq!(res, original);
    }

    #[test]
    fn test_resample_linear_downsample() {
        let original = vec![0.0, 0.5, 1.0, 0.5];
        let res = resample_linear(&original, 32000, 16000);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_resample_linear_upsample() {
        let original = vec![0.0, 1.0];
        let res = resample_linear(&original, 8000, 16000);
        assert_eq!(res.len(), 4);
    }
}
