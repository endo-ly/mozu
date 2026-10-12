# mozu

ローカルの音声認識・音声合成を使うアプリ集。エンジンは [speech.cpp](https://github.com/nyosegawa/speech.cpp)（Qwen3-ASR / Irodori-TTS）。

## mozu-dictation

Windows 用の音声入力。**Ctrl+Win を押している間だけ録音**し、離すと文字にして、前面のアプリの入力欄に貼り付ける。

- 音声は `127.0.0.1` の speech.cpp にだけ送る。録音は保存しない
- タスクトレイに常駐。アイコンの色が状態を表す（白: 待機 / 赤: 録音 / 黄: 認識 / 灰: speech.cpp につながらない）
- クリップボードは貼り付け後に元に戻し、履歴やクラウド同期に残さない

### 必要なもの

- Windows 10 / 11
- Rust（stable）
- `speech serve` が動いていること（例: `speech serve qwen3-asr-1.7b --host 127.0.0.1 --port 8012`）

### インストール

```powershell
copy .env.example .env          # 接続先を変えるときは MOZU_SPEECHCPP_URL を書き換える
pwsh scripts\install.ps1        # 確認 → ビルド → bin\ に置いて起動
```

起動したら、トレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れる。更新するときも `install.ps1` を実行し直す。

### 設定

`%APPDATA%\mozu\dictation.toml`（初回起動時に説明付きで作られる。トレイの「設定ファイルを開く」でも開ける）。言語、固有名詞のヒント、マイク、効果音など。ログは `%APPDATA%\mozu\logs\`。

### 制限

- 管理者権限で動いているアプリには貼り付けられない（Windows の UIPI）
- 元のクリップボードは、テキストのみ戻す

## 構成

```
crates/speechcpp-client/   speech.cpp の HTTP API クライアント
apps/dictation/            mozu-dictation
scripts/                   check.ps1（fmt / clippy / test）、install.ps1
docs/manual-test.md        手で確かめる項目
```

開発の決まりや仕組みの詳細は [AGENTS.md](AGENTS.md)。
