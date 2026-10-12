# mozu-dictation をビルドして bin\ に置き、起動し直す。コードを直したら、これを実行し直す。
#   pwsh scripts\install.ps1            # 確認（check.ps1）→ ビルド → 入れ替え → 起動
#   pwsh scripts\install.ps1 -SkipCheck # 確認を飛ばす
#Requires -Version 7
param([switch]$SkipCheck)
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root 'bin'
$exe = Join-Path $bin 'mozu-dictation.exe'

Push-Location $root
try {
    if (-not $SkipCheck) { & (Join-Path $PSScriptRoot 'check.ps1') }

    Write-Host '== cargo build --release' -ForegroundColor Cyan
    cargo build --release -p mozu-dictation

    # 動いているものに終了の合図を送り、終わるのを待つ（トレイのアイコンを片付けさせるため）
    $running = Get-Process mozu-dictation -ErrorAction SilentlyContinue
    if ($running) {
        Write-Host '== 動いている mozu-dictation を終了します' -ForegroundColor Cyan
        try {
            $quit = [System.Threading.EventWaitHandle]::OpenExisting('Local\mozu-dictation-quit')
            $null = $quit.Set()
            $quit.Dispose()
        }
        catch [System.Threading.WaitHandleCannotBeOpenedException] {
            # 終了の合図に対応する前の版
        }
        $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
        $left = Get-Process mozu-dictation -ErrorAction SilentlyContinue
        if ($left) {
            Write-Warning '終了しなかったので強制終了します'
            $left | Stop-Process -Force
            $left | Wait-Process -Timeout 5 -ErrorAction SilentlyContinue
        }
    }

    New-Item -ItemType Directory -Force $bin | Out-Null
    Copy-Item (Join-Path $root 'target\release\mozu-dictation.exe') $exe -Force
    if (-not (Test-Path (Join-Path $root '.env'))) {
        Copy-Item (Join-Path $root '.env.example') (Join-Path $root '.env')
        Write-Host '.env を .env.example から作りました'
    }

    Start-Process $exe -WorkingDirectory $root
    Write-Host "起動しました: $exe" -ForegroundColor Green
    Write-Host '初回は、タスクトレイのアイコンを右クリックして「ログオン時に起動」にチェックを入れてください'
}
finally {
    Pop-Location
}
