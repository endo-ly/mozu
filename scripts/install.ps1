# mozu-tray（常駐）と mozu（コマンド）をビルドして bin\ に置き、mozu-tray を起動し直す。
# コードを直したら、これを実行し直す。
#   pwsh scripts\install.ps1            # 確認（check.ps1）→ ビルド → 入れ替え → 起動
#   pwsh scripts\install.ps1 -SkipCheck # 確認を飛ばす
#Requires -Version 7
param([switch]$SkipCheck)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root 'bin'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'

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
        # 終了の合図に対応する前の版
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
    Stop-Resident 'mozu-dictation' # 改名前の名前

    New-Item -ItemType Directory -Force $bin | Out-Null
    foreach ($exe in 'mozu-tray.exe', 'mozu.exe') {
        Copy-Item (Join-Path $root "target\release\$exe") (Join-Path $bin $exe) -Force
    }
    if (-not (Test-Path (Join-Path $root '.env'))) {
        Copy-Item (Join-Path $root '.env.example') (Join-Path $root '.env')
        Write-Host '.env を .env.example から作りました'
    }

    # 改名前（mozu-dictation）の片付け。自動起動の登録は新しい名前で引き継ぐ
    Remove-Item (Join-Path $bin 'mozu-dictation.exe') -ErrorAction SilentlyContinue
    if (Get-ItemProperty $runKey -Name 'mozu-dictation' -ErrorAction SilentlyContinue) {
        Remove-ItemProperty $runKey -Name 'mozu-dictation'
        Set-ItemProperty $runKey -Name 'mozu-tray' -Value ('"{0}"' -f (Join-Path $bin 'mozu-tray.exe'))
        Write-Host 'ログオン時の起動を mozu-tray に引き継ぎました'
    }
    $oldConfig = Join-Path $env:APPDATA 'mozu\dictation.toml'
    if (Test-Path $oldConfig) {
        Move-Item $oldConfig "$oldConfig.old" -Force
        Write-Host "設定は %APPDATA%\mozu\config.toml に変わりました（古い設定は dictation.toml.old）"
    }

    # mozu コマンドをどこからでも使えるよう、bin\ をユーザーの PATH に足す
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $bin) {
        [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ";$bin"), 'User')
        Write-Host "PATH に $bin を足しました（新しく開いたターミナルから mozu が使えます）"
    }

    Start-Process (Join-Path $bin 'mozu-tray.exe') -WorkingDirectory $root
    Write-Host "起動しました: $(Join-Path $bin 'mozu-tray.exe')" -ForegroundColor Green
    if (-not (Get-ItemProperty $runKey -Name 'mozu-tray' -ErrorAction SilentlyContinue)) {
        Write-Host '初回は、タスクトレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れてください'
    }
}
finally {
    Pop-Location
}
