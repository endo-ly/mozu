//! WAV の読み書き（モノラルの f32 との変換）。

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

/// WAV を読み、モノラルの f32 とサンプリング周波数を返す。複数チャンネルは平均する。
pub fn decode(bytes: &[u8]) -> anyhow::Result<(Vec<f32>, u32)> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))?;
    let spec = reader.spec();
    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => reader.samples::<f32>().collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, 16) => reader
            .samples::<i16>()
            .map(|s| s.map(|s| s.to_sample::<f32>()))
            .collect::<Result<_, _>>()?,
        (hound::SampleFormat::Int, bits) => {
            let scale = (1i64 << (bits - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|s| s as f32 / scale))
                .collect::<Result<_, _>>()?
        }
    };
    let channels = spec.channels.max(1) as usize;
    let mono = interleaved
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect();
    Ok((mono, spec.sample_rate))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let samples = [0.0, 0.5, -0.5, 1.0, -1.0];
        let wav = encode(&samples, 16_000).unwrap();
        let (decoded, rate) = decode(&wav).unwrap();
        assert_eq!(rate, 16_000);
        for (a, b) in samples.iter().zip(&decoded) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }
}
