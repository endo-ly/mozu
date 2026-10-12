# mozu — 声の基盤

音声認識と音声合成を使うアプリをまとめるリポジトリ。名前は百舌（モズ。「百の舌」を持ち、ほかの鳥の声をまねる鳥）から。

## 構成

```
mozu/
├─ apps/
│  ├─ tray/              # mozu-tray.exe — 常駐（音声入力・読み上げ）。タスクトレイに 1 つだけ
│  └─ cli/               # mozu.exe — コマンド（say / transcribe / listen / voices / devices / health）
├─ crates/
│  ├─ speechcpp-client/  # speech.cpp の HTTP API（認識・合成・モデル一覧・health）
│  ├─ config/            # mozu-config — 設定・読み方の辞書・.env の読み込み（tray と cli で共有）
│  ├─ text/              # mozu-text — 読み上げ前の下ごしらえ（Markdown の除去・カタカナ化）
│  ├─ audio/             # mozu-audio — 録音・再生（cpal）、WAV の読み書き
│  ├─ speak/             # mozu-speak — 読み上げ（下ごしらえ → 丸ごと送り、speech.cpp が合成した文から順に再生）
│  └─ server/            # mozu-server — speech.cpp（speech serve）の探索・起動・停止。managed のときだけ使う
├─ scripts/
│  ├─ check.ps1          # 整形・clippy（警告はエラー）・テスト
│  ├─ install.ps1        # check → ビルド → bin\ に入れ替え →（なければ speech.cpp を導入）→ mozu-tray を起動 → mozu doctor
│  └─ setup-speech.ps1   # speech.cpp の最新版（Windows 用 Vulkan 版）を GitHub から取り、チェックサムを確かめて %APPDATA%\mozu\speech に入れる。モデルは speech pull
├─ docs/manual-test.md   # 手で確かめる項目
└─ bin/                  # インストール先（Git に入れない。ユーザーの PATH に入っている）
```

- **エンジンは speech.cpp (`https://github.com/nyosegawa/speech.cpp`)**。Windows サービス `speechcpp` が `127.0.0.1:8012` で常駐し、Qwen3-ASR 1.7B・Irodori-TTS・silero-vad を載せている。マニフェストは `C:\svc\runtimes\winsvc\manifests\speechcpp.yaml`
- **mozu は、既定ではサーバーを起動・停止しない。** `/health` が応答しないときは、エラーとして知らせるだけにする。例外は `[server] managed = true` のときだけで、このときは mozu-tray が応答のないサーバーを起動し、終了時に止める（Windows のジョブオブジェクトで、mozu-tray が落ちても一緒に止まる）。応答がある間は何もしない
- **この PC（作業場）では `managed = false` のまま。** speech.cpp は Windows サービス `speechcpp` で常時動いていて、作業場の決まりでエージェントがサーバーを起動してはいけない。動作確認で本物の speech.exe を起動するときは、サービスと別のポート・小さなモデル・CPU にする（`cargo test -p mozu-server -- --ignored`）
- アプリは speech.cpp を `speechcpp-client` 経由で直接呼ぶ。間に中継サーバー（旧 voice-gateway のようなもの）は置かない
- **常駐は `mozu-tray` 1 つにまとめる。** 新しい機能は、別の常駐アプリを作らず、ホットキーを足して tray に入れる
- 共通の処理は、2 つ目の使い道ができた時点で `crates/` に移す

## 設定（すべて tray と cli で共有）

