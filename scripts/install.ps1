# mozu-tray（常駐）と mozu（コマンド）をビルドして bin\ に置き、mozu-tray を起動し直す。
# コードを直したら、これを実行し直す。
#   pwsh scripts\install.ps1            # 確認（check.ps1）→ ビルド → 入れ替え → 起動
#   pwsh scripts\install.ps1 -SkipCheck # 確認を飛ばす
#   pwsh scripts\install.ps1 -InstallSpeech # speech.cpp がなければ、尋ねずに入れる
#   pwsh scripts\install.ps1 -SkipSpeech    # speech.cpp は入れない（尋ねない）
#Requires -Version 7
param([switch]$SkipCheck, [switch]$InstallSpeech, [switch]$SkipSpeech)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root 'bin'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

# 終了コードが 0 以外でも止まらずに、成功したかどうかを返す
function Test-Command([scriptblock]$command) {
    $PSNativeCommandUseErrorActionPreference = $false
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

Push-Location $root
try {
    if (-not $SkipCheck) { & (Join-Path $PSScriptRoot 'check.ps1') }

    Write-Host '== cargo build --release' -ForegroundColor Cyan
    cargo build --release -p mozu-tray -p mozu-cli

    Stop-Resident 'mozu-tray'

    New-Item -ItemType Directory -Force $bin | Out-Null
    foreach ($exe in 'mozu-tray.exe', 'mozu.exe') {
        Copy-Item (Join-Path $root "target\release\$exe") (Join-Path $bin $exe) -Force
    }
    if (-not (Test-Path (Join-Path $root '.env'))) {
        Copy-Item (Join-Path $root '.env.example') (Join-Path $root '.env')
        Write-Host '.env を .env.example から作りました'
    }


    # mozu コマンドをどこからでも使えるよう、bin\ をユーザーの PATH に足す
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$bin"), 'User')
        Write-Host "PATH に $bin を足しました（新しく開いたターミナルから mozu が使えます）"
    }

    # speech.cpp が動いておらず、入ってもいなければ、入れるか尋ねる
    $mozu = Join-Path $bin 'mozu.exe'
    $managed = $false
    if (-not (Test-Command { & $mozu health } )) {
        $hasSpeech = (Test-Path (Join-Path $env:APPDATA 'mozu\speech\bin\speech.exe')) -or (Get-Command speech.exe -ErrorAction SilentlyContinue)
        if ($hasSpeech) {
            Write-Host 'speech.cpp は入っていますが、応答がありません。動かし方は README の「speech.cpp の動かし方」を見てください' -ForegroundColor Yellow
        }
        elseif ($SkipSpeech) {
            Write-Host 'speech.cpp を入れていません（-SkipSpeech）' -ForegroundColor Yellow
        }
        else {
            $answer = if ($InstallSpeech) { 'y' }
            elseif ([Console]::IsInputRedirected) { 'n' }
            else { Read-Host 'speech.cpp（音声認識・合成のエンジン。本体約 30MB + モデル約 4.3GB）を入れますか？ [Y/n]' }
            if ($answer -in '', 'y', 'Y') {
                & (Join-Path $PSScriptRoot 'setup-speech.ps1') -EnableManaged
                $managed = $true
            }
            else {
                Write-Host '後で入れるときは: pwsh scripts\setup-speech.ps1 -EnableManaged'
            }
        }
    }

    Start-Process (Join-Path $bin 'mozu-tray.exe') -WorkingDirectory $root
    Write-Host "起動しました: $(Join-Path $bin 'mozu-tray.exe')" -ForegroundColor Green

    # mozu-tray が speech.cpp を起動するときは、応答するまで待ってから診断する
    if ($managed) {
        Write-Host 'speech.cpp の起動を待っています…'
        for ($i = 0; $i -lt 60 -and -not (Test-Command { & $mozu health }); $i++) { Start-Sleep 1 }
    }
    Write-Host '== mozu doctor' -ForegroundColor Cyan
    $PSNativeCommandUseErrorActionPreference = $false # 直すべき項目があっても、インストールは成功とする
    & $mozu doctor
    if (-not (Get-ItemProperty $runKey -Name 'mozu-tray' -ErrorAction SilentlyContinue)) {
        Write-Host '初回は、タスクトレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れてください'
    }
}
finally {
    Pop-Location
}
