# mozu — 声の基盤

音声認識と音声合成を使うアプリをまとめるリポジトリ。名前は百舌（モズ。「百の舌」を持ち、ほかの鳥の声をまねる鳥）から。

## 構成

```
mozu/
├─ apps/
│  ├─ tray/              # mozu-tray.exe — 常駐（音声入力・読み上げ）
│  └─ cli/               # mozu.exe — コマンド
├─ crates/
│  ├─ speechcpp-client/  # speech.cpp の HTTP API
│  ├─ config/            # mozu-config — 設定・.env・%APPDATA%\mozu の置き場（tray と cli で共有）
│  ├─ audio/             # mozu-audio — 録音・再生（cpal）、WAV の書き出し
│  ├─ speak/             # mozu-speak — 読み上げ。下ごしらえ（Markdown の除去・カタカナ化）→ 合成 → 届いた分から再生
│  └─ server/            # mozu-server — speech.cpp の探索・起動・停止（managed のときだけ）
├─ scripts/
│  ├─ check.ps1          # 整形・clippy（警告はエラー）・テスト
│  ├─ install.ps1        # check → ビルド → bin\ に入れ替え →（なければ speech.cpp を導入）→ 起動 → mozu doctor
│  └─ setup-speech.ps1   # speech.cpp（Windows 用 Vulkan 版）を GitHub から取り、チェックサムを確かめて %APPDATA%\mozu\speech に入れる
└─ bin/                  # インストール先（Git に入れない。ユーザーの PATH に入っている）
```

## 決まり

- **エンジンは speech.cpp。** この PC では Windows サービス `speechcpp` が `127.0.0.1:8012` で常駐している（マニフェストは `C:\svc\runtimes\winsvc\manifests\speechcpp.yaml`）。アプリは `speechcpp-client` で直接呼び、中継サーバーは置かない
- **mozu は、既定ではサーバーを起動・停止しない。** 応答がなければエラーとして知らせるだけ。`[server] managed = true` のときだけ mozu-tray が起動し、終了時に止める（ジョブオブジェクトで、mozu-tray が落ちても一緒に止まる）
- **この PC では `managed = false` のまま。** エージェントがサーバーを起動してはいけない。本物の speech.exe で確かめるときは、サービスと別のポート・小さなモデル・CPU にする（`cargo test -p mozu-server -- --ignored`）
- **常駐は mozu-tray 1 つ。** 新しい機能は、別の常駐アプリを作らず、ホットキーを足して tray に入れる。Windows サービスにはできない（セッション 0 ではフック・SendInput・クリップボード・トレイが使えない）
- 共通の処理は、2 つ目の使い道ができた時点で `crates/` に移す
- 利用者のファイル（`config.toml`・`readings.toml`）は、なければ作り、あれば書き換えない（足りない項目は既定値）。`config.toml` のひな形と `Config::default()` はテストで一致を確かめている
- 録音は保存しない（`keep_last_wav = true` のときだけ直前の 1 回を残す）。認識した文字列や読み上げた文は、`RUST_LOG=debug` のときだけログに出す
- `mozu` コマンドは、ログを標準エラー、結果（文字起こし・`--show`）を標準出力に出す

## 開発

```powershell
pwsh scripts\check.ps1     # 整形の確認・clippy（-D warnings）・テスト
pwsh scripts\install.ps1   # check → ビルド → 動いているものを終了 → bin\ に置く → 起動
cargo test -- --ignored    # speechcpp サービス・マイク・スピーカーを使うテスト
```

- 変更を入れたら `check.ps1` を通す。lint はルートの `Cargo.toml` の `[workspace.lints]`、依存のバージョンは `[workspace.dependencies]` にまとめる
- キー入力・貼り付け・トレイは自動テストにできないので、動作に関わる変更のあとは `install.ps1` で入れ替えて手で確かめる
- Rust 2024 edition。コメント・ログ・エラーメッセージは日本語

## 読み上げ（mozu-speak）

- **英単語はすべてカタカナにしてから合成する**（Irodori-TTS は英語の読みが崩れる）。辞書は利用者の `readings.toml`（配布用の例は `readings.example.toml`）。読めない語は英語のまま残し、ログと `mozu say --show` で「辞書にない語」として出す
- Markdown の記号・URL・コードの塊は読まない。行の区切りは残す（speech.cpp が改行を文の切れ目として扱う）
- **文の区切り・間・種は speech.cpp に任せる。** 下ごしらえした文章を丸ごと 1 回で送ると、speech.cpp が文ごとに同じ種で合成し、間を空けてつなぐ。mozu で足すのは、speech.cpp にできないこと（Markdown の除去・カタカナ化・再生と停止）だけ
- 音声は `response_format: "pcm"` で、合成した文から順に受け取って鳴らす。止めるときは接続を切る（speech.cpp は残りの合成をやめる）
- `speed` は必ず送る（既定 1.4。speech.cpp の既定 1 は遅い）

## mozu-tray

| モジュール | 役割 |
|---|---|
| `main.rs` | 起動（二重起動の防止・ログ・設定・各スレッド）。失敗したらダイアログで知らせる |
| `server_watch.rs` | speech.cpp の `/health` を 30 秒ごとに確かめる。managed のときは起動・再起動と、メニューからの停止・起動 |
| `ui.rs` | メインスレッドの見えないウィンドウとメッセージループ。フック・トレイ・タイマー・通知はここで動き、ほかのスレッドは `UiHandle` で知らせる |
| `tray.rs` | トレイのアイコンとメニュー |
| `hotkey.rs` | `WH_KEYBOARD_LL` で Ctrl+Win と Ctrl+Win+Alt を検出する（キーは横取りしない） |
| `app.rs` | 音声入力の状態（`Idle → Recording → Busy → Idle`） |
| `reader.rs` | 選択中の文の読み上げ。辞書は読み上げのたびに読み直す |
| `insert.rs` | 前面のアプリとの文字のやりとり（Ctrl+V での貼り付け、Ctrl+C での選択中の文の取得） |
| `sound.rs` | 効果音 |
| `autostart.rs` | ログオン時の起動（`HKCU\…\Run` の `mozu-tray`） |

- `install.ps1` との取り決め: 終了の合図は名前付きイベント `Local\mozu-tray-quit`、二重起動の防止は名前付きミューテックス `Local\mozu-tray`
