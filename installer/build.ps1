# IME Indicator のインストーラをワンショットでビルドする PowerShell スクリプト。
#
# 使い方:
#   pwsh .\installer\build.ps1
#
# 流れ:
#   1. cargo build --release でリリース exe を作る
#   2. tools/gen_icon.py が生成した assets/icon.ico があるか確認
#   3. ISCC.exe を per-user / system の標準パスから探して installer/setup.iss をコンパイル

$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")

Write-Host "[1/3] cargo build --release" -ForegroundColor Cyan
Push-Location $Root
try {
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
}

$IcoPath = Join-Path $Root "assets/icon.ico"
if (-not (Test-Path $IcoPath)) {
    Write-Host "[2/3] generating icon (assets/icon.ico is missing)" -ForegroundColor Cyan
    Push-Location $Root
    try {
        python tools/gen_icon.py
        if ($LASTEXITCODE -ne 0) { throw "icon generation failed" }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "[2/3] icon already present at $IcoPath" -ForegroundColor DarkGray
}

$IsccCandidates = @(
    (Join-Path $env:LOCALAPPDATA "Programs/Inno Setup 6/ISCC.exe"),
    "C:/Program Files (x86)/Inno Setup 6/ISCC.exe",
    "C:/Program Files/Inno Setup 6/ISCC.exe"
)
$Iscc = $IsccCandidates | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $Iscc) {
    throw "ISCC.exe not found. Install Inno Setup 6 first: winget install JRSoftware.InnoSetup"
}

Write-Host "[3/3] compiling installer with $Iscc" -ForegroundColor Cyan
& $Iscc (Join-Path $PSScriptRoot "setup.iss")
if ($LASTEXITCODE -ne 0) { throw "ISCC compile failed" }

$Output = Join-Path $PSScriptRoot "out/IMEIndicator-Setup.exe"
if (Test-Path $Output) {
    $sz = [math]::Round((Get-Item $Output).Length / 1MB, 2)
    Write-Host ""
    Write-Host "✓ installer built: $Output ($sz MB)" -ForegroundColor Green
} else {
    throw "expected output not found at $Output"
}
