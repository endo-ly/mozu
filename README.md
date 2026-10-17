# mozu

Windows で使う音声入力と読み上げ。認識と合成はローカルの [speech.cpp](https://github.com/nyosegawa/speech.cpp)（Qwen3-ASR / Irodori-TTS）が行い、録音は保存しない。

## mozu-tray（常駐）

| ホットキー | 動作 |
|---|---|
| **Ctrl+Win**（押している間） | 録音し、離すと文字にして、前面のアプリに貼り付ける |
| **Ctrl+Win+Alt** | 選択中の文を読み上げる。読み上げ中にもう一度押すと止まる |

鳥のアイコンの右下の丸が状態を表す（なし: 待機 / 赤: 録音 / 黄: 認識 / 青: 読み上げ）。鳥が灰色なら speech.cpp につながっていない。

## mozu（コマンド）

```powershell
mozu say "文章"                  # 読み上げ（-o out.wav で保存、--voice / --speed / --seed）
Get-Content memo.md | mozu say   # 標準入力から
mozu say --show "文章"           # 読み方（カタカナにした結果）と辞書にない語を表示
mozu transcribe memo.m4a         # 文字起こし（WAV 以外は FFmpeg で変換）
mozu listen                      # マイクで聞いて、話し終わったら文字を出力
mozu voices / devices / health
mozu doctor                      # 必要なものがそろっているか調べ、直し方を示す
```

## インストール

Windows 10 / 11 と Rust（stable）が要る。`mozu transcribe` で WAV 以外を使うなら FFmpeg も。

```powershell
pwsh scripts\install.ps1
```

ビルドして `bin\` に置き（PATH に足す）、mozu-tray を起動して `mozu doctor` を実行する。speech.cpp がなければ、入れるか尋ねる。更新するときも同じ。初回は、トレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れる。

## speech.cpp の動かし方

| 形 | 設定（`config.toml`） | 内容 |
|---|---|---|
| 自分で動かす | `[server] managed = false`（既定） | Windows サービスや手動で動かしておく |
| mozu-tray に任せる | `[server] managed = true` | 応答がなければ mozu-tray が起動し、mozu-tray が終わると一緒に止まる |

- `pwsh scripts\setup-speech.ps1 -EnableManaged` で、speech.cpp とモデル（約 4.3GB）を `%APPDATA%\mozu\speech` に入れ、`managed = true` にする（`install.ps1` が尋ねるのもこれ）
- managed のときは、トレイのメニューで speech.cpp を止められる（使っていない間に GPU のメモリ約 4.4GB を空けたいとき）
- 別の PC の speech.cpp を使うときは、mozu の `.env` に `MOZU_SPEECHCPP_URL=http://192.168.1.20:8012` のように書く（managed は使えない）

## 設定

`%APPDATA%\mozu\` にある。設定ファイルと辞書は初回に作られ、トレイのメニューから開ける。

| 場所 | 内容 |
|---|---|
| `config.toml` | 声・速さ・出力デバイス、音声入力の言語・マイク、speech.cpp の起動（`[server]`） |
| `readings.toml` | 読み方の辞書（例: `Zebra = "ゼブラ"`）。初回は [`readings.example.toml`](readings.example.toml) がコピーされる |
| `voices\` | 声の置き場（`名前.wav` か `名前.voice.gguf`）。managed のとき読み込む |
| `logs\` | ログ |

接続先は mozu の `.env`（`MOZU_SPEECHCPP_URL`、既定は `http://127.0.0.1:8012`）で決める。

## 制限

- 管理者権限で動いているアプリには、貼り付けも選択中の文の取得もできない（Windows の UIPI）
- 読み上げは Ctrl+C で文を取るので、ターミナルで何も選択していないと「中断」として届く
- クリップボードは、使ったあとテキストだけ元に戻す（画像などは上書きされる）
