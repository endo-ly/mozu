# GitHub Releases の exe を取得・検証して Programs\mozu に入れる。
#   powershell -NoProfile -File scripts\install.ps1             # 最新リリースを導入・更新
#   powershell -NoProfile -File scripts\install.ps1 -FromSource # ソースを検証・ビルドして bin\ に導入
#   powershell -NoProfile -File scripts\install.ps1 -SkipCheck # 確認を飛ばす
#   powershell -NoProfile -File scripts\install.ps1 -InstallSpeech # speech.cpp がなければ、尋ねずに入れる
#   powershell -NoProfile -File scripts\install.ps1 -SkipSpeech    # speech.cpp は入れない（尋ねない）
#   powershell -NoProfile -File scripts\install.ps1 -Uninstall     # アプリ・PATH・自動起動を削除。設定・辞書・モデルは残す
#Requires -Version 7
param([switch]$SkipCheck, [switch]$InstallSpeech, [switch]$SkipSpeech, [switch]$NoStart,
    [string]$Dir, [switch]$NoPath, [switch]$Uninstall, [switch]$FromSource,
    [string]$Version = 'latest', [string]$ReleaseDir)
$ErrorActionPreference = 'Stop'

$root = if ($PSScriptRoot) {
    if ((Split-Path $PSScriptRoot -Leaf) -eq 'scripts') { Split-Path $PSScriptRoot -Parent } else { $PSScriptRoot }
} else { $null }
$prebuilt = -not $FromSource
if ($FromSource -and ($ReleaseDir -or -not $root -or -not (Test-Path (Join-Path $root 'Cargo.toml')))) {
    throw '-FromSource は mozu のリポジトリで実行してください（-ReleaseDir と併用できません）'
}
if (-not $Dir) {
    $Dir = if ($prebuilt -and $root -and (Test-Path (Join-Path $root 'mozu.exe'))) { $root }
        elseif ($prebuilt) { Join-Path $env:LOCALAPPDATA 'Programs\mozu' }
        else { Join-Path $root 'bin' }
}
$bin = [IO.Path]::GetFullPath($Dir)
if ($bin -eq [IO.Path]::GetPathRoot($bin)) { throw 'ドライブ直下は指定できません' }
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$mozuDownloadState = [pscustomobject]@{ Path = $null }
$assets = [ordered]@{
    'mozu.exe' = 'mozu.exe'; 'mozu-tray.exe' = 'mozu-tray.exe'
    'README.md' = 'README.md'; 'LICENSE-MIT' = 'LICENSE-MIT'; 'LICENSE-APACHE' = 'LICENSE-APACHE'
    'THIRD-PARTY-NOTICES.md' = 'THIRD-PARTY-NOTICES.md'; 'DEPENDENCIES.txt' = 'DEPENDENCIES.txt'
    'readings.example.toml' = 'readings.example.toml'
    'install.ps1' = 'scripts/install.ps1'; 'setup-speech.ps1' = 'scripts/setup-speech.ps1'
}

# 全ファイルを検証するまで、動いているアプリや導入先には触れない。
function Get-ReleaseFiles {
    if ($ReleaseDir) { $source = [IO.Path]::GetFullPath($ReleaseDir) }
    else {
        if ($Version -ne 'latest' -and $Version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$') {
            throw '-Version は latest または v0.1.0 のようなタグにしてください'
        }
        $path = if ($Version -eq 'latest') { 'latest' } else { "tags/$Version" }
        $release = Invoke-RestMethod -UseBasicParsing "https://api.github.com/repos/endo-ly/mozu/releases/$path"
        if ($release.draft -or ($Version -ne 'latest' -and $release.tag_name -ne $Version)) { throw '公開リリースを確認できません' }
        $source = Join-Path ([IO.Path]::GetTempPath()) "mozu-release-$([guid]::NewGuid().ToString('N'))"
        $mozuDownloadState.Path = $source
        New-Item -ItemType Directory $source | Out-Null
        foreach ($name in @('SHA256SUMS') + @($assets.Keys)) {
            $releaseAssets = @($release.assets | Where-Object name -CEQ $name)
            if ($releaseAssets.Count -ne 1) { throw "リリースに $name がありません" }
            $uri = [Uri]$releaseAssets[0].browser_download_url
            if ($uri.Scheme -ne 'https' -or $uri.Host -ne 'github.com') { throw '取得先が GitHub の HTTPS URL ではありません' }
            Invoke-WebRequest -UseBasicParsing $uri.AbsoluteUri -OutFile (Join-Path $source $name)
        }
        Write-Host "取得しました: $($release.tag_name)"
    }
    $checksums = @{}
    foreach ($line in Get-Content -LiteralPath (Join-Path $source 'SHA256SUMS')) {
        if ($line -notmatch '^([0-9a-fA-F]{64})  ([A-Za-z0-9_.-]+)$' -or $checksums.ContainsKey($Matches[2])) {
            throw 'SHA256SUMS の書式が不正です'
        }
        $checksums[$Matches[2]] = $Matches[1]
    }
    foreach ($name in $assets.Keys) {
        $actual = (Get-FileHash -LiteralPath (Join-Path $source $name) -Algorithm SHA256).Hash
        if (-not $checksums.ContainsKey($name) -or $actual -ne $checksums[$name]) { throw "チェックサムが一致しません: $name" }
    }
    return $source
}