| 場所 | 内容 |
|---|---|
| mozu の `.env` | 環境ごとの値。`MOZU_SPEECHCPP_URL`（なければ `http://127.0.0.1:8012`）。ひな形は `.env.example`、Git に入れない。実行ファイルの場所とカレントディレクトリから上へ探す |
| `%APPDATA%\mozu\config.toml` | 使い方の好み。`[speech]`（声・速さ・出力デバイス）、`[server]`（speech.cpp を mozu-tray が起動するか・exe・モデル・デバイス・声の置き場）、`[dictation]`（言語・マイク・待ち時間など）。なければ説明付きで作られる。既存のファイルは書き換えない（足りない項目は既定値） |
| `%APPDATA%\mozu\speech\` | `setup-speech.ps1` が入れた speech.cpp（`bin\`）とモデル（`models\`）。mozu 自身が入れたものは `SPEECH_MODEL_DIR` をここにして起動する |
| `%APPDATA%\mozu\voices\` | 声の置き場。`名前.wav` / `名前.voice.gguf` を `--add-voice 名前=ファイル` として渡す（managed のとき） |
| `%APPDATA%\mozu\readings.toml` | 読み方の辞書（英単語 → カタカナ）。初回に、リポジトリ直下の `readings.example.toml`（配布用の例。自分のプロジェクト名などは入れない）がコピーされ、以後は利用者が管理する（mozu を更新しても書き換えない）。語を足すときは、`mozu say --show` で「辞書にない語」を見つけ、トレイのメニュー「読み方の辞書を開く」で開いて書く |
| `%APPDATA%\mozu\logs\` | `tray.<日付>.log`（14 日分）。認識した文字列や読み上げた文は既定では残さない（`RUST_LOG=debug` で残る） |

## 開発

```powershell
pwsh scripts\check.ps1              # 整形の確認・clippy（-D warnings）・テスト
pwsh scripts\install.ps1            # check → ビルド → 動いているものを終了 → bin\ に置く → 起動
cargo test -- --ignored             # speechcpp サービス・マイク・スピーカーを使うテスト
```

- 変更を入れたら `check.ps1` を通す。lint の設定はルートの `Cargo.toml` の `[workspace.lints]`（各クレートは `[lints] workspace = true`）
- 動作に関わる変更のあとは `docs\manual-test.md` の項目を確かめる
- Rust 2024 edition。依存のバージョンはルートの `Cargo.toml` の `[workspace.dependencies]` にまとめる
- コメント・ログ・エラーメッセージは日本語

## 読み上げ（mozu-speak / mozu-text）

- **英単語はすべてカタカナにしてから合成する**（Irodori-TTS は英語の読みが崩れる）。順に、複数語の項目（"pull request"）→ 語のかたまりごと（"Node.js"）→ 記号・数字・大文字の切れ目で分けて（"PowerShell" → Power + Shell）→ 大文字だけの 6 文字以下は 1 文字ずつ（"GPU" → ジーピーユー）。それでも読めない語は英語のまま残し、ログと `mozu say --show` で「辞書にない語」として出す。辞書は利用者の `readings.toml`（配布用の例は `readings.example.toml`）
- Markdown の記号・URL・コードの塊は読まない。行の区切りは残す（speech.cpp が改行を文の切れ目として扱う）
- **文の区切り・間・種は speech.cpp に任せ、mozu では二重に持たない。** 下ごしらえした文章を丸ごと 1 回で送ると、speech.cpp が文ごとに合成し（長すぎる文は読点・空白で切る）、どの文も同じ種で読み、文と文の間を 0.9 秒以上空けてつなぐ（`tools/common/sentences.cpp`）。mozu で足すのは、speech.cpp にできないこと（Markdown の除去・カタカナ化・再生と停止）だけにする
- 音声は `response_format: "pcm"` で、合成した文から順に受け取って鳴らす。最初の文ができた時点で話し始める。止めるときは接続を切る（speech.cpp は残りの合成をやめる）
- `speed` は必ず送る（既定 1.4。speech.cpp の既定 1 は遅い）。種はサーバーが選び、`RUST_LOG=debug` でログに出る（`mozu say --seed` で同じ読み方を再現できる）

## mozu-tray（常駐）

| ホットキー | 動作 |
|---|---|
| Ctrl+Win（押している間） | 録音し、離すと文字にして前面のアプリに貼り付ける |
| Ctrl+Win+Alt | 選択中の文を読み上げる。読み上げ中にもう一度押すと止まる |

- **起動**: `bin\mozu-tray.exe`（`install.ps1` が置いて起動する）。コンソールは出さない（デバッグ版の `target\debug\` はコンソールにもログを出す）
- **自動起動**: トレイのメニュー「ログオン時に起動」。`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` の `mozu-tray` に、動いている実行ファイルのパスを登録する（`bin\` のものから登録すること）
- **終了**: トレイのメニュー「終了」。外からは名前付きイベント `Local\mozu-tray-quit` を立てる（`install.ps1` がこれを使う）。二重起動は名前付きミューテックス `Local\mozu-tray` で防ぐ
- **Windows サービスにはできない。** サービスはセッション 0 で動くため、キーボードのフック・SendInput・クリップボード・トレイが使えない

| モジュール | 役割 |
|---|---|
| `main.rs` | 起動。二重起動の防止、ログ、`.env`、設定、各スレッドの起動。起動に失敗したらダイアログで知らせる。30 秒ごとに speech.cpp の `/health` を確かめてトレイに反映する。`managed = true` のときだけ、応答がなければ `mozu-server` で起動する（起動後は応答するまで最大 120 秒待つ。終わっていたら次の確認で起動し直す） |
| `ui.rs` | メインスレッドの見えないウィンドウとメッセージループ。フック・トレイ・タイマーはここで動く。ほかのスレッドは `UiHandle` でメッセージを送って状態を知らせる |
| `tray.rs` | トレイのアイコン（鳥の絵。右下の丸が状態を表す: なし 待機 / 赤 録音 / 黄 認識 / 青 読み上げ。鳥が灰色なら speech.cpp につながらない）とメニュー |
| `autostart.rs` | ログオン時の起動の登録・解除 |
| `hotkey.rs` | `WH_KEYBOARD_LL` で Ctrl+Win（押下・解放）と Ctrl+Win+Alt を検出。キーは横取りしない。Ctrl+Win にほかのキーが混ざったら録音を取り消す（Ctrl+Win+→ など）。Alt が加わったら（どの順でも）読み上げの合図にする。押した瞬間に割り当てのないキー（0xE8）を送り、スタートメニューが開かないようにする。60 秒ごと・スリープ復帰・ロック解除でフックを入れ直し（録音中は除く）、1 秒ごとに実際のキーの状態と突き合わせて解放の取りこぼしを補う |
| `app.rs` | 音声入力の状態 `Idle → Recording → Busy → Idle`。Busy の間のホットキーは無視。0.3 秒未満の押下は捨てる。録音開始の音は 0.15 秒遅らせる（Alt が続いて読み上げになったとき鳴らさないため） |
| `reader.rs` | 読み上げ。Ctrl+C で選択中の文を受け取り（クリップボードの更新番号で「選択なし」を見分け、元のテキストに戻す）、`mozu-speak` で読む。読み方の辞書は読み上げのたびに読み直す（足した語がすぐ効く） |
| `insert.rs` | 前面のアプリとの文字のやりとり。貼り付け（修飾キーが離されるのを待ち、前面のウィンドウが録音開始時と同じなら Ctrl+V。元のテキストに戻し、履歴とクラウド同期から外す）と、選択中の文のコピー |
| `sound.rs` | 開始・終了・失敗の効果音（起動時に正弦波で作り、PlaySound で鳴らす） |

### 制限

- 管理者権限で動いているアプリには、Windows の制限（UIPI）で貼り付けも選択中の文の取得もできない
- 読み上げは Ctrl+C で文を取るので、ターミナルで何も選択していないと Ctrl+C が「中断」として届く
- 元のクリップボードは、テキストだけ戻す。画像などは上書きされる
- 録音データは保存しない（`keep_last_wav = true` のときだけ、直前の 1 回を `%APPDATA%\mozu\dictation-last.wav` に残す）

## mozu（コマンド）

```powershell
mozu say "文章"                      # 読み上げ（--voice / --speed / --seed）
mozu say -o out.wav "文章"           # WAV に保存
Get-Content memo.md | mozu say       # 標準入力から
mozu say --show "文章"               # 合成せず、読み上げる文章（カタカナにした結果）と辞書にない語を表示
mozu transcribe memo.m4a             # 文字起こし（WAV 以外は FFmpeg で変換）
mozu listen                          # マイクで聞いて、話し終わったら文字を出力
mozu voices / devices / health
mozu doctor                          # 設定・サーバー・モデル・声・マイク・スピーカー・FFmpeg を確かめ、直し方を示す（NG があれば終了コード 1）
```

- ログは標準エラーに出す（`-v` で途中経過も）。結果（文字起こし・`--show`）は標準出力
