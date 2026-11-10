# speech.cpp（音声認識・合成のエンジン）を mozu 用に入れる。
#   powershell -NoProfile -File scripts\setup-speech.ps1                  # 本体（Windows 用 Vulkan 版）を入れ、モデルを選ぶ
#   powershell -NoProfile -File scripts\setup-speech.ps1 -EnableManaged   # 入れたあと、設定の [server] managed を true にする
#   powershell -NoProfile -File scripts\setup-speech.ps1 -NoModels        # 本体だけ（モデルは初めて使うときに取得される）
#
# 入れる場所は %APPDATA%\mozu\speech（bin\ と models\）。ほかの場所に入っている speech.cpp には触らない。
#Requires -Version 7
param(
    [string]$Dir = (Join-Path $env:APPDATA 'mozu\speech'),
    [switch]$NoModels,
    [switch]$EnableManaged,
    [switch]$NoRestart,
    [switch]$Force,
    [string]$Version = 'v0.10.1',
    [string[]]$Models,
    [string]$Mozu
)
$ErrorActionPreference = 'Stop'

$repo = 'nyosegawa/speech.cpp'
# speech.cpp のカタログを選択肢にする。量子化形式や認識モデルは固定しない。
function Get-SpeechCatalog([string]$Exe) {
    $json = & $Exe models --json
    if ($LASTEXITCODE -ne 0) { throw 'speech models --json に失敗しました。対応する speech.cpp を導入してください' }
    $catalog = ($json -join "`n") | ConvertFrom-Json
    if (-not $catalog.models) { throw 'speech.cpp のモデル一覧が空です' }
    return $catalog
}

function Get-ModelChoices($Catalog, [string]$Task) {
    foreach ($model in $Catalog.models) {
        if ($model.task -ne $Task) { continue }
        if ($Task -eq 'synthesis' -and $model.name -notin @('irodori-tts-mf', 'irodori-tts', 'irodori-tts-large')) { continue }
        foreach ($file in $model.files) {
            $name = if ($file.type -eq $model.type) { $model.name } else { "$($model.name):$($file.type)" }
            [pscustomobject]@{
                Name = $name; Task = $Task; Type = $file.type; Size = [long]$file.size
                Languages = $model.languages -join ', '; Fetched = [bool]$file.fetched
            }
        }
    }
}

function Select-SpeechModels($Catalog, [string[]]$Requested) {
    # powershell -File 経由でもカンマ区切りを渡せるようにする。
    $Requested = @($Requested | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() })
    $tasks = [ordered]@{ recognition = '音声認識'; synthesis = '音声合成（Irodori）'; detection = '発話検出' }
    $all = @($tasks.Keys | ForEach-Object { Get-ModelChoices $Catalog $_ })
    if ($Requested.Count) {
        foreach ($name in $Requested) {
            if ($name -notin $all.Name) { throw "対応していないモデルです: $name" }
        }
        foreach ($task in $tasks.Keys) {
            if (@($all | Where-Object { $_.Task -eq $task -and $_.Name -in $Requested }).Count -gt 1) {
                throw "$($tasks[$task]) は 1 つだけ指定してください"
            }
        }
        return $Requested
    }
    if ([Console]::IsInputRedirected) { throw '対話できません。-Models に取得するモデル名を指定してください' }
    $selected = @()
    foreach ($task in $tasks.Keys) {
        $choices = @($all | Where-Object Task -eq $task)
        Write-Host "`n== $($tasks[$task])"
        Write-Host '  0: 使わない'
        for ($i = 0; $i -lt $choices.Count; $i++) {
            $item = $choices[$i]
            $size = if ($item.Size -lt 10000000) { '{0:F1} MB' -f ($item.Size / 1e6) } else { '{0:F2} GB' -f ($item.Size / 1e9) }
            $state = if ($item.Fetched) { '取得済み' } else { '未取得' }
            Write-Host "  $($i + 1): $($item.Name) [$($item.Type)] $size / $state / $($item.Languages)"
        }
        do {
            $answer = Read-Host '番号を選んでください'
            $number = -1
            $valid = [int]::TryParse($answer, [ref]$number) -and $number -ge 0 -and $number -le $choices.Count
        } until ($valid)
        if ($number -gt 0) { $selected += $choices[$number - 1].Name }
    }
    return $selected
}
$bin = Join-Path $Dir 'bin'
$exe = Join-Path $bin 'speech.exe'
$Dir = [IO.Path]::GetFullPath($Dir)
if ($Dir -eq [IO.Path]::GetPathRoot($Dir)) { throw 'ドライブ直下には導入できません' }
if (-not $Mozu) {
    $adjacent = Join-Path $PSScriptRoot '..\mozu.exe'
    $Mozu = if (Test-Path -LiteralPath $adjacent) { $adjacent } else { Join-Path $PSScriptRoot '..\bin\mozu.exe' }
}
if (-not (Test-Path -LiteralPath $Mozu)) { throw "先に mozu を導入してください。設定の保存に mozu.exe が必要です: $Mozu" }

if (-not [Environment]::Is64BitOperatingSystem) { throw '64bit の Windows だけに対応しています' }

