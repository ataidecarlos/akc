# Build the Windows release binaries for every supported architecture.
#
# Run from the project root:
#   .\scripts\build-release.ps1 [-OutputDir target\release-dist]
#
# Produces akc-windows-x86_64.exe and akc-windows-aarch64.exe, matching the asset
# names that `akc upgrade` looks for.
#
# Requires the MSVC build tools and both Rust targets:
#   rustup target add x86_64-pc-windows-msvc aarch64-pc-windows-msvc
#
# Linux binaries are built separately on Linux/WSL; see scripts/build-release.sh.

[CmdletBinding()]
param(
    [string]$OutputDir = "target\release-dist"
)

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath (Join-Path $PSScriptRoot "..")

$targets = [ordered]@{
    "x86_64" = "x86_64-pc-windows-msvc"
    "aarch64" = "aarch64-pc-windows-msvc"
}

$installed = rustup target list --installed
foreach ($rustTarget in $targets.Values) {
    if ($installed -notcontains $rustTarget) {
        throw "Rust target '$rustTarget' is not installed. Run: rustup target add $rustTarget"
    }
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null

foreach ($arch in $targets.Keys) {
    $rustTarget = $targets[$arch]
    Write-Host "building $rustTarget"
    cargo build --release --target $rustTarget
    if ($LASTEXITCODE -ne 0) { throw "build failed for $rustTarget" }

    $built = "target\$rustTarget\release\akc.exe"
    if (-not (Test-Path -LiteralPath $built)) { throw "expected $built" }

    $dest = Join-Path $OutputDir "akc-windows-$arch.exe"
    Copy-Item -LiteralPath $built -Destination $dest -Force
    Write-Host ("  -> {0} ({1} bytes)" -f $dest, (Get-Item -LiteralPath $dest).Length)
}

Write-Host ""
Write-Host "Windows binaries in ${OutputDir}:"
Get-ChildItem -LiteralPath $OutputDir | Format-Table Name, Length -AutoSize
