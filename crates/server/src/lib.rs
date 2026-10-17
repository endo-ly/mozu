//! speech.cpp（`speech serve`）を `mozu-tray` が起動・管理するための部品。
//!
//! 設定の `[server] managed = true` のときだけ使う。サーバーが応答しないときに起動し、
//! mozu-tray が終わると（異常終了でも）一緒に止まる。すでに応答があれば何もしない。

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

use anyhow::{Context, anyhow, bail};
use mozu_config::ServerConfig;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows::core::PCWSTR;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// mozu が speech.cpp を入れる場所（`scripts\setup-speech.ps1` と同じ）。`%APPDATA%\mozu\speech`
fn install_dir() -> anyhow::Result<PathBuf> {
    Ok(mozu_config::data_dir()?.join("speech"))
}

/// 声の既定の置き場。`%APPDATA%\mozu\voices`
fn default_voices_dir() -> anyhow::Result<PathBuf> {
    Ok(mozu_config::data_dir()?.join("voices"))
}

/// speech.exe を探す。設定の `exe` → mozu が入れたもの → PATH の順。
pub fn find_exe(configured: &str) -> Option<PathBuf> {
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        return path.is_file().then_some(path);
    }
    let installed = install_dir().ok().map(|d| d.join("bin").join("speech.exe"));
    installed
        .filter(|p| p.is_file())
        .or_else(|| find_in_path("speech.exe"))
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

/// `speech --version` の 1 行目（例: "speech.cpp 0.8.3, C API 3.1"）。
pub fn version(exe: &Path) -> anyhow::Result<String> {
    let output = Command::new(exe)
        .arg("--version")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .with_context(|| format!("{} を実行できません", exe.display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text.lines().next().unwrap_or_default().trim().to_owned())
}

/// 起動のしかた。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub exe: PathBuf,
    pub args: Vec<OsString>,
    /// `SPEECH_MODEL_DIR`。なければ speech.cpp の既定に任せる。
    pub model_dir: Option<PathBuf>,
    /// 標準出力・標準エラーの出力先。
    pub log: PathBuf,
}

impl Spec {
    /// 設定と接続先（`MOZU_SPEECHCPP_URL`）から、起動のしかたを決める。
    pub fn new(config: &ServerConfig, base_url: &str) -> anyhow::Result<Self> {
        let exe = find_exe(&config.exe).ok_or_else(|| {
            anyhow!(
                "speech.exe が見つかりません。`pwsh scripts\\setup-speech.ps1` で入れるか、\
                 設定の [server] exe に場所を書いてください"
            )
        })?;
        let voices_dir = if config.voices_dir.is_empty() {
            default_voices_dir()?
        } else {
            PathBuf::from(&config.voices_dir)
        };
        let model_dir = if !config.model_dir.is_empty() {
            Some(PathBuf::from(&config.model_dir))
        } else if exe.starts_with(install_dir()?) {
            Some(install_dir()?.join("models"))
        } else {
            None
        };
        let log = mozu_config::data_dir()?.join("logs").join("speech.log");
        Self::build(config, base_url, exe, &voices_dir, model_dir, log)
    }

