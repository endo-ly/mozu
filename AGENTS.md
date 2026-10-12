# mozu — 声の基盤

音声認識と音声合成を使うアプリをまとめるリポジトリ。名前は百舌（モズ。「百の舌」を持ち、ほかの鳥の声をまねる鳥）から。

## 構成

```
mozu/
├─ scripts/
│  ├─ check.ps1          # 整形・clippy（警告はエラー）・テスト
│  └─ install.ps1        # check → ビルド → bin\ に入れ替え → 起動
├─ docs/
│  └─ manual-test.md     # 手で確かめる項目
├─ bin/                  # インストール先（Git に入れない）
├─ crates/
│  └─ speechcpp-client/   # speech.cpp の HTTP API（認識・合成・health）の同期クライアント
└─ apps/
   └─ dictation/          # 音声入力（mozu-dictation.exe）
```

- **接続先は `.env`** の `MOZU_SPEECHCPP_URL`（ひな形は `.env.example`。`.env` は Git に入れない）。なければ `http://127.0.0.1:8012`。アプリは実行ファイルの場所とカレントディレクトリから上へ `.env` を探す
- **エンジンは speech.cpp (`https://github.com/nyosegawa/speech.cpp`)**。Windows サービス `speechcpp` が `127.0.0.1:8012` で常駐し、Qwen3-ASR 1.7B・Irodori-TTS・silero-vad を載せている。マニフェストは `C:\svc\runtimes\winsvc\manifests\speechcpp.yaml`
- **mozu はサーバーを起動・停止しない。** `/health` が応答しないときは、エラーとして知らせるだけにする
- アプリは speech.cpp を `speechcpp-client` 経由で直接呼ぶ。間に中継サーバー（旧 voice-gateway のようなもの）は置かない
- 共通の処理（録音、キー入力など）は、まずアプリの中に書く。2つ目のアプリで必要になった時点で `crates/` に移す

## 開発

```powershell
pwsh scripts\check.ps1              # 整形の確認・clippy（-D warnings）・テスト
pwsh scripts\install.ps1            # check → ビルド → 動いているものを終了 → bin\ に置く → 起動
cargo test -- --ignored             # speechcpp サービスとマイクを使うテスト
```

- 変更を入れたら `check.ps1` を通す。lint の設定はルートの `Cargo.toml` の `[workspace.lints]`（各クレートは `[lints] workspace = true`）
- 動作に関わる変更のあとは `docs\manual-test.md` の項目を確かめる
- Rust 2024 edition。依存のバージョンはルートの `Cargo.toml` の `[workspace.dependencies]` にまとめる
- コメント・ログ・エラーメッセージは日本語

## dictation（音声入力）

Ctrl+Win を押している間だけ録音し、離すと文字にして、前面のアプリの入力欄に貼り付ける。

- **起動**: `bin\mozu-dictation.exe`（`install.ps1` が置いて起動する）。タスクトレイに常駐し、コンソールは出さない（デバッグ版の `target\debug\` はコンソールにもログを出す）
- **自動起動**: トレイのメニュー「ログオン時に起動」。`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の `mozu-dictation` に、動いている実行ファイルのパスを登録する（`bin\` のものから登録すること）
- **終了**: トレイのメニュー「終了」。外からは名前付きイベント `Local\mozu-dictation-quit` を立てる（`install.ps1` がこれを使う）。二重起動は名前付きミューテックス `Local\mozu-dictation` で防ぐ
- **Windows サービスにはできない。** サービスはセッション 0 で動くため、キーボードのフック・SendInput・クリップボード・トレイが使えない
- **設定**: `%APPDATA%\mozu\dictation.toml`（初回起動時に、説明付きで作られる）。言語・マイク・待ち時間など使い方の好みを書く。接続先は `.env`
- **ログ**: `%APPDATA%\mozu\logs\dictation.<日付>.log`（14日分）。認識した文字列は既定では残さない（`RUST_LOG=debug` で残る）
- **入力デバイスの一覧**: 起動時にログに出る。コマンドなら `bin\mozu-dictation.exe --list-devices | Out-String`（GUI アプリなので、パイプしないと出力を待たない）

### 流れ

| モジュール | 役割 |
|---|---|
| `main.rs` | 起動。二重起動の防止、ログ、`.env`、設定、各スレッドの起動。起動に失敗したらダイアログで知らせる。30秒ごとに speech.cpp の `/health` を確かめてトレイに反映する（サービスは起動しない） |
| `ui.rs` | メインスレッドの見えないウィンドウとメッセージループ。フック・トレイ・タイマーはここで動く。ほかのスレッドは `UiHandle` でメッセージを送って状態を知らせる |
| `tray.rs` | トレイのアイコン（白: 待機 / 赤: 録音 / 黄: 認識 / 灰: speech.cpp につながらない）とメニュー |
| `autostart.rs` | ログオン時の起動の登録・解除 |
| `hotkey.rs` | `WH_KEYBOARD_LL` で Ctrl+Win の押下・解放を検出。キーは横取りしない。ほかのキーが混ざったら取り消す（Ctrl+Win+→ などの Windows のショートカット）。押した瞬間に割り当てのないキー（0xE8）を送り、Win を離したときにスタートメニューが開かないようにする。Windows が知らせずにフックを外すことがあるので、60秒ごと・スリープ復帰・ロック解除で入れ直す（録音中は除く）。1秒ごとに実際のキーの状態と突き合わせ、解放を取りこぼしても録音が止まらなくならないようにする |
| `app.rs` | 状態 `Idle → Recording → Busy → Idle`。Busy の間のホットキーは無視する。0.3秒未満の押下は捨てる。状態が変わるたびにトレイに知らせる |
| `recorder.rs` | cpal（WASAPI）で、録音中だけ入力ストリームを開く。モノラルにまとめ、デバイスのサンプリング周波数のまま 16bit WAV にする（16kHz への変換は speech.cpp が行う） |
| `insert.rs` | 修飾キーが離されるのを待ち、前面のウィンドウが録音開始時と同じなら、クリップボード経由で Ctrl+V を送り、元のテキストに戻す。貼り付けた文字はクリップボードの履歴とクラウド同期から外す。ウィンドウが変わっていたら貼らず、結果をクリップボードに残す |
| `sound.rs` | 開始・終了・失敗の効果音（起動時に正弦波で作り、PlaySound で鳴らす） |

### 制限

- 管理者権限で動いているアプリには、Windows の制限（UIPI）で貼り付けられない
- 元のクリップボードは、テキストだけ戻す。画像などは認識結果で上書きされる
- 録音データは保存しない（`keep_last_wav = true` のときだけ、直前の1回を `%APPDATA%\mozu\dictation-last.wav` に残す）