# 終了コードが 0 以外でも止まらずに、成功したかどうかを返す
function Test-Command([scriptblock]$command) {
    $ErrorActionPreference = 'Continue'
    & $command *> $null
    return $LASTEXITCODE -eq 0
}

# 動いているものに終了の合図を送り、終わるのを待つ（トレイのアイコンを片付けさせるため）
function Stop-Resident([string]$name) {
    $running = Get-Process $name -ErrorAction SilentlyContinue
    if (-not $running) { return }
    Write-Host "== 動いている $name を終了します" -ForegroundColor Cyan
    try {
        $quit = [System.Threading.EventWaitHandle]::OpenExisting("Local\$name-quit")
        $null = $quit.Set()
        $quit.Dispose()
    }
    catch [System.Threading.WaitHandleCannotBeOpenedException] {
        # 合図を受け取れないときは、下で強制終了する
    }
    $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    $left = Get-Process $name -ErrorAction SilentlyContinue
    if ($left) {
        Write-Warning "$name が終了しなかったので強制終了します"
        $left | Stop-Process -Force
        $left | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
    }
}

Push-Location $(if ($FromSource) { $root } else { Get-Location })
try {
    if ($Uninstall) {
        if (-not (Test-Path -LiteralPath (Join-Path $bin 'mozu.exe'))) { throw 'mozu の導入フォルダを確認できません' }
        Stop-Resident 'mozu-tray'
        $entry = Get-ItemProperty -LiteralPath $runKey -Name mozu-tray -ErrorAction SilentlyContinue
        if ($entry -and $entry.'mozu-tray' -eq ('"' + (Join-Path $bin 'mozu-tray.exe') + '"')) {
            Remove-ItemProperty -LiteralPath $runKey -Name mozu-tray
        }
        if (-not $NoPath) {
            $userPath = [string][Environment]::GetEnvironmentVariable('Path', 'User')
            [Environment]::SetEnvironmentVariable('Path', (($userPath -split ';' | Where-Object { $_.TrimEnd('\') -ne $bin.TrimEnd('\') }) -join ';'), 'User')
        }
        foreach ($exe in 'mozu.exe', 'mozu-tray.exe') {
            Remove-Item -LiteralPath (Join-Path $bin $exe) -ErrorAction SilentlyContinue
        }
        Write-Host "アプリを削除しました。残った配布文書は $bin から手動で削除できます。設定・辞書・モデル・ログは残しています。"
        return
    }
    if (-not $prebuilt) {
        if (-not $SkipCheck) {
            cargo fmt --all --check
            if ($LASTEXITCODE -ne 0) { throw "整形確認に失敗しました（終了コード $LASTEXITCODE）" }
            cargo clippy --locked --workspace --all-targets -- -D warnings
            if ($LASTEXITCODE -ne 0) { throw "clippyに失敗しました（終了コード $LASTEXITCODE）" }
            cargo test --locked --workspace
            if ($LASTEXITCODE -ne 0) { throw "テストに失敗しました（終了コード $LASTEXITCODE）" }
        }
        Write-Host '== cargo build --release' -ForegroundColor Cyan
        cargo build --locked --release -p mozu-tray -p mozu-cli
        if ($LASTEXITCODE -ne 0) { throw "ビルドに失敗しました（終了コード $LASTEXITCODE）" }
    } else {
        $releaseSource = Get-ReleaseFiles
    }

    Stop-Resident 'mozu-tray'

    New-Item -ItemType Directory -Force $bin | Out-Null
    if ($prebuilt) {
        foreach ($name in $assets.Keys) {
            $destination = Join-Path $bin $assets[$name]
            New-Item -ItemType Directory -Force (Split-Path $destination -Parent) | Out-Null
            Copy-Item -LiteralPath (Join-Path $releaseSource $name) -Destination $destination -Force
        }
    } else {
        foreach ($exe in 'mozu-tray.exe', 'mozu.exe') {
            Copy-Item (Join-Path $root "target\release\$exe") (Join-Path $bin $exe) -Force
        }
    }
    # mozu コマンドをどこからでも使えるよう、bin\ をユーザーの PATH に足す
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not $NoPath -and ($userPath -split ';') -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', (([string]$userPath).TrimEnd(';') + ";$bin").TrimStart(';'), 'User')
        Write-Host "PATH に $bin を足しました（新しく開いたターミナルから mozu が使えます）"
    }
    if (Get-ItemProperty -LiteralPath $runKey -Name mozu-tray -ErrorAction SilentlyContinue) {
        Set-ItemProperty -LiteralPath $runKey -Name mozu-tray -Value ('"' + (Join-Path $bin 'mozu-tray.exe') + '"')
    }

    # speech.cpp が動いておらず、入ってもいなければ、入れるか尋ねる
    $mozu = Join-Path $bin 'mozu.exe'
    $managed = $false
    if (-not $SkipSpeech -and -not (Test-Command { & $mozu health } )) {
        $hasSpeech = (Test-Path (Join-Path $env:APPDATA 'mozu\speech\bin\speech.exe')) -or (Get-Command speech.exe -ErrorAction SilentlyContinue)
        if ($hasSpeech) {
            Write-Host 'speech.cpp は入っていますが、応答がありません。動かし方は README の「speech.cpp の動かし方」を見てください' -ForegroundColor Yellow
        }
        else {
            $answer = if ($InstallSpeech) { 'y' }
            elseif ([Console]::IsInputRedirected) { 'n' }
            else { Read-Host 'speech.cpp（音声認識・合成のエンジン）を入れ、モデルを選びますか？ [Y/n]' }
            if ($answer -in '', 'y', 'Y') {
                $setup = if ($prebuilt) { Join-Path $bin 'scripts/setup-speech.ps1' } else { Join-Path $PSScriptRoot 'setup-speech.ps1' }
                & $setup -EnableManaged -NoRestart -Mozu $mozu
                $managed = $true
            }
            else {
                Write-Host '後で入れるときは: powershell -NoProfile -File scripts\setup-speech.ps1 -EnableManaged'
            }
        }
    }
    Write-Host "導入しました: $bin"

    if ($NoStart) { return }
    Start-Process (Join-Path $bin 'mozu-tray.exe') -WorkingDirectory $bin -WindowStyle Hidden
    Write-Host "起動しました: $(Join-Path $bin 'mozu-tray.exe')" -ForegroundColor Green

    # mozu-tray が speech.cpp を起動するときは、応答するまで待ってから診断する
    if ($managed) {
        Write-Host 'speech.cpp の起動を待っています…'
        for ($i = 0; $i -lt 60 -and -not (Test-Command { & $mozu health }); $i++) { Start-Sleep 1 }
    }
    Write-Host '== mozu doctor' -ForegroundColor Cyan
    # 診断の非ゼロ終了は表示するが、導入失敗にはしない。
    & { $ErrorActionPreference = 'Continue'; & $mozu doctor }
    if (-not (Get-ItemProperty $runKey -Name 'mozu-tray' -ErrorAction SilentlyContinue)) {
        Write-Host '初回は、タスクトレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れてください'
    }
}
finally {
    if ($mozuDownloadState.Path) {
        $downloadPath = [IO.Path]::GetFullPath($mozuDownloadState.Path)
        $tempParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\', '/')
        if ((Split-Path $downloadPath -Parent) -eq $tempParent -and (Split-Path $downloadPath -Leaf) -match '^mozu-release-[0-9a-f]{32}$') {
            foreach ($name in @('SHA256SUMS') + @($assets.Keys)) {
                $file = Join-Path $downloadPath $name
                if (Test-Path -LiteralPath $file -PathType Leaf) { Remove-Item -LiteralPath $file }
            }
            if ((Test-Path -LiteralPath $downloadPath) -and -not @(Get-ChildItem -LiteralPath $downloadPath -Force).Count) {
                Remove-Item -LiteralPath $downloadPath
            }
        }
    }
    Pop-Location
}
