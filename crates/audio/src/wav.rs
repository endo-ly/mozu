//! WAV の書き出し（モノラルの f32 から）。

use std::io::Cursor;

use cpal::Sample;

/// 16bit PCM モノラルの WAV にする。
pub fn encode(samples: &[f32], sample_rate: u32) -> anyhow::Result<Vec<u8>> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::with_capacity(44 + samples.len() * 2));
    let mut writer = hound::WavWriter::new(&mut cursor, spec)?;
    for &s in samples {
        writer.write_sample(i16::from_sample(s.clamp(-1.0, 1.0)))?;
    }
    writer.finalize()?;
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_16bit_mono() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        let wav = encode(&samples, 16_000).unwrap();
        let mut reader = hound::WavReader::new(Cursor::new(wav)).unwrap();
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);
        let decoded: Vec<f32> = reader
            .samples::<i16>()
            .map(|s| s.unwrap().to_sample::<f32>())
            .collect();
        for (a, b) in samples.iter().zip(&decoded) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }
}
