# 外部のソフトウェアとモデル

mozu 自体は MIT または Apache-2.0 のいずれかを選んで利用できます。リリースとインストール先の `DEPENDENCIES.txt` には、ビルドに使った Rust 依存ライブラリの利用条件と著作権表示を収録します。

speech.cpp とモデルは mozu の配布物に同梱せず、セットアップ時に別途取得します。これらの利用条件は mozu のライセンスとは別です。[speech.cpp のライセンス案内](https://github.com/nyosegawa/speech.cpp#license)と、選んだモデルのモデルカードを確認してください。カタログの取得先は `speech models --json` でも確認できます。

Irodori-TTS の Small / Small-MF は MIT とモデルカードの制約、Large は Gemma Terms of Use とモデルカードの制約が適用されます。声の複製には本人の同意が必要です。認識モデルにもそれぞれ Apache-2.0、CC-BY-4.0、OpenMDW などの条件があります。モデルを第三者へ再配布するときは、そのモデルの条件と表示を引き継いでください。

FFmpeg は mozu に同梱しません。WAV 以外の入力を使う場合は利用者が別途導入します。
