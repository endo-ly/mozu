# mozu

mozu は、Windows で音声入力と読み上げを日常的に使うための常駐アプリです。キーボードから手を離さずに文章を入力し、画面の文を耳で確かめられます。

文章を書いている途中で `Ctrl+Win` を押したまま話すと、話した内容がその場の入力欄に文字として入ります。読みたい文を選択して `Ctrl+Win+Alt` を押すと、選んだ部分を声で読み上げます。チャット、エディタ、ブラウザなど、文字を扱うアプリを切り替えずに使えます。

音声認識と音声合成は  [speech.cpp](https://github.com/nyosegawa/speech.cpp)  が担当します。音声は PC の外に出ず、録音も保存しません。読み上げ用の辞書を育てれば、英単語や専門用語の読みを自分好みにできます。常駐する mozu-tray がホットキーとトレイを受け持ち、mozu コマンドはファイルの読み上げや文字起こし、調子の確認に使います。

既定では、音声と文字は次のように手元の speech.cpp を通ります。

```text
音声入力  マイク → mozu-tray → speech.cpp → 前面のアプリ
読み上げ  選択した文 → mozu-tray → speech.cpp → スピーカー
コマンド  mozu（読み上げ・文字起こし） → speech.cpp
```

## インストール

Windows 10 / 11 の x64 版と PowerShell 7 が必要です。PowerShell 7 で、次の一行を実行します。

```powershell
irm https://github.com/endo-ly/mozu/releases/latest/download/install.ps1 | iex
```

最新リリースの exe と必要なファイルを取得・検証し、`%LOCALAPPDATA%\Programs\mozu` に入れます。speech.cpp がなければ、そのまま導入とモデル選択に進めます。最後に PATH に追加して mozu-tray を起動するので、新しく開いたターミナルから `mozu` が使えます。

### speech.cpp とモデル

インストール中に speech.cpp の導入を選ぶと、使うモデルを対話で選べます。Windows サービスなどですでに動いている場合は、それを使います。既定の接続先は `http://127.0.0.1:8012` です。 `mozu config` で変更できます。

後から導入したりモデルを選び直したりする場合は、次を実行します。導入先を変えた場合は、そのフォルダの `scripts\setup-speech.ps1` を使ってください。

```powershell
pwsh "$env:LOCALAPPDATA\Programs\mozu\scripts\setup-speech.ps1" -EnableManaged
```

`%APPDATA%\mozu\speech` に speech.cpp を入れ、使うモデルを対話で選びます。モデルは音声認識・音声合成・発話検出からそれぞれ選び、使わないものは「0: 使わない」にできます。画面には容量・言語・取得済みかどうかが出るので、用途に合わせて選んでください。`-EnableManaged` を付けると mozu-tray が speech.cpp の起動と停止を受け持ち、付けない場合は自分で動かしておく形になります。

セットアップは既定で v0.10.1 の speech.cpp を入れます。別のバージョンを使うときは `-Version`、入れ直すときは `-Force` を付けます。モデルを選ぶと設定が更新され、元の設定は `config.toml.bak` に残ります。

セットアップが終わると mozu-tray が自動で起動・再起動し、設定が反映されます。`-EnableManaged` を付けた場合は speech.cpp も起動します。モデルの利用条件は mozu のライセンスとは別なので、[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) を確認してください。

### 最初の確認

トレイの鳥のアイコンを右クリックし、「ログオン時に起動」を必要に応じて有効にします。`mozu doctor` を実行すると、speech.cpp、モデル、マイク、スピーカー、FFmpeg の状態を確認できます。トレイの状態が「speech.cpp 接続中」になってから実行すると確実です。

WAV 以外の音声を文字起こしする場合だけ、FFmpeg を PATH に入れてください（`winget install Gyan.FFmpeg`）。WAV の文字起こしと読み上げには要りません。

## 使い方

mozu-tray が動いている間は、どのアプリでも同じホットキーが使えます。

| ホットキー | 動作 |
|---|---|
| `Ctrl+Win` を押している間 | 話した内容を録音し、離すと入力欄に文字を入れる |
| `Ctrl+Win+Alt` | 選択した文を読み上げる。読み上げ中にもう一度押すと止まる |

### 音声入力

文字を入れたい場所にカーソルを置き、`Ctrl+Win` を押したまま話してからキーを離します。録音中はトレイの鳥に赤い丸が付き、認識中は黄色に変わります。認識が終わると、話し始めたときの入力欄に文字が入ります。

管理者として動いているアプリには、Windows の仕組み上、文字を送ることも選択中の文を取ることもできません。

### 読み上げ

読みたい文を選択して `Ctrl+Win+Alt` を押すと、選択範囲をコピーしてから読み上げが始まります。読み上げ中にもう一度押すと止まり、長い文章でも最初の文ができたところから順に鳴ります。選択がないまま押すと、トレイからエラー音が鳴ります。

### トレイ

鳥のアイコンの右下の丸で、今の状態が分かります。タスクトレイに隠れている場合は、「^」を開くと見つかります。

| 表示 | 状態 |
|---|---|
| 丸なし | 待機中 |
| 赤 | 録音中 |
| 黄 | 認識中 |
| 青 | 読み上げ中 |
| 灰色の鳥（待機中） | speech.cpp に接続していない（停止中や未使用の停止を含む） |

右クリックのメニューから、次の操作ができます。

| メニュー | 内容 |
|---|---|
| 状態 | 今の状態と speech.cpp の接続 |
| speech.cpp を止める／起動する | 起動を任せている場合だけ |
| ログオン時に起動 | Windows へのログオン時に mozu-tray を起動する |
| 設定ファイルを開く | `config.toml` をメモ帳で開く |
| 読み方の辞書を開く | `readings.toml` をメモ帳で開く |
| ログのフォルダを開く | `%APPDATA%\mozu\logs` を開く |
| 終了 | mozu-tray を終了する（mozu-tray が起動した speech.cpp も止まる） |

終了するとホットキーも止まるので、次に使うときは導入先の `mozu-tray.exe` を起動します。

## 読み上げの文章

読み上げる前に、原文を声にしやすい形へ整えます。Markdown の見出しや箇条書きの記号、URL、コードブロックは読みません。行の区切りは残すので、段落の切れ目には間が入ります。

英単語は、Irodori-TTS が英語の読みを苦手とするため、`readings.toml` の辞書を使ってカタカナに直します。辞書にない語は英語のまま読み、`mozu say --show` で「辞書にない語」として確認できます。GPU のような短い大文字の語は、辞書になくても 1 文字ずつ読みます。

辞書は `readings.toml` に `英単語 = "カタカナ"` の形で書きます。大文字と小文字は区別せず、複数語の見出しはダブルクォートで囲みます。

```toml
zebra = "ゼブラ"
"pull request" = "プルリクエスト"
```

初回は [readings.example.toml](readings.example.toml) の内容が `%APPDATA%\mozu\readings.toml` にコピーされます。トレイの「読み方の辞書を開く」から開けます。変更は次に読み上げるときから反映され、コマンドは次に実行したときから新しい辞書を使います。

## コマンド

mozu コマンドは、ファイルやマイクをまとめて扱いたいときに使います。読み上げ、文字起こし、マイクからの文字起こし、声やデバイスの確認ができます。トレイと同じ設定と speech.cpp を使うため、起動を mozu-tray に任せている場合は、トレイも動かしておきます。

| コマンド | 用途 |
|---|---|
| `mozu say "文章"` | 文章を読み上げる |
| `mozu say -o out.wav "文章"` | 音声を WAV に保存する |
| `Get-Content memo.md \| mozu say` | ファイルを読み上げる |
| `mozu say --show "文章"` | 読み方と辞書にない語を表示する |
| `mozu transcribe 音声ファイル` | 音声を文字にする |
| `mozu listen` | マイクで聞いて文字を出力する |
| `mozu voices` | 使える声を確認する |
| `mozu devices` | 入出力デバイスを確認する |
| `mozu health` | speech.cpp に接続できるか確認する |
| `mozu doctor` | 必要なものと設定を診断する |
| `mozu config` | 設定ファイルを開く |

`mozu say` は文章を引数に渡すほか、標準入力からも読めます。声や速さを変えたいときは `--voice` / `--speed` を使います。`mozu transcribe` は WAV 以外のファイルを FFmpeg で変換してから文字にします。`mozu listen` はマイクに向かって話し、無音が続いたところで録音を止めます。文字起こしの結果は標準出力に出るので、そのままファイルに保存できます。

```powershell
mozu say "こんにちは。"
Get-Content memo.md | mozu say
mozu say --show "GitHub の Pull Request"
mozu transcribe 録音.m4a > 文字起こし.txt
```

## 設定とデータ

設定とデータは `%APPDATA%\mozu` にあります。`config.toml` と `readings.toml` は初回に作られ、`mozu config` またはトレイのメニューから開けます。設定ファイルには声、速さ、出力デバイス、認識言語、マイク、効果音などの項目があり、各項目の説明はファイル内のコメントに書いてあります。変更したあとは mozu-tray を起動し直すと反映され、コマンドは次に実行したときから新しい設定を使います。

| パス | 内容 |
|---|---|
| `config.toml` | 声、速さ、入出力デバイス、認識言語、speech.cpp の接続など |
| `readings.toml` | 読み方の辞書 |
| `voices\` | 声のファイル。mozu-tray に起動を任せている場合に読み込む |
| `speech\` | セットアップで入れた speech.cpp とモデル |
| `logs\` | mozu-tray と speech.cpp のログ |
| `dictation-last.wav` | 直前の録音を残す設定にしたときだけできるファイル |

声を変えるときは、`%APPDATA%\mozu\voices\`に声のファイルを置き、設定ファイルの声の名前を変えます。名前はファイル名から `.wav` か `.voice.gguf` を除いたものです。`mozu voices` で使える声を確認できます。声のファイルの作り方は [speech.cpp](https://github.com/nyosegawa/speech.cpp) の案内を見てください。

認識した文字や読み上げた文は、既定ではログに残しません。

## speech.cpp の起動

| 動かし方 | 動き |
|---|---|
| 自分で動かす（既定） | Windows サービスなどで先に起動しておき、mozu はそこへ接続する |
| mozu-tray に任せる | 利用時に応答がなければ mozu-tray が起動し、mozu-tray を終えると一緒に止める |

既定では mozu が speech.cpp を起動することはなく、Windows サービスなどで先に動かしておいたものに接続します。応答がないときは、トレイの鳥が灰色になり、コマンドはエラーを返します。

mozu-tray に任せた場合は、起動した speech.cpp を 30 分使わないと止め、次に音声入力や読み上げ、コマンドで使ったときに再開します。待ち時間は設定ファイルで変えられ、0 にすると止まりません。トレイのメニューから手動で止めた場合は、使っても自動では起動せず、「speech.cpp を起動する」で再開します。

## 更新と削除

削除は、導入先の `scripts\install.ps1` に `-Uninstall` を付けて実行します。アプリ、PATH、自動起動の登録が消え、設定・辞書・モデル・ログは残ります。すべて消す場合は `%APPDATA%\mozu` を削除してください。削除後に残った文書は、導入先のフォルダから手で削除できます。導入先を変えた場合は、導入時と同じ `-Dir` を指定します。

## ライセンス

mozu は [MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のどちらかを選んで使えます。配布物には、Rust の依存ライブラリの利用条件も含まれています。

speech.cpp とモデルは別に取得するもので、利用条件は mozu とは異なります。詳しくは [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) と、選んだモデルのモデルカードを確認してください。声を複製して使うときは、本人の同意を得てください。
