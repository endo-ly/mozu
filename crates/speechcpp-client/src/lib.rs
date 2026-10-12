//! speech.cpp (`speech serve`) の OpenAI 互換 HTTP API を呼ぶ同期クライアント。
//!
//! mozu のアプリはすべてこのクレートを通して speech.cpp を使う。
//! サーバーの起動・停止はしない（Windows サービス `speechcpp` が常駐している）。

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// サービス `speechcpp` の既定の待ち受け先。
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8012";

/// 接続先を指定する環境変数（`.env` から読み込む想定）。
pub const BASE_URL_ENV: &str = "MOZU_SPEECHCPP_URL";

/// 環境変数 `MOZU_SPEECHCPP_URL` の値。なければ（空なら）既定の待ち受け先。
pub fn base_url_from_env() -> String {
    std::env::var(BASE_URL_ENV)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_owned())
}

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

/// `POST /v1/audio/transcriptions` のオプション。
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
#[derive(Debug, Clone, Serialize)]
pub struct SpeechRequest {
    pub input: String,
    pub voice: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// speechcpp はサーバー側に既定値を持たないので、呼び出す側が毎回送る。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    pub response_format: String,
}

impl SpeechRequest {
    /// WAV で返す要求を作る。
    pub fn wav(input: impl Into<String>, voice: impl Into<String>) -> Self {
        Self {
            input: input.into(),
            voice: voice.into(),
            model: None,
            speed: None,
            seed: None,
            response_format: "wav".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    base_url: String,
    agent: ureq::Agent,
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
        }
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

    /// 文章を音声にして、音声ファイルの中身（既定は WAV）を返す。
    pub fn speech(&self, request: &SpeechRequest) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(request).map_err(|e| Error::Decode(e.to_string()))?;
        let mut res = self
            .agent
            .post(format!("{}/v1/audio/speech", self.base_url))
            .header("Content-Type", "application/json")
            .send(&json[..])?;
        check_status(&mut res)?;
        // 30 秒・48kHz・16bit で約 3MB。余裕を見て 64MB まで読む。
        Ok(res
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()?)
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
        let mut request = SpeechRequest::wav("今日はいい天気ですね。", "musa");
        request.speed = Some(1.4);
        let wav = client.speech(&request).unwrap();
        let options = TranscribeOptions {
            language: Some("ja".into()),
            ..Default::default()
        };
        let text = client.transcribe(&wav, &options).unwrap();
        assert!(text.contains("天気"), "認識結果: {text}");
    }
}
