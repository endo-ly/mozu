//! 開始・終了・失敗の効果音。起動時に短い WAV を作っておき、PlaySound で非同期に鳴らす。

use std::io::Cursor;
use std::sync::OnceLock;

use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::core::PCWSTR;

#[derive(Debug, Clone, Copy)]
pub enum Cue {
    Start,
    Stop,
    Error,
}

struct Sounds {
    start: Vec<u8>,
    stop: Vec<u8>,
    error: Vec<u8>,
}

// SND_MEMORY|SND_ASYNC は再生が終わるまでメモリを読み続けるので、プロセスの終わりまで持つ。
static SOUNDS: OnceLock<Sounds> = OnceLock::new();
static ENABLED: OnceLock<bool> = OnceLock::new();

pub fn init(enabled: bool) {
    ENABLED.get_or_init(|| enabled);
    SOUNDS.get_or_init(|| Sounds {
        start: tones(&[(660.0, 0.06), (880.0, 0.07)]),
        stop: tones(&[(880.0, 0.06), (660.0, 0.07)]),
        error: tones(&[(220.0, 0.12), (0.0, 0.05), (220.0, 0.12)]),
    });
}

pub fn play(cue: Cue) {
    if !ENABLED.get().copied().unwrap_or(false) {
        return;
    }
    let Some(sounds) = SOUNDS.get() else { return };
    let wav = match cue {
        Cue::Start => &sounds.start,
        Cue::Stop => &sounds.stop,
        Cue::Error => &sounds.error,
    };
    // SND_MEMORY のときは文字列ではなく WAV の先頭を指すポインタを渡す。
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

/// `(周波数, 秒)` の並びを正弦波でつないだ WAV。周波数 0 は無音。
fn tones(notes: &[(f32, f32)]) -> Vec<u8> {
    const RATE: u32 = 44_100;
    const FADE: f32 = 0.005;
    const GAIN: f32 = 0.2;
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("メモリへの書き込み");
    for &(freq, secs) in notes {
        let n = (secs * RATE as f32) as usize;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            // 前後 5ms をなだらかにして、プツッという音を消す
            let envelope = (t / FADE).min((secs - t) / FADE).clamp(0.0, 1.0);
            let s = (t * freq * std::f32::consts::TAU).sin() * envelope * GAIN;
            writer
                .write_sample((s * i16::MAX as f32) as i16)
                .expect("メモリへの書き込み");
        }
    }
    writer.finalize().expect("メモリへの書き込み");
    cursor.into_inner()
}
