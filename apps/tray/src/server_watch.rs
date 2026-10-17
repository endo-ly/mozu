//! speech.cpp の見張り。`/health` を定期的に確かめて、トレイのアイコンに反映する。
//!
//! `[server] managed = true` のときだけ、応答がなければ speech.cpp を起動する。
//! 起動や短時間での終了が続くときは、再起動の間隔を倍々に空け、失敗が続いたら通知する。
//! トレイのメニューからの `Control` で、手動の停止・起動を受け付ける。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use mozu_server::{Managed, Spec};
use speechcpp_client::Client;

use crate::tray::ServerState;
use crate::ui::UiHandle;

/// speech.cpp の状態を確かめる間隔。
const HEALTH_INTERVAL: Duration = Duration::from_secs(30);
/// 再起動の待ち時間の基本。失敗が続くと倍々に伸ばす（上限 `RETRY_MAX`）。
const RETRY_BASE: Duration = HEALTH_INTERVAL;
const RETRY_MAX: Duration = Duration::from_secs(600);
/// これより短い時間で終わったら「起動に失敗した」と数える
/// （モデルの読み込みに時間がかかって正常に動いていたときは、数えない）。
const QUICK_DEATH: Duration = Duration::from_secs(60);
/// 起動の失敗がこの回数続いたら、通知する。
const NOTIFY_AFTER: u32 = 3;
/// 起動した speech.cpp が応答するまで待つ上限。
const READY_LIMIT: Duration = Duration::from_secs(120);

/// トレイのメニューからの、speech.cpp の停止・起動の合図。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Control {
    /// managed の speech.cpp を手で止める。
    Stop,
    /// 手で止めた speech.cpp を起動する。
    Start,
}

/// 見張りのスレッドを起動する。`spec` があるとき（managed）だけ speech.cpp を起動する。
/// 送り手が消えたら（トレイが終わったら）戻り、`Managed` を捨てて speech.cpp も止める。
pub fn spawn(client: Client, ui: UiHandle, spec: Option<Spec>, control: mpsc::Receiver<Control>) {
    std::thread::spawn(move || {
        let mut managed: Option<Managed> = None;
        // いちばん最近 speech.cpp を起動した時刻（終了が「短時間だったか」の判定用）
        let mut started_at: Option<Instant> = None;
        let mut stopped = false;
        let mut failures: u32 = 0;
        let mut next_try = Instant::now();
        let mut notified = false;
        let mut last: Option<ServerState> = None;
        loop {
            if stopped {
                report(&ui, &mut last, ServerState::Stopped);
                // 起動の合図を待つ
                match control.recv() {
                    Ok(Control::Start) => {
                        tracing::info!("speech.cpp を起動します");
                        stopped = false;
                        failures = 0;
                        notified = false;
                        next_try = Instant::now();
                    }
                    // 止まっている最中の「止める」は、そのまま無視
                    Ok(Control::Stop) => {}
                    Err(_) => return,
                }
                continue;
            }

            let mut health = client.health();
            if health.is_err()
                && let Some(spec) = &spec
            {
                if let Some(status) = managed.as_mut().and_then(Managed::exited) {
                    let quick = started_at.is_some_and(|at| at.elapsed() < QUICK_DEATH);
                    tracing::warn!("speech.cpp が終了しました ({status})");
                    managed = None;
                    failures = if quick { failures + 1 } else { 0 };
                }
                if managed.is_none() && Instant::now() >= next_try {
                    report(&ui, &mut last, ServerState::Down);
                    match Managed::start(spec) {
                        Ok(child) => {
                            started_at = Some(Instant::now());
                            match wait_until_ready(&client, managed.insert(child), &control) {
                                Wait::Ready(result) => health = result,
                                // 待っている間に「止める」が来た。managed を捨てて止める
                                Wait::Stopped => {
                                    stopped = true;
                                    managed = None;
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            failures += 1;
                            tracing::error!("{e:#}");
                        }
                    }
                    if health.is_err() && failures > 0 {
                        next_try = Instant::now() + retry_delay(failures);
                        if failures >= NOTIFY_AFTER && !notified {
                            notified = true;
                            ui.notify();
                        }
                    }
                }
            }
            if health.is_ok() {
                failures = 0;
                notified = false;
            }
            let state = if health.is_ok() {
                ServerState::Up
            } else {
                ServerState::Down
            };
            if last != Some(state) {
                match &health {
                    Ok(()) => tracing::info!("speech.cpp に接続しました ({})", client.base_url()),
                    Err(e) => tracing::warn!("{e}"),
                }
            }
            report(&ui, &mut last, state);

            // 待つ。メニューからの合図が来たら、確認の間隔を待たずに反応する
            match control.recv_timeout(HEALTH_INTERVAL) {
                Ok(Control::Stop) => {
                    stopped = true;
                    managed = None;
                }
                Ok(Control::Start) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        }
    });
}

/// 失敗が続いたときの、次の起動までの待ち時間（30 秒 → 1 分 → 2 分 … 上限 10 分）。
fn retry_delay(failures: u32) -> Duration {
    let step = failures.saturating_sub(1).min(5);
    (RETRY_BASE * (1 << step)).min(RETRY_MAX)
}

/// 状態をトレイに送る。同じ状態の送り直しは飛ばす。
fn report(ui: &UiHandle, last: &mut Option<ServerState>, state: ServerState) {
    if *last != Some(state) {
        *last = Some(state);
        ui.server(state);
    }
}

enum Wait {
    /// 確かめ終えた（speech.cpp の応答の結果）。
    Ready(Result<(), speechcpp_client::Error>),
    /// 待っている間に「止める」の合図が来た。
    Stopped,
}

/// 起動した speech.cpp が応答するまで待つ（モデルの読み込みに数秒かかる）。
/// 初めてモデルを取得するときは長くかかるので、待ちきれなければ次の確認に回す。
fn wait_until_ready(
    client: &Client,
    server: &mut Managed,
    control: &mpsc::Receiver<Control>,
) -> Wait {
    let started = Instant::now();
    loop {
        let result = client.health();
        if result.is_ok() || server.exited().is_some() || started.elapsed() > READY_LIMIT {
            return Wait::Ready(result);
        }
        match control.recv_timeout(Duration::from_secs(1)) {
            Ok(Control::Stop) => return Wait::Stopped,
            Ok(Control::Start) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Wait::Ready(client.health()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_delay_doubles_up_to_limit() {
        let delays: Vec<_> = (1..=7).map(|n| retry_delay(n).as_secs()).collect();
        assert_eq!(delays, [30, 60, 120, 240, 480, 600, 600]);
    }
}
