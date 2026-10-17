//! speech.cpp (`speech serve`) の OpenAI 互換 HTTP API を呼ぶ同期クライアント。
//!
//! mozu のアプリはすべてこのクレートを通して speech.cpp を使う。
//! サーバーの起動・停止はしない（managed のときは `mozu-server` が行う）。

use std::io::Read;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 接続先の既定。
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8012";

/// 接続先を指定する環境変数（`.env` から読み込む想定）。
pub const BASE_URL_ENV: &str = "MOZU_SPEECHCPP_URL";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// 接続できない、タイムアウトしたなど、HTTP の応答が得られなかった。
    #[error("speech.cpp に接続できません: {0}")]
    Transport(#[from] ureq::Error),
    /// サーバーがエラーを返した。
    #[error("speech.cpp がエラーを返しました (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    /// 応答の形が想定と違う。
    #[error("speech.cpp の応答を読めません: {0}")]
    Decode(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// `POST /v1/audio/transcriptions` のオプション。空文字は `None` と同じ（送らない）。
#[derive(Debug, Clone, Default)]
pub struct TranscribeOptions {
    /// 例: `ja`。`None` ならサーバー側の自動判定。
    pub language: Option<String>,
    /// 固有名詞や表記のヒント。
    pub prompt: Option<String>,
    /// 例: `Qwen3-ASR-1.7B`。`None` ならサーバーが載せている認識モデル。
    pub model: Option<String>,
}

/// `POST /v1/audio/speech` の要求。
///
/// 文章は丸ごと送ってよい。speech.cpp が文ごとに分けて合成し（長すぎる文は読点で切る）、
/// どの文も同じ種で読み、文と文の間を空けてつなぐ。
#[derive(Debug, Clone, Serialize)]
pub struct SpeechRequest {
    pub input: String,
    pub voice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    /// `None` ならサーバーが選ぶ（選んだ値は `SpeechStream::seed`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl SpeechRequest {
    pub fn new(input: impl Into<String>, voice: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            voice: voice.into(),
            model: None,
            speed: None,
            seed: None,
        }
    }
}

/// 合成した分から順に届く音声（16bit・リトルエンディアン・モノラルの PCM）。
/// 落とすと接続が切れ、サーバーは残りの合成をやめる。
pub struct SpeechStream {
    pub sample_rate: u32,
    /// 合成に使った種。同じ種を送ると同じ読み方になる。
    pub seed: Option<u64>,
    reader: ureq::BodyReader<'static>,
    /// 前回の読み出しで余った 1 バイト（サンプルの途中で切れたとき）。
    odd: Option<u8>,
}

impl SpeechStream {
    /// 次に届いた分を -1.0〜1.0 のサンプルにして返す。終わりまで読んだら `None`。
    pub fn next_samples(&mut self) -> Result<Option<Vec<f32>>> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            let n = self.reader.read(&mut buf).map_err(read_error)?;
            if n == 0 {
                return Ok(None);
            }
            let mut bytes = Vec::with_capacity(n + 1);
            bytes.extend(self.odd.take());
            bytes.extend_from_slice(&buf[..n]);
            if bytes.len() % 2 == 1 {
                self.odd = bytes.pop();
            }
            if !bytes.is_empty() {
                return Ok(Some(
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&b| i16::from_le_bytes(b) as f32 / 32768.0)
                        .collect(),
                ));
            }
        }
    }
}

/// `GET /v1/models` の 1 項目。
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub id: String,
    /// `synthesis`（合成）/ `recognition`（認識）/ `detection`（発話の検出）
    pub task: String,
    /// 合成モデルの声。`none`（参照音声なし）も含む。
    pub voices: Vec<VoiceInfo>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VoiceInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    agent: ureq::Agent,
    timeout: Duration,
}

