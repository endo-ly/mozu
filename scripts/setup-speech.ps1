# speech.cpp（音声認識・合成のエンジン）を mozu 用に入れる。
#   pwsh scripts\setup-speech.ps1                  # 本体（Windows 用 Vulkan 版）とモデル（約 4.3GB）を入れる
#   pwsh scripts\setup-speech.ps1 -EnableManaged   # 入れたあと、設定の [server] managed を true にする
#   pwsh scripts\setup-speech.ps1 -NoModels        # 本体だけ（モデルは初めて使うときに取得される）
#
# 入れる場所は %APPDATA%\mozu\speech（bin\ と models\）。ほかの場所に入っている speech.cpp には触らない。
#Requires -Version 7
param(
    [string]$Dir = (Join-Path $env:APPDATA 'mozu\speech'),
    [switch]$NoModels,
    [switch]$EnableManaged,
    [switch]$Force
)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$repo = 'nyosegawa/speech.cpp'
$models = 'irodori-tts-mf', 'qwen3-asr-1.7b', 'silero-vad'
$bin = Join-Path $Dir 'bin'
$exe = Join-Path $bin 'speech.exe'

if (-not [Environment]::Is64BitOperatingSystem) { throw '64bit の Windows だけに対応しています' }

if ((Test-Path $exe) -and -not $Force) {
    Write-Host "== speech.cpp は入っています: $(& $exe --version | Select-Object -First 1)（入れ直すときは -Force）" -ForegroundColor Cyan
}
else {
    Write-Host '== 最新版を調べます' -ForegroundColor Cyan
    $release = Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest" -Headers @{ 'User-Agent' = 'mozu-setup' }
    $zip = $release.assets | Where-Object { $_.name -like 'speech-*-windows-x64-vulkan.zip' } | Select-Object -First 1
    $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
    if (-not $zip -or -not $sums) { throw "$($release.tag_name) に Windows 用の配布物が見つかりません" }

    $work = Join-Path ([IO.Path]::GetTempPath()) "mozu-speech-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory $work | Out-Null
    try {
        Write-Host "== $($zip.name) をダウンロードします（$([math]::Round($zip.size / 1MB)) MB）" -ForegroundColor Cyan
        $zipPath = Join-Path $work $zip.name
        Invoke-WebRequest $zip.browser_download_url -OutFile $zipPath
        $sumsText = (Invoke-WebRequest $sums.browser_download_url).Content
        if ($sumsText -is [byte[]]) { $sumsText = [Text.Encoding]::UTF8.GetString($sumsText) }

        # チェックサムの行は「ハッシュ *ファイル名」
        $expected = ($sumsText -split "`n" | Where-Object { $_ -match [regex]::Escape($zip.name) } | Select-Object -First 1) -split '\s+' | Select-Object -First 1
        $actual = (Get-FileHash $zipPath -Algorithm SHA256).Hash
        if (-not $expected -or $actual -ne $expected.ToUpper()) {
            throw "チェックサムが合いません（期待: $expected / 実際: $actual）。ダウンロードをやり直してください"
        }
        Write-Host 'チェックサムを確認しました'

        $extracted = Join-Path $work 'out'
        Expand-Archive $zipPath $extracted
        if (-not (Test-Path (Join-Path $extracted 'speech.exe'))) { throw 'zip に speech.exe がありません' }

        New-Item -ItemType Directory -Force $Dir | Out-Null
        if (Test-Path $bin) { Remove-Item $bin -Recurse -Force }
        Move-Item $extracted $bin
        Write-Host "入れました: $exe ($($release.tag_name))" -ForegroundColor Green
    }
    finally {
        Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    }
}

Write-Host '== 使える動かす先' -ForegroundColor Cyan
try { & $exe devices } catch { Write-Warning "デバイスの一覧を取れません: $_" }

if (-not $NoModels) {
    $env:SPEECH_MODEL_DIR = Join-Path $Dir 'models'
    Write-Host "== モデルを取得します（初回は約 4.3GB。途中で止まっても、実行し直せば続きから）: $($models -join ', ')" -ForegroundColor Cyan
    & $exe pull @models | Out-Null
    Write-Host 'モデルがそろいました' -ForegroundColor Green
}

if ($EnableManaged) {
    $config = Join-Path $env:APPDATA 'mozu\config.toml'
    if (Test-Path $config) {
        $text = Get-Content $config -Raw
        $changed = [regex]::Replace($text, '(?m)^managed\s*=\s*false', 'managed = true')
        if ($changed -ne $text) {
            Set-Content $config $changed -NoNewline
            Write-Host '設定の [server] managed を true にしました（mozu-tray が speech.cpp を起動します）'
        }
    }
    else {
        Write-Warning "$config がまだありません。mozu-tray を一度起動してから、[server] managed = true にしてください"
    }
}
else {
    Write-Host '次は、%APPDATA%\mozu\config.toml の [server] managed を true にして mozu-tray を起動し直してください'
}