if ((Test-Path $exe) -and -not $Force) {
    $versionText = & $exe --version
    if ($LASTEXITCODE -ne 0) { throw 'speech.cpp のバージョンを取得できません' }
    Write-Host "== speech.cpp は入っています: $($versionText | Select-Object -First 1)（入れ直すときは -Force）" -ForegroundColor Cyan
}
else {
    $releasePath = if ($Version -eq 'latest') { 'latest' } else { "tags/$Version" }
    Write-Host "== speech.cpp $Version を調べます" -ForegroundColor Cyan
    $release = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/$repo/releases/$releasePath" -Headers @{ 'User-Agent' = 'mozu-setup' }
    $zip = $release.assets | Where-Object { $_.name -like 'speech-*-windows-x64-vulkan.zip' } | Select-Object -First 1
    $sums = $release.assets | Where-Object { $_.name -eq 'SHA256SUMS' } | Select-Object -First 1
    if (-not $zip -or -not $sums) { throw "$($release.tag_name) に Windows 用の配布物が見つかりません" }

    $work = Join-Path ([IO.Path]::GetTempPath()) "mozu-speech-$([guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory $work | Out-Null
    try {
        Write-Host "== $($zip.name) をダウンロードします（$([math]::Round($zip.size / 1MB)) MB）" -ForegroundColor Cyan
        $zipPath = Join-Path $work $zip.name
        Invoke-WebRequest -UseBasicParsing $zip.browser_download_url -OutFile $zipPath
        $sumsText = (Invoke-WebRequest -UseBasicParsing $sums.browser_download_url).Content
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
        $resolvedDir = [IO.Path]::GetFullPath($Dir)
        $resolvedBin = [IO.Path]::GetFullPath($bin)
        if ($resolvedBin -ne (Join-Path $resolvedDir 'bin')) { throw '導入先の bin を確認できません' }
        if (Test-Path $bin) { Remove-Item -LiteralPath $resolvedBin -Recurse -Force }
        Move-Item $extracted $bin
        Write-Host "入れました: $exe ($($release.tag_name))" -ForegroundColor Green
    }
    finally {
        if ([IO.Path]::GetFullPath($work).StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}

Write-Host '== 使える動かす先' -ForegroundColor Cyan
try { & $exe devices; if ($LASTEXITCODE -ne 0) { throw 'デバイスの取得に失敗しました' } } catch { Write-Warning "デバイスの一覧を取れません: $_" }

$previousModelDir = $env:SPEECH_MODEL_DIR
try {
    $env:SPEECH_MODEL_DIR = [IO.Path]::GetFullPath((Join-Path $Dir 'models'))
    $catalog = Get-SpeechCatalog $exe
    $selection = @(Select-SpeechModels $catalog $Models)
    if (-not $selection.Count) { throw 'モデルを 1 つ以上選んでください' }
    Write-Host "選択: $($selection -join ', ')"
    $choices = @('recognition', 'synthesis', 'detection' | ForEach-Object { Get-ModelChoices $catalog $_ } | Where-Object { $_.Name -in $selection })
    $total = ($choices | Measure-Object Size -Sum).Sum
    $download = ($choices | Where-Object { -not $_.Fetched } | Measure-Object Size -Sum).Sum
    Write-Host ('モデルの合計: {0:F2} GB / 未取得: {1:F2} GB' -f ($total / 1e9), ($download / 1e9))
    Write-Host 'モデルの利用条件: https://github.com/nyosegawa/speech.cpp/blob/main/README.md#license'
    if (-not $NoModels) {
        & $exe pull @selection | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'モデルの取得に失敗しました。設定は更新していません' }
    }
    if (-not (Test-Path -LiteralPath $Mozu)) { throw "設定の保存に mozu.exe が必要です: $Mozu" }
    $configureArgs = @('configure-models', '--model-dir', $env:SPEECH_MODEL_DIR, '--exe', [IO.Path]::GetFullPath($exe))
    if ($EnableManaged) { $configureArgs += '--managed' }
    $recognizerNames = @($selection | ForEach-Object { ($_ -split ':')[0] })
    $languages = @($catalog.models | Where-Object { $_.task -eq 'recognition' -and $_.name -in $recognizerNames } | ForEach-Object { $_.languages })
    if ($languages.Count) { $configureArgs += @('--languages') + $languages }
    $configureArgs += @('--models') + $selection
    & $Mozu @configureArgs
    if ($LASTEXITCODE -ne 0) { throw 'モデルの設定を保存できません' }
    Write-Host 'モデルを設定へ保存しました。元の設定は config.toml.bak にあります。' -ForegroundColor Green
}
finally {
    $env:SPEECH_MODEL_DIR = $previousModelDir
}

# 設定を保存できた場合だけ反映する。インストーラー経由では、呼び出し元が最後に起動する。
if (-not $NoRestart) {
    $running = @(Get-Process mozu-tray -ErrorAction SilentlyContinue)
    $tray = if ($running.Count) { $running[0].Path } else { Join-Path (Split-Path ([IO.Path]::GetFullPath($Mozu)) -Parent) 'mozu-tray.exe' }
    if (-not $tray -or -not (Test-Path -LiteralPath $tray)) { throw '設定は保存しましたが、mozu-tray.exe が見つからないため起動できません' }
    if ($running.Count) {
        Write-Host '== 設定を反映するため mozu-tray を再起動します' -ForegroundColor Cyan
        $quit = [System.Threading.EventWaitHandle]::OpenExisting('Local\mozu-tray-quit')
        try { $null = $quit.Set() } finally { $quit.Dispose() }
        $running | Wait-Process -Timeout 15 -ErrorAction SilentlyContinue
        if (@($running | Where-Object { -not $_.HasExited }).Count) {
            throw '設定は保存しましたが、mozu-tray の終了を待ちきれませんでした。処理の終了後にトレイを起動し直してください'
        }
    }
    Start-Process -FilePath $tray -WorkingDirectory (Split-Path $tray -Parent) -WindowStyle Hidden
    Write-Host 'mozu-tray を起動しました。新しい設定で利用できます。' -ForegroundColor Green
}
