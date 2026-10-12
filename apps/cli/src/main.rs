//! `mozu` — speech.cpp を使う道具のコマンド。スクリプトや Claude Code のフックから呼ぶ。

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};
use mozu_audio::{Player, Recorder};
use mozu_config::Config;
use mozu_speak::{SpeakOptions, Speaker};
use speechcpp_client::{Client, TranscribeOptions};

#[derive(Parser)]
#[command(
    name = "mozu",
    version,
    about = "speech.cpp を使う音声の道具（読み上げ・文字起こし）"
)]
struct Cli {
    /// 途中経過もログに出す
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 文章を読み上げる（-o でファイルに保存）
    Say {
        /// 読み上げる文章。省略するか "-" なら標準入力から読む
        text: Vec<String>,
        /// 声（既定は設定ファイルの speech.voice）
        #[arg(long)]
        voice: Option<String>,
        /// 速さ（既定は設定ファイルの speech.speed）
        #[arg(long)]
        speed: Option<f32>,
        /// 同じ値なら同じ読み方になる
        #[arg(long)]
        seed: Option<u64>,
        /// 再生せず、WAV に保存する
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 合成せず、読み上げる文（カタカナにした結果）と、辞書にない語を表示する
        #[arg(long)]
        show: bool,
    },
    /// 音声ファイルを文字にする（WAV 以外は FFmpeg で変換してから送る）
    Transcribe {
        file: PathBuf,
        /// 言語（既定は設定ファイルの dictation.language）
        #[arg(long)]
        language: Option<String>,
        /// 固有名詞や表記のヒント
        #[arg(long)]
        prompt: Option<String>,
    },
    /// マイクで聞いて、話し終わったら文字を出力する
    Listen {
        /// 話し終わったとみなす無音の長さ（ミリ秒）
        #[arg(long, default_value_t = 1200)]
        silence_ms: u64,
        /// 録音の上限（秒）
        #[arg(long, default_value_t = 60)]
        max_secs: u32,
    },
    /// 使える声の一覧
    Voices,
    /// 入力・出力デバイスの一覧
    Devices,
    /// speech.cpp が動いているか確かめる
    Health,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let default_filter = if cli.verbose {
        "info"
    } else {
        "warn,mozu_speak=info"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_filter));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .init();

    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("エラー: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> anyhow::Result<()> {
    if let Err(e) = mozu_config::load_dotenv() {
        tracing::warn!(".env を読めません: {e:#}");
    }
    let config = mozu_config::load()?;
    let client = Client::new(
        speechcpp_client::base_url_from_env(),
        config.request_timeout(),
    );

    match command {
        Command::Say {
            text,
            voice,
            speed,
            seed,
            output,
            show,
        } => {
            let text = text_or_stdin(&text)?;
            let options = SpeakOptions {
                voice: voice.unwrap_or_else(|| config.speech.voice.clone()),
                speed: speed.unwrap_or(config.speech.speed),
                seed,
                sentence_pause: Duration::from_millis(config.speech.sentence_pause_ms),
                paragraph_pause: Duration::from_millis(config.speech.paragraph_pause_ms),
            };
            let speaker = Speaker::new(client, mozu_config::load_readings()?, options);
            if show {
                return show_prepared(&speaker, &text);
            }
            match output {
                Some(path) => {
                    let wav = speaker.render(&text)?;
                    std::fs::write(&path, wav)
                        .with_context(|| format!("{} に書けません", path.display()))?;
                    eprintln!("保存しました: {}", path.display());
                }
                None => {
                    let player = Player::open(&config.speech.output_device)?;
                    speaker.speak(&text, &player, &AtomicBool::new(false))?;
                }
            }
        }
        Command::Transcribe {
            file,
            language,
            prompt,
        } => {
            let wav = read_as_wav(&file)?;
            let options = transcribe_options(&config, language, prompt);
            println!("{}", client.transcribe(&wav, &options)?.trim());
        }
        Command::Listen {
            silence_ms,
            max_secs,
        } => {
            let wav = listen(&config, Duration::from_millis(silence_ms), max_secs)?;
            let options = transcribe_options(&config, None, None);
            println!("{}", client.transcribe(&wav, &options)?.trim());
        }
        Command::Voices => {
            for model in client.models()?.iter().filter(|m| m.task == "synthesis") {
                for voice in model.voices.iter().filter(|v| v.name != "none") {
                    let description = voice.description.trim();
                    if description.is_empty() {
                        println!("{}", voice.name);
                    } else {
                        println!("{}\t{description}", voice.name);
                    }
                }
            }
        }
        Command::Devices => {
            println!("入力:");
            mozu_audio::list_inputs()?
                .iter()
                .for_each(|n| println!("  {n}"));
            println!("出力:");
            mozu_audio::list_outputs()?
                .iter()
                .for_each(|n| println!("  {n}"));
        }
        Command::Health => {
            client.health()?;
            println!("OK ({})", client.base_url());
        }
    }
    Ok(())
}

