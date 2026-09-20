//! WAV encoding for synthesised speech.
//!
//! Kept separate from `engine.rs` (which only knows about sherpa-onnx) and
//! from `mod.rs` (which only knows about Matrix) — this file's one job is
//! turning `f32` PCM samples into 16-bit PCM WAV bytes with `hound`, already a
//! dependency for the rest of the audio pipeline.

use std::io::Cursor;

use hound::{SampleFormat, WavSpec, WavWriter};

/// Encodes mono `f32` PCM samples in `[-1.0, 1.0]` as 16-bit PCM mono WAV
/// bytes at `sample_rate`.
pub fn encode_wav(samples: &[f32], sample_rate: i32) -> Result<Vec<u8>, String> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: sample_rate.max(0) as u32,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer =
            WavWriter::new(&mut cursor, spec).map_err(|e| format!("WAV header: {}", e))?;
        for &sample in samples {
            let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
            writer
                .write_sample(scaled)
                .map_err(|e| format!("WAV sample write: {}", e))?;
        }
        writer
            .finalize()
            .map_err(|e| format!("WAV finalize: {}", e))?;
    }

    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encodes_a_valid_wav_header_and_can_be_read_back() {
        let samples = vec![0.0_f32, 0.5, -0.5, 1.0, -1.0];
        let bytes = encode_wav(&samples, 24000).expect("encoding should succeed");

        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");

        let mut reader =
            hound::WavReader::new(Cursor::new(bytes)).expect("the bytes should parse as WAV");
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 24000);
        assert_eq!(reader.spec().bits_per_sample, 16);

        let read_back: Vec<i16> = reader
            .samples::<i16>()
            .collect::<Result<_, _>>()
            .expect("samples should decode");
        assert_eq!(read_back.len(), samples.len());
        assert_eq!(read_back[3], i16::MAX);
        assert_eq!(read_back[4], -i16::MAX);
    }

    #[test]
    fn test_empty_samples_still_produce_a_valid_wav() {
        let bytes = encode_wav(&[], 24000).expect("encoding empty audio should succeed");
        let reader =
            hound::WavReader::new(Cursor::new(bytes)).expect("the bytes should parse as WAV");
        assert_eq!(reader.duration(), 0);
    }
}