    /// 起動するコマンドの全体（ログと診断用）。
    pub fn command_line(&self) -> String {
        std::iter::once(self.exe.as_os_str())
            .chain(self.args.iter().map(OsString::as_os_str))
            .map(OsStr::to_string_lossy)
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn build(
        config: &ServerConfig,
        base_url: &str,
        exe: PathBuf,
        voices_dir: &Path,
        model_dir: Option<PathBuf>,
        log: PathBuf,
    ) -> anyhow::Result<Self> {
        let (host, port) = host_port(base_url)?;
        let mut args: Vec<OsString> = vec!["serve".into()];
        args.extend(config.models.iter().map(OsString::from));
        for (name, file) in voices(voices_dir) {
            let mut arg = OsString::from(format!("{name}="));
            arg.push(file);
            args.extend(["--add-voice".into(), arg]);
        }
        args.extend([
            "--host".into(),
            host.into(),
            "--port".into(),
            port.to_string().into(),
            "--device".into(),
            config.device.clone().into(),
        ]);
        Ok(Self {
            exe,
            args,
            model_dir,
            log,
        })
    }
}

/// 接続先の URL から、起動するときの host と port を取り出す。自分の PC のアドレスだけ許す
/// （他の PC のサーバーを、こちらから起動することはできない）。
fn host_port(base_url: &str) -> anyhow::Result<(String, u16)> {
    let rest = base_url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow!("[server] managed は http:// の接続先にだけ使えます: {base_url}"))?;
    let authority = rest.split('/').next().unwrap_or(rest);
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("接続先にポートがありません: {base_url}"))?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        bail!("[server] managed は自分の PC（127.0.0.1）の接続先にだけ使えます: {base_url}");
    }
    let port = port
        .parse()
        .with_context(|| format!("ポートが数字ではありません: {base_url}"))?;
    Ok((host.trim_matches(['[', ']']).to_owned(), port))
}

/// 声の置き場にあるファイルを、（声の名前, ファイル）にする。
/// `lyre.voice.gguf` も `lyre.wav` も、名前は `lyre`。
fn voices(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let file = p.file_name()?.to_str()?.to_owned();
            let lower = file.to_lowercase();
            let name = if lower.ends_with(".voice.gguf") {
                &file[..file.len() - ".voice.gguf".len()]
            } else if lower.ends_with(".wav") {
                &file[..file.len() - ".wav".len()]
            } else {
                return None;
            };
            (!name.is_empty()).then(|| (name.to_owned(), p))
        })
        .collect();
    found.sort();
    found
}

/// 起動した speech.cpp。捨てる（または mozu-tray が終わる）と止まる。
pub struct Managed {
    child: Child,
    _job: Job,
}

impl Managed {
    pub fn start(spec: &Spec) -> anyhow::Result<Self> {
        let log = File::create(&spec.log)
            .with_context(|| format!("{} を作れません", spec.log.display()))?;
        let mut command = Command::new(&spec.exe);
        command
            .args(&spec.args)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .creation_flags(CREATE_NO_WINDOW);
        // 相対パスの exe は親が空になるので、その場合は作業フォルダを変えない
        if let Some(dir) = spec.exe.parent().filter(|d| !d.as_os_str().is_empty()) {
            command.current_dir(dir);
        }
        if let Some(dir) = &spec.model_dir {
            command.env("SPEECH_MODEL_DIR", dir);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("{} を起動できません", spec.exe.display()))?;
        // mozu-tray が落ちても、speech.cpp だけが残らないようにする
        let job = match Job::new().and_then(|job| job.assign(&child).map(|()| job)) {
            Ok(job) => job,
            Err(e) => {
                let _ = child.kill();
                return Err(e.context("speech.cpp を mozu-tray に結び付けられません"));
            }
        };
        tracing::info!("speech.cpp を起動しました: {}", spec.command_line());

        Ok(Self { child, _job: job })
    }

    /// 終わっていれば、その終わり方。
    pub fn exited(&mut self) -> Option<ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}

impl Drop for Managed {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        tracing::info!("speech.cpp を止めました");
    }
}

/// 持ち主が閉じると、中のプロセスをすべて終わらせるジョブオブジェクト。
struct Job(HANDLE);

// ハンドルは、閉じるまで持ち主のスレッドだけが使う
unsafe impl Send for Job {}

