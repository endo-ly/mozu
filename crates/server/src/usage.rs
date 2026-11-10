//! トレイと CLI で利用中の印を共有する。音声や文章は保存しない。
//! 排他的に開いたファイルを保持し、異常終了でも OS が利用中の印を解放する。

use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail, ensure};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::core::HSTRING;

#[derive(Clone)]
pub struct UsageSession {
    directory: PathBuf,
    mutex: HSTRING,
    ready_timeout: Duration,
}

pub struct UsageGuard {
    _file: File,
}

impl UsageSession {
    pub fn new(base_url: &str, ready_timeout: Duration) -> anyhow::Result<Self> {
        let root = mozu_config::data_dir()?.join("usage");
        // プロセス間で同じ名前になる固定のハッシュ。認証や秘密情報には使わない。
        let key = format!("{}:{base_url}", root.display())
            .bytes()
            .fold(0xcbf29ce484222325u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
            });
        Self::at(
            root.join(format!("{key:x}")),
            HSTRING::from(format!("Local\\mozu-use-{key:x}")),
            ready_timeout,
        )
    }

    fn at(directory: PathBuf, mutex: HSTRING, ready_timeout: Duration) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&directory)?;
        Ok(Self {
            directory,
            mutex,
            ready_timeout,
        })
    }

    fn lock(&self) -> anyhow::Result<Lock> {
        let handle = unsafe { CreateMutexW(None, false, &self.mutex)? };
        let status = unsafe { WaitForSingleObject(handle, 5000) };
        if status != WAIT_OBJECT_0 && status != WAIT_ABANDONED {
            unsafe {
                let _ = CloseHandle(handle);
            }
            bail!("speech.cpp の利用状態を確認できません（ロック待ち）");
        }
        Ok(Lock(handle))
    }

    pub fn begin(&self) -> anyhow::Result<UsageGuard> {
        let _lock = self.lock()?;
        ensure!(
            !self.directory.join("paused").exists(),
            "speech.cpp は手動停止中です。トレイから起動してください"
        );
        static SERIAL: AtomicU64 = AtomicU64::new(0);
        let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = self
            .directory
            .join(format!("{}-{time}-{serial}.use", std::process::id()));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path)?;
        Ok(UsageGuard { _file: file })
    }

    /// 確認と停止判断を同じロックの中で行い、新しい利用開始との競合を防ぐ。
    pub fn inspect<T>(&self, inspect: impl FnOnce(bool, bool) -> T) -> anyhow::Result<T> {
        let _lock = self.lock()?;
        let mut active = false;
        let mut completed = false;
        for item in std::fs::read_dir(&self.directory)? {
            let path = item?.path();
            if path.extension().is_none_or(|ext| ext != "use") {
                continue;
            }
            match OpenOptions::new()
                .read(true)
                .write(true)
                .share_mode(0)
                .open(&path)
            {
                Ok(file) => {
                    drop(file);
                    std::fs::remove_file(path)?;
                    completed = true;
                }
                Err(e) if e.raw_os_error() == Some(32) => active = true,
                Err(e) => return Err(e).context("speech.cpp の利用状態を読めません"),
            }
        }
        Ok(inspect(active, completed))
    }

    pub fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        let _lock = self.lock()?;
        let path = self.directory.join("paused");
        if paused {
            std::fs::write(path, [])?;
        } else if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }

    /// 利用中の印を保持したまま待つ。起動するのは mozu-tray だけ。
    pub fn wait_ready(
        &self,
        stop: &AtomicBool,
        mut healthy: impl FnMut() -> bool,
    ) -> anyhow::Result<bool> {
        let started = Instant::now();
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            ensure!(
                !self.directory.join("paused").exists(),
                "speech.cpp は手動停止中です。トレイから起動してください"
            );
            if healthy() {
                return Ok(!stop.load(Ordering::Relaxed));
            }
            ensure!(
                started.elapsed() < self.ready_timeout,
                "speech.cpp の起動を待ちきれません。mozu-tray が動いているか確認してください"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

struct Lock(HANDLE);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0);
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct IdleTimer {
    timeout: Duration,
    last_used: Instant,
}
impl IdleTimer {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            last_used: Instant::now(),
        }
    }
    pub fn reset(&mut self) {
        self.last_used = Instant::now();
    }
    pub fn expired(&mut self, active: bool, completed: bool, now: Instant) -> bool {
        if active || completed {
            self.last_used = now;
        }
        !active
            && !self.timeout.is_zero()
            && now.saturating_duration_since(self.last_used) >= self.timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_work_and_completion_reset_idle_deadline() {
        let start = Instant::now();
        let mut timer = IdleTimer {
            timeout: Duration::from_secs(1800),
            last_used: start,
        };
        let later = start + Duration::from_secs(3600);
        assert!(!timer.expired(true, false, later));
        assert!(!timer.expired(false, true, later));
        assert!(!timer.expired(false, false, later + Duration::from_secs(1799)));
        assert!(timer.expired(false, false, later + Duration::from_secs(1800)));
        timer.timeout = Duration::ZERO;
        assert!(!timer.expired(false, false, later + Duration::from_secs(99999)));
    }

    #[test]
    fn leases_survive_overlap_and_release_on_close() {
        let key = format!(
            "mozu-use-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let session = UsageSession::at(
            std::env::temp_dir().join(&key),
            HSTRING::from(format!("Local\\{key}")),
            Duration::from_secs(1),
        )
        .unwrap();
        let first = session.begin().unwrap();
        let second = session.clone().begin().unwrap();
        assert_eq!(session.inspect(|a, c| (a, c)).unwrap(), (true, false));
        drop(first);
        assert_eq!(session.inspect(|a, c| (a, c)).unwrap(), (true, true));
        drop(second);
        assert_eq!(session.inspect(|a, c| (a, c)).unwrap(), (false, true));
        assert_eq!(session.inspect(|a, c| (a, c)).unwrap(), (false, false));
        session.set_paused(true).unwrap();
        assert!(session.begin().is_err());
        session.set_paused(false).unwrap();
        std::fs::remove_dir(&session.directory).unwrap();
    }
}
