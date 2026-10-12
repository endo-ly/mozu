# mozu

ローカルの音声認識・音声合成を使う道具集。エンジンは [speech.cpp](https://github.com/nyosegawa/speech.cpp)（Qwen3-ASR / Irodori-TTS）。音声は `127.0.0.1` の speech.cpp にだけ送り、録音は保存しない。

## mozu-tray（常駐）

Windows のタスクトレイに常駐する。

| ホットキー | 動作 |
|---|---|
| **Ctrl+Win**（押している間） | 録音し、離すと文字にして、前面のアプリの入力欄に貼り付ける |
| **Ctrl+Win+Alt** | 選択中の文を読み上げる。読み上げ中にもう一度押すと止まる |

- アイコンの色が状態を表す（白: 待機 / 赤: 録音 / 黄: 認識 / 青: 読み上げ / 灰: speech.cpp につながらない）
- 英単語はカタカナにしてから読む（読み方の辞書は追加できる）。Markdown の記号や URL は読まない
- クリップボードは使ったあと元に戻し、履歴やクラウド同期に残さない

## mozu（コマンド）

```powershell
mozu say "読み上げる文章"            # -o out.wav で保存、--voice / --speed で声と速さ
Get-Content memo.md | mozu say       # 標準入力から
mozu say --show "文章"               # 読み方（カタカナにした結果）と辞書にない語を確かめる
mozu transcribe memo.m4a             # 音声ファイルを文字に
mozu listen                          # マイクで聞いて、話し終わったら文字を出力
mozu voices / devices / health
mozu doctor                          # 必要なものがそろっているか調べ、足りないものの直し方を示す
```

## 必要なもの

- Windows 10 / 11、Rust（stable）
- speech.cpp（`install.ps1` が、なければ入れるか尋ねる。→ [speech.cpp](#speechcpp)）
- `mozu transcribe` で WAV 以外を使うときは FFmpeg

## インストール

```powershell
copy .env.example .env          # 接続先を変えるときは MOZU_SPEECHCPP_URL を書き換える
pwsh scripts\install.ps1        # 確認 → ビルド → bin\ に置く（PATH に追加）→ mozu-tray を起動 → 診断
```

起動したら、トレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れる。更新するときも `install.ps1` を実行し直す。

## speech.cpp

認識と合成は speech.cpp（`speech serve`）がやる。mozu-tray は、次のどちらの形でも使える。

| 形 | 設定 | 内容 |
|---|---|---|
| **mozu-tray に任せる** | `[server] managed = true` | 応答がなければ mozu-tray が `speech serve` を起動し、終了するとき（異常終了でも）一緒に止める。すでに応答があれば起動しない |
| 自分で動かす | `managed = false`（既定） | Windows サービスや手動で動かしておく。つながらないときはアイコンが灰色になるだけ |

- `pwsh scripts\setup-speech.ps1 -EnableManaged` で、speech.cpp（Windows 用 Vulkan 版、約 30MB）とモデル（約 4.3GB）を `%APPDATA%\mozu\speech` に入れ、`managed = true` にする。`install.ps1` が尋ねて実行するのもこれ
- 動かしている間、GPU のメモリを約 4.4GB 使う。ほかの GPU の作業と同時に動かすときは注意
- 声は `%APPDATA%\mozu\voices\` に参照音声（`名前.wav`）か声のファイル（`名前.voice.gguf`）を置くと、`managed` のとき一緒に読み込む（既定の声は `none`）

## 設定

`%APPDATA%\mozu\` にある（初回に説明付きで作られる。トレイのメニューからも開ける）。

- `config.toml` — 声・速さ・出力デバイス、音声入力の言語・マイクなど
- `readings.toml` — 読み方の辞書（例: `Zebra = "ゼブラ"`）
- `logs\` — ログ（`speech.log` は mozu-tray が起動した speech.cpp の出力）
- `speech\` — `setup-speech.ps1` が入れた speech.cpp とモデル
- `voices\` — 声の置き場

## 制限

- 管理者権限で動いているアプリには、貼り付けも選択中の文の取得もできない（Windows の UIPI）
- 元のクリップボードは、テキストのみ戻す

## 構成

```
apps/tray/                 mozu-tray（常駐）
apps/cli/                  mozu（コマンド）
crates/speechcpp-client/   speech.cpp の HTTP API クライアント
crates/config/             設定・読み方の辞書
crates/text/               読み上げ前の下ごしらえ（Markdown の除去・カタカナ化・文の区切り）
crates/audio/              録音・再生
crates/speak/              読み上げ（合成と再生を並行）
crates/server/             speech.cpp の起動・管理（managed のとき）
scripts/                   check.ps1（fmt / clippy / test）、install.ps1、setup-speech.ps1
docs/manual-test.md        手で確かめる項目
```

開発の決まりや仕組みの詳細は [AGENTS.md](AGENTS.md)。
