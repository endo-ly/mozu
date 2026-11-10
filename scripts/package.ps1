# 常駐や利用者の設定に触れず、リリースに添付する exe・文書・チェックサムを作る。
#Requires -Version 7
param([switch]$SkipCheck)
$ErrorActionPreference = 'Stop'
function Write-DependencyLicenses([string]$OutDir) {
    $json = cargo metadata --locked --format-version 1 --filter-platform x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw '依存ライブラリの情報を取得できません' }
    $metadata = ($json -join "`n") | ConvertFrom-Json
    $resolved = @($metadata.resolve.nodes.id)
    $notices = [Collections.Generic.List[string]]::new()
    $notices.Add('mozu の依存ライブラリとライセンス本文')
    $notices.Add('各ライブラリの著作権・利用条件は、以下の本文に従います。')
    foreach ($package in ($metadata.packages | Where-Object { $_.source -and $_.id -in $resolved } | Sort-Object name, version)) {
        if (-not $package.license) { throw "ライセンスの確認が必要です: $($package.name)" }
        $directory = Split-Path $package.manifest_path -Parent
        $files = @(Get-ChildItem -LiteralPath $directory -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT|UNLICENSE)' })
        foreach ($subdir in @('licenses', 'LICENSES', 'license')) {
            $sub = Join-Path $directory $subdir
            if (Test-Path -LiteralPath $sub -PathType Container) { $files += @(Get-ChildItem -LiteralPath $sub -File -Recurse) }
        }
        if ($package.license_file) { $files += Get-Item -LiteralPath $package.license_file }
        $files = @($files | Sort-Object FullName -Unique)
        if (-not $files.Count) {
            $vendored = Join-Path $PSScriptRoot "../licenses/$($package.name)-$($package.version)"
            if (Test-Path -LiteralPath $vendored -PathType Container) {
                $directory = [IO.Path]::GetFullPath($vendored)
                $files = @(Get-ChildItem -LiteralPath $directory -File)
            }
        }
        if (-not $files.Count) { throw "ライセンス本文がありません。配布前に確認してください: $($package.name) $($package.version)" }
        $notices.Add("`n" + ('=' * 72) + "`n$($package.name) $($package.version)`n$($package.license)`n$($package.repository)")
        foreach ($file in $files) {
            $relative = $file.FullName.Substring($directory.TrimEnd('\', '/').Length).TrimStart('\', '/')
            $notices.Add("`n--- $relative ---`n" + (Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8))
        }
    }
    $notices | Set-Content -LiteralPath (Join-Path $OutDir 'DEPENDENCIES.txt') -Encoding utf8
}

$root = Split-Path $PSScriptRoot -Parent
Push-Location $root
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
try {
    if (-not $SkipCheck) {
        cargo fmt --all --check
        if ($LASTEXITCODE -ne 0) { throw "整形確認に失敗しました（終了コード $LASTEXITCODE）" }
        cargo clippy --locked --workspace --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw "clippyに失敗しました（終了コード $LASTEXITCODE）" }
        cargo test --locked --workspace
        if ($LASTEXITCODE -ne 0) { throw "テストに失敗しました（終了コード $LASTEXITCODE）" }
    }
    # ビルドした PC の作業場所やユーザー名を配布バイナリに残さない。
    $env:CARGO_ENCODED_RUSTFLAGS = @('-C', 'target-feature=+crt-static', "--remap-path-prefix=$root=/mozu", "--remap-path-prefix=$env:USERPROFILE=/build") -join [char]31
    cargo build --locked --release --target x86_64-pc-windows-msvc -p mozu-tray -p mozu-cli
    if ($LASTEXITCODE -ne 0) { throw "ビルドに失敗しました（終了コード $LASTEXITCODE）" }
    $json = cargo metadata --locked --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'バージョンを取得できません' }
    $metadata = ($json -join "`n") | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'mozu-cli').version
    $stage = Join-Path $root 'dist/release'
    New-Item -ItemType Directory -Force $stage | Out-Null
    $files = @('mozu.exe', 'mozu-tray.exe', 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE',
        'THIRD-PARTY-NOTICES.md', 'DEPENDENCIES.txt', 'readings.example.toml', 'install.ps1', 'setup-speech.ps1')
    foreach ($file in @('mozu.exe', 'mozu-tray.exe')) {
        Copy-Item (Join-Path $root "target/x86_64-pc-windows-msvc/release/$file") $stage -Force
    }
    foreach ($file in @('README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'THIRD-PARTY-NOTICES.md', 'readings.example.toml')) {
        Copy-Item (Join-Path $root $file) $stage -Force
    }
    foreach ($file in @('install.ps1', 'setup-speech.ps1')) {
        Copy-Item (Join-Path $PSScriptRoot $file) $stage -Force
    }
    Write-DependencyLicenses -OutDir $stage
    & (Join-Path $stage 'mozu.exe') --version
    if ($LASTEXITCODE -ne 0) { throw '配布 exe のバージョンを取得できません' }
    & (Join-Path $stage 'mozu.exe') --help | Out-Null
    if ($LASTEXITCODE -ne 0) { throw '配布 exe を実行できません' }
    $sums = foreach ($file in $files) {
        $hash = (Get-FileHash -LiteralPath (Join-Path $stage $file) -Algorithm SHA256).Hash.ToLowerInvariant()
        "$hash  $file"
    }
    $sums | Set-Content (Join-Path $stage 'SHA256SUMS') -Encoding ascii
    Write-Host "リリースの添付ファイル: $stage（$version）"
}
finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
    Pop-Location
}
