# Post-build feature tests for akc.
# Run from the project root: .\test\test.ps1
# Exit code 0 = all passed, 1 = one or more failures.

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath (Join-Path $PSScriptRoot "..")

$exe = @("target\release\akc.exe", "target/release/akc", "target\debug\akc.exe", "target/debug/akc") |
    Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $exe) {
    Write-Host "No build found; running cargo build --release..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: build"; exit 1 }
    $exe = @("target\release\akc.exe", "target/release/akc") |
        Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
}

$script:pass = 0
$script:fail = 0

function Assert-True {
    param([string]$Name, [bool]$Condition, [string]$Detail = "")
    if ($Condition) {
        $script:pass++
        Write-Host "PASS  $Name"
    } else {
        $script:fail++
        Write-Host "FAIL  $Name  $Detail"
    }
}

function Invoke-Akc {
    param([string[]]$AkcArgs, [string]$Password = "test-pass-123")
    $ErrorActionPreference = "Continue"
    $output = & $exe @AkcArgs --password $Password 2>&1 | Out-String
    [pscustomobject]@{
        ExitCode = $LASTEXITCODE
        Output   = $output.Trim()
    }
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) "akc-test-$PID"
New-Item -ItemType Directory -Path $work | Out-Null

try {
    $kc = Join-Path $work "secrets.akc"

    # --- init ---
    $r = Invoke-Akc @($kc, "init")
    Assert-True "init creates file" ($r.ExitCode -eq 0 -and (Test-Path -LiteralPath $kc)) $r.Output

    $size = (Get-Item -LiteralPath $kc).Length
    Assert-True "init file is small binary" ($size -gt 40 -and $size -lt 200) "size=$size"

    $r = Invoke-Akc @($kc, "init")
    Assert-True "init backs up existing file" ($r.ExitCode -eq 0 -and $r.Output -match "Backed up") $r.Output

    $r = Invoke-Akc @($kc, "init", "x") -Password ""
    Assert-True "init rejects empty password" ($r.ExitCode -ne 0) $r.Output

    # --- set ---
    $r = Invoke-Akc @($kc, "set", "zeta", "last-value")
    Assert-True "set adds secret" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "alpha", "first-value")
    Assert-True "set adds second secret" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "alpha", "updated-value")
    Assert-True "set updates existing secret" ($r.ExitCode -eq 0) $r.Output

    # --- get ---
    $r = Invoke-Akc @($kc, "get", "alpha")
    Assert-True "get returns value" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @($kc, "get", "missing")
    Assert-True "get missing key fails" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    # --- list ---
    $r = Invoke-Akc @($kc, "list")
    $lines = $r.Output -split "`r?`n"
    Assert-True "list shows sorted keys" ($r.ExitCode -eq 0 -and $lines[0] -eq "alpha" -and $lines[1] -eq "zeta" -and $lines.Count -eq 2) $r.Output

    # --- delete ---
    $r = Invoke-Akc @($kc, "delete", "zeta")
    Assert-True "delete removes key" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "get", "zeta")
    Assert-True "deleted key is gone" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-Akc @($kc, "delete", "zeta")
    Assert-True "delete missing key fails" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    # --- wrong password ---
    $r = Invoke-Akc @($kc, "get", "alpha") -Password "wrong-pass"
    Assert-True "wrong password fails" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output
    $r = Invoke-Akc @($kc, "list") -Password "wrong-pass"
    Assert-True "wrong password list fails" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "evil", "x") -Password "wrong-pass"
    Assert-True "wrong password set fails" ($r.ExitCode -ne 0) $r.Output

    # --- portability ---
    $copy = Join-Path $work "copied.akc"
    Copy-Item -LiteralPath $kc -Destination $copy
    $r = Invoke-Akc @($copy, "get", "alpha")
    Assert-True "copied file still works" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    # --- tamper detection ---
    $bytes = [System.IO.File]::ReadAllBytes($copy)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xFF
    [System.IO.File]::WriteAllBytes($copy, $bytes)
    $r = Invoke-Akc @($copy, "get", "alpha")
    Assert-True "tampered file rejected" ($r.ExitCode -ne 0) $r.Output

    # --- persistence across runs is implied; verify file unchanged after failed ops ---
    $before = (Get-Item -LiteralPath $kc).Length
    $null = Invoke-Akc @($kc, "get", "missing")
    $null = Invoke-Akc @($kc, "delete", "missing")
    $after = (Get-Item -LiteralPath $kc).Length
    Assert-True "failed ops leave file untouched" ($before -eq $after) "$before -> $after"
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Results: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 }
exit 0
