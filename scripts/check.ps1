# 整形・lint・テストをまとめて確かめる。install.ps1 も最初にこれを通す。
#   pwsh scripts\check.ps1
#Requires -Version 7
$ErrorActionPreference = 'Stop'
$PSNativeCommandUseErrorActionPreference = $true

Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    Write-Host '== cargo fmt --check' -ForegroundColor Cyan
    cargo fmt --all --check
    Write-Host '== cargo clippy' -ForegroundColor Cyan
    cargo clippy --workspace --all-targets -- -D warnings
    Write-Host '== cargo test' -ForegroundColor Cyan
    cargo test --workspace
    Write-Host 'OK' -ForegroundColor Green
}
finally {
    Pop-Location
}