/// 引数の文章。省略または "-" なら標準入力。
fn text_or_stdin(args: &[String]) -> anyhow::Result<String> {
    if !args.is_empty() && args != ["-"] {
        return Ok(args.join(" "));
    }
    if std::io::stdin().is_terminal() {
        bail!("読み上げる文章を引数か標準入力で渡してください");
    }
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .context("標準入力を読めません")?;
    Ok(text)
}

fn show_prepared(speaker: &Speaker, text: &str) -> anyhow::Result<()> {
    let prepared = speaker.prepare(text);
    let mut out = std::io::stdout().lock();
    for (i, sentence) in prepared.sentences.iter().enumerate() {
        writeln!(out, "{:>3}  {}", i + 1, sentence.text)?;
    }
    if !prepared.unknown.is_empty() {
        writeln!(out, "\n辞書にない語: {}", prepared.unknown.join(", "))?;
        writeln!(
            out,
            "（{} に足すと読めるようになります）",
            mozu_config::readings_path()?.display()
        )?;
    }
    Ok(())
}

fn transcribe_options(
    config: &Config,
    language: Option<String>,
    prompt: Option<String>,
) -> TranscribeOptions {
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    TranscribeOptions {
        language: non_empty(language.unwrap_or_else(|| config.dictation.language.clone())),
        prompt: non_empty(prompt.unwrap_or_else(|| config.dictation.prompt.clone())),
        model: None,
    }
}

/// WAV はそのまま、それ以外は FFmpeg で 16kHz モノラルの WAV にする（speech.cpp は WAV しか読めない）。
fn read_as_wav(path: &Path) -> anyhow::Result<Vec<u8>> {
    let is_wav = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    if is_wav {
        return std::fs::read(path).with_context(|| format!("{} を読めません", path.display()));
    }
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-ac", "1", "-ar", "16000", "-f", "wav", "-"])
        .output()
        .context("FFmpeg を起動できません（PATH にありますか？）")?;
    if !output.status.success() {
        bail!(
            "FFmpeg で変換できません: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

/// 話し始めを待ち、話し終わり（`silence` だけ静かになる）まで録る。
fn listen(config: &Config, silence: Duration, max_secs: u32) -> anyhow::Result<Vec<u8>> {
    const STEP: Duration = Duration::from_millis(50);
    const CALIBRATE: Duration = Duration::from_millis(300);
    const WAIT_FOR_SPEECH: Duration = Duration::from_secs(10);

    let recorder = Recorder::start(&config.dictation.input_device, max_secs)?;
    let rate = recorder.sample_rate() as usize;
    let window = rate * STEP.as_millis() as usize / 1000;
    eprintln!("聞いています…（話し終わると止まります）");

    // 最初の 0.3 秒で周りの雑音の大きさを測り、その 3 倍を声の目安にする
    std::thread::sleep(CALIBRATE);
    let noise = mozu_audio::rms(&recorder.peek(0));
    let threshold = (noise * 3.0).max(0.01);

    let started = Instant::now();
    let mut read = 0;
    let mut speech_started = false;
    let mut quiet = Duration::ZERO;
    loop {
        std::thread::sleep(STEP);
        let new = recorder.peek(read);
        for chunk in new.chunks(window.max(1)) {
            if chunk.len() < window {
                break;
            }
            read += chunk.len();
            if mozu_audio::rms(chunk) > threshold {
                speech_started = true;
                quiet = Duration::ZERO;
            } else if speech_started {
                quiet += STEP;
            }
        }
        if speech_started && quiet >= silence {
            break;
        }
        if !speech_started && started.elapsed() > WAIT_FOR_SPEECH {
            bail!("声が聞こえませんでした");
        }
        if started.elapsed() > Duration::from_secs(max_secs.into()) {
            break;
        }
    }
    recorder.stop().to_wav()
}