impl Job {
    fn new() -> anyhow::Result<Self> {
        unsafe {
            let handle = CreateJobObjectW(None, PCWSTR::null())?;
            let job = Self(handle);
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&raw const info).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            Ok(job)
        }
    }

    fn assign(&self, child: &Child) -> anyhow::Result<()> {
        unsafe { AssignProcessToJobObject(self.0, HANDLE(child.as_raw_handle()))? };
        Ok(())
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_port_accepts_loopback_only() {
        assert_eq!(
            host_port("http://127.0.0.1:8012").unwrap(),
            ("127.0.0.1".into(), 8012)
        );
        assert_eq!(
            host_port("http://localhost:9000/").unwrap(),
            ("localhost".into(), 9000)
        );
        assert!(host_port("http://192.168.0.5:8012").is_err());
        assert!(host_port("https://127.0.0.1:8012").is_err());
        assert!(host_port("http://127.0.0.1").is_err());
    }

    #[test]
    fn voices_are_named_after_files() {
        let dir = std::env::temp_dir().join(format!("mozu-voices-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["lyre.voice.gguf", "musa.WAV", "memo.txt", ".wav"] {
            std::fs::write(dir.join(name), b"").unwrap();
        }
        let names: Vec<_> = voices(&dir).into_iter().map(|(n, _)| n).collect();
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(names, ["lyre", "musa"]);
    }

    /// 本物の speech.cpp ではなく ping で、起動して捨てると止まることを確かめる。
    #[test]
    fn managed_process_stops_on_drop() {
        let spec = Spec {
            exe: PathBuf::from("ping.exe"),
            args: vec!["-t".into(), "127.0.0.1".into()],
            model_dir: None,
            log: std::env::temp_dir().join(format!("mozu-server-test-{}.log", std::process::id())),
        };
        let mut managed = Managed::start(&spec).unwrap();
        let pid = managed.child.id();
        assert!(managed.exited().is_none());
        assert!(is_running(pid));
        drop(managed);
        let _ = std::fs::remove_file(&spec.log);
        assert!(!is_running(pid));
    }

    /// 本物の speech.exe を、小さな検出モデルだけ・CPU・別ポートで起動して、`/health` が返ることを確かめる。
    /// （既存のサービスには触らない。speech.exe が PATH などにあるときだけ動かす）
    #[test]
    #[ignore = "speech.exe を起動する"]
    fn real_speech_serves_health() {
        use std::io::{Read, Write};
        let exe = find_exe("").expect("speech.exe が見つかりません");
        let spec = Spec {
            exe,
            args: [
                "serve",
                "silero-vad",
                "--host",
                "127.0.0.1",
                "--port",
                "8099",
                "--device",
                "cpu",
            ]
            .map(OsString::from)
            .to_vec(),
            model_dir: None,
            log: std::env::temp_dir().join("mozu-server-real-test.log"),
        };
        let mut managed = Managed::start(&spec).unwrap();
        let pid = managed.child.id();
        let mut answer = String::new();
        for _ in 0..60 {
            assert!(managed.exited().is_none(), "speech.exe が終了しました");
            if let Ok(mut stream) = std::net::TcpStream::connect("127.0.0.1:8099") {
                stream
                    .write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
                    .unwrap();
                stream.read_to_string(&mut answer).unwrap();
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        drop(managed);
        assert!(
            answer.starts_with("HTTP/1.0 200") || answer.starts_with("HTTP/1.1 200"),
            "{answer}"
        );
        assert!(!is_running(pid));
    }

    fn is_running(pid: u32) -> bool {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
    }

    #[test]
    fn args_follow_config() {
        let config = ServerConfig::default();
        let spec = Spec::build(
            &config,
            "http://127.0.0.1:8012",
            PathBuf::from("speech.exe"),
            Path::new("no-such-dir"),
            None,
            PathBuf::from("speech.log"),
        )
        .unwrap();
        let args: Vec<_> = spec
            .args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "serve",
                "irodori-tts-mf",
                "qwen3-asr-1.7b",
                "silero-vad",
                "--host",
                "127.0.0.1",
                "--port",
                "8012",
                "--device",
                "auto"
            ]
        );
    }
}