impl Client {
    pub fn new(base_url: impl Into<String>, timeout: Duration) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            agent,
            timeout,
        }
    }

    /// 接続先を環境変数 `MOZU_SPEECHCPP_URL` にして作る。なければ（空なら）既定の接続先。
    pub fn from_env(timeout: Duration) -> Self {
        let base_url = std::env::var(BASE_URL_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned());
        Self::new(base_url, timeout)
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /health` が 200 を返すか確かめる。
    pub fn health(&self) -> Result<()> {
        let mut res = self.agent.get(format!("{}/health", self.base_url)).call()?;
        check_status(&mut res)?;
        Ok(())
    }

    /// WAV ファイルの中身を丸ごと送り、認識した文字列を返す。
    pub fn transcribe(&self, wav: &[u8], options: &TranscribeOptions) -> Result<String> {
        let mut form = Multipart::new();
        form.file("file", "audio.wav", "audio/wav", wav);
        let fields = [
            ("language", &options.language),
            ("prompt", &options.prompt),
            ("model", &options.model),
        ];
        for (name, value) in fields {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                form.text(name, value);
            }
        }
        let (content_type, body) = form.finish();

        let mut res = self
            .agent
            .post(format!("{}/v1/audio/transcriptions", self.base_url))
            .header("Content-Type", &content_type)
            .send(&body[..])?;
        check_status(&mut res)?;

        #[derive(Deserialize)]
        struct Transcription {
            text: String,
        }
        let text = res.body_mut().read_to_string()?;
        let parsed: Transcription =
            serde_json::from_str(&text).map_err(|e| Error::Decode(e.to_string()))?;
        Ok(parsed.text)
    }

    /// `GET /v1/models`。サーバーが載せているモデルと、合成モデルの声の一覧。
    pub fn models(&self) -> Result<Vec<ModelInfo>> {
        let mut res = self
            .agent
            .get(format!("{}/v1/models", self.base_url))
            .call()?;
        check_status(&mut res)?;

        #[derive(Deserialize)]
        struct List {
            data: Vec<Entry>,
        }
        #[derive(Deserialize)]
        struct Entry {
            id: String,
            #[serde(default)]
            speech: Option<Detail>,
        }
        #[derive(Deserialize)]
        struct Detail {
            #[serde(default)]
            task: String,
            #[serde(default)]
            voices: Vec<VoiceInfo>,
        }
        let text = res.body_mut().read_to_string()?;
        let list: List = serde_json::from_str(&text).map_err(|e| Error::Decode(e.to_string()))?;
        Ok(list
            .data
            .into_iter()
            .map(|e| {
                let (task, voices) = e.speech.map(|d| (d.task, d.voices)).unwrap_or_default();
                ModelInfo {
                    id: e.id,
                    task,
                    voices,
                }
            })
            .collect())
    }

    /// 文章を音声にし、合成した分から順に受け取る。
    ///
    /// 長い文章は合成し終えるまで時間がかかるので、待つ上限は応答が始まるまでにだけ掛ける。
    pub fn speech(&self, request: &SpeechRequest) -> Result<SpeechStream> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(flatten)]
            request: &'a SpeechRequest,
            response_format: &'static str,
        }
        let body = Body {
            request,
            response_format: "pcm",
        };
        let json = serde_json::to_vec(&body).map_err(|e| Error::Decode(e.to_string()))?;
        let mut res = self
            .agent
            .post(format!("{}/v1/audio/speech", self.base_url))
            .config()
            .timeout_global(None)
            .timeout_recv_response(Some(self.timeout))
            .build()
            .header("Content-Type", "application/json")
            .send(&json[..])?;
        check_status(&mut res)?;
        let header = |name: &str| {
            res.headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
        };
        let sample_rate = header("X-Sample-Rate")
            .and_then(|r| u32::try_from(r).ok())
            .ok_or_else(|| Error::Decode("X-Sample-Rate がありません".into()))?;
        let seed = header("X-Speech-Seed");
        Ok(SpeechStream {
            sample_rate,
            seed,
            reader: res.into_body().into_reader(),
            odd: None,
        })
    }
}

/// 本文を読む途中の失敗。
fn read_error(e: std::io::Error) -> Error {
    match e.downcast::<ureq::Error>() {
        Ok(e) => Error::Transport(e),
        Err(e) => Error::Transport(ureq::Error::Io(e)),
    }
}

/// 2xx 以外なら、OpenAI 形式のエラー本文からメッセージを取り出して返す。
fn check_status(res: &mut ureq::http::Response<ureq::Body>) -> Result<()> {
    let status = res.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }

    #[derive(Deserialize)]
    struct ErrorBody {
        error: ErrorDetail,
    }
    #[derive(Deserialize)]
    struct ErrorDetail {
        message: String,
    }
    let body = res.body_mut().read_to_string().unwrap_or_default();
    let message = serde_json::from_str::<ErrorBody>(&body)
        .map(|b| b.error.message)
        .unwrap_or(body);
    Err(Error::Api { status, message })
}

/// multipart/form-data の本文を組み立てる。
///
/// ureq 3 の multipart は semver の対象外（unversioned）なので、自前で組む。
struct Multipart {
    boundary: String,
    body: Vec<u8>,
}

impl Multipart {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        Self {
            boundary: format!("mozu-boundary-{nanos:x}"),
            body: Vec::new(),
        }
    }

    fn text(&mut self, name: &str, value: &str) {
        self.header(&format!(
            "Content-Disposition: form-data; name=\"{name}\"\r\n"
        ));
        self.body.extend_from_slice(value.as_bytes());
        self.body.extend_from_slice(b"\r\n");
    }

    fn file(&mut self, name: &str, file_name: &str, mime: &str, data: &[u8]) {
        self.header(&format!(
            "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\n\
             Content-Type: {mime}\r\n"
        ));
        self.body.extend_from_slice(data);
        self.body.extend_from_slice(b"\r\n");
    }

    fn header(&mut self, headers: &str) {
        self.body
            .extend_from_slice(format!("--{}\r\n{headers}\r\n", self.boundary).as_bytes());
    }

    /// `(Content-Type ヘッダーの値, 本文)` を返す。
    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_layout() {
        let mut form = Multipart::new();
        form.text("language", "ja");
        form.file("file", "a.wav", "audio/wav", b"RIFF");
        let boundary = form.boundary.clone();
        let (content_type, body) = form.finish();
        assert_eq!(
            content_type,
            format!("multipart/form-data; boundary={boundary}")
        );
        let expected = format!(
            "--{b}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nja\r\n\
             --{b}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\n\
             Content-Type: audio/wav\r\n\r\nRIFF\r\n--{b}--\r\n",
            b = boundary
        );
        assert_eq!(String::from_utf8(body).unwrap(), expected);
    }

    /// 稼働中の speechcpp に対して、合成した声をそのまま認識させる。
    /// `cargo test -p speechcpp-client -- --ignored` で実行する。
    #[test]
    #[ignore = "speechcpp サービスが必要"]
    fn roundtrip_against_service() {
        let client = Client::new(DEFAULT_BASE_URL, Duration::from_secs(60));
        client.health().unwrap();
        let mut request = SpeechRequest::new("今日はいい天気ですね。", "musa");
        request.speed = Some(1.4);
        let mut stream = client.speech(&request).unwrap();
        let mut samples = Vec::new();
        while let Some(chunk) = stream.next_samples().unwrap() {
            samples.extend(chunk);
        }
        let wav = wav_bytes(&samples, stream.sample_rate);
        let options = TranscribeOptions {
            language: Some("ja".into()),
            ..Default::default()
        };
        let text = client.transcribe(&wav, &options).unwrap();
        assert!(text.contains("天気"), "認識結果: {text}");
    }

    /// 16bit PCM モノラルの WAV（このクレートは WAV を扱わないので、テスト用に組む）。
    fn wav_bytes(samples: &[f32], rate: u32) -> Vec<u8> {
        let data = samples.len() as u32 * 2;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // PCM
        out.extend_from_slice(&1u16.to_le_bytes()); // モノラル
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data.to_le_bytes());
        for s in samples {
            out.extend_from_slice(&((s * 32767.0) as i16).to_le_bytes());
        }
        out
    }
}
