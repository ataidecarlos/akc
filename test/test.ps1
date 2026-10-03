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

function Invoke-AkcEnv {
    param([string[]]$AkcArgs, [string]$Password = "test-pass-123")
    $ErrorActionPreference = "Continue"
    $previous = $env:AKC_PASSWORD
    $env:AKC_PASSWORD = $Password
    try {
        $output = & $exe @AkcArgs 2>&1 | Out-String
        $code = $LASTEXITCODE
    }
    finally {
        if ($null -eq $previous) { Remove-Item Env:\AKC_PASSWORD -ErrorAction SilentlyContinue }
        else { $env:AKC_PASSWORD = $previous }
    }
    [pscustomobject]@{ ExitCode = $code; Output = $output.Trim() }
}

# Drive interactive mode from a script, with a hard timeout. A command that
# blocks on a console password prompt must fail the run instead of hanging it.
function Invoke-AkcInteractive {
    param(
        [string[]]$AkcArgs,
        [string]$Script,
        [string]$Password = "test-pass-123",
        [int]$TimeoutSec = 45
    )
    $job = Start-Job -ScriptBlock {
        param($exePath, $akcArgs, $stdinText, $pw)
        $env:AKC_PASSWORD = $pw
        $out = $stdinText | & $exePath @akcArgs 2>&1 | Out-String
        [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = $out.Trim() }
    } -ArgumentList $exe, $AkcArgs, $Script, $Password

    if (Wait-Job $job -Timeout $TimeoutSec) {
        $result = Receive-Job $job
        Remove-Job $job -Force
        $result
    }
    else {
        Stop-Job $job
        Remove-Job $job -Force
        [pscustomobject]@{ ExitCode = -1; Output = "TIMED OUT after ${TimeoutSec}s (interactive mode hung)" }
    }
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
    Assert-True "init refuses an existing keychain" ($r.ExitCode -ne 0 -and $r.Output -match "already exists") $r.Output
    Assert-True "init refusal points at change-password" ($r.Output -match "change-password") $r.Output

    $r = Invoke-Akc @($kc, "init", "--force")
    Assert-True "init --force replaces and backs up" ($r.ExitCode -eq 0 -and $r.Output -match "Backed up") $r.Output

    $r = Invoke-Akc @($kc, "init", "x") -Password ""
    Assert-True "init rejects empty password" ($r.ExitCode -ne 0) $r.Output

    # --- set ---
    $r = Invoke-Akc @($kc, "set", "zeta", "last-value")
    Assert-True "set adds secret" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "alpha", "first-value")
    Assert-True "set adds second secret" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "alpha", "updated-value")
    Assert-True "set updates existing secret" ($r.ExitCode -eq 0) $r.Output

    # --- value normalization ---
    $r = Invoke-Akc @($kc, "set", "padded", "  padded-value  ")
    Assert-True "set strips padding from values" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @($kc, "get", "padded")
    Assert-True "stored value has no padding" ($r.ExitCode -eq 0 -and $r.Output -eq "padded-value") $r.Output

    $r = Invoke-Akc @($kc, "set", "blank", "")
    Assert-True "set rejects an empty value" ($r.ExitCode -ne 0 -and $r.Output -match "must not be empty") $r.Output
    $r = Invoke-Akc @($kc, "set", "blank", "   ")
    Assert-True "set rejects a whitespace-only value" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-Akc @($kc, "get", "blank")
    Assert-True "rejected value was not stored" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-Akc @($kc, "set", "blank", "   ")
    Assert-True "empty-value error explains the rule" ($r.Output -match "whitespace is removed") $r.Output

    # --- get ---
    $r = Invoke-Akc @($kc, "get", "alpha")
    Assert-True "get returns value" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @($kc, "get", "missing")
    Assert-True "get missing key fails" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    # --- list ---
    $r = Invoke-Akc @($kc, "list")
    $lines = $r.Output -split "`r?`n"
    Assert-True "list shows sorted keys" ($r.ExitCode -eq 0 -and $lines[0] -eq "alpha" -and $lines[1] -eq "padded" -and $lines[2] -eq "zeta" -and $lines.Count -eq 3) $r.Output

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

    # --- container format ---
    $raw = [System.IO.File]::ReadAllBytes($kc)
    $magic = [System.Text.Encoding]::ASCII.GetString($raw[0..7])
    Assert-True "keychain starts with the AKCSTORE magic" ($magic -eq "AKCSTORE") "magic=$magic"
    $version = [System.BitConverter]::ToUInt32($raw, 8)
    Assert-True "keychain records container version 2" ($version -eq 2) "version=$version"

    # --- environment password ---
    $r = Invoke-AkcEnv @($kc, "get", "alpha")
    Assert-True "AKC_PASSWORD environment variable works" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-AkcEnv @($kc, "get", "alpha") -Password "wrong-pass"
    Assert-True "AKC_PASSWORD wrong value fails" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output

    # --- interactive mode ---
    $r = Invoke-AkcInteractive -AkcArgs @($kc) -Script "set from-interactive interactive-value`nget from-interactive`nlist`nexit`n"
    Assert-True "interactive mode runs multiple commands" ($r.ExitCode -eq 0 -and $r.Output -match "interactive-value") $r.Output
    Assert-True "interactive mode does not hang" ($r.ExitCode -ne -1) $r.Output

    $r = Invoke-AkcEnv @($kc, "get", "from-interactive")
    Assert-True "interactive set is persisted" ($r.ExitCode -eq 0 -and $r.Output -eq "interactive-value") $r.Output

    $r = Invoke-AkcInteractive -AkcArgs @($kc) -Script "set ipad   padded-interactive  `nset iblank    `nexit`n"
    Assert-True "interactive set strips padding" ($r.ExitCode -eq 0) $r.Output
    Assert-True "interactive blank value is rejected with an explanation" ($r.Output -match "whitespace is removed") $r.Output
    $r = Invoke-AkcEnv @($kc, "get", "ipad")
    Assert-True "interactive stored value has no padding" ($r.ExitCode -eq 0 -and $r.Output -eq "padded-interactive") $r.Output
    $r = Invoke-AkcEnv @($kc, "get", "iblank")
    Assert-True "interactive rejects a blank value" ($r.ExitCode -ne 0) $r.Output

    $r = Invoke-AkcInteractive -AkcArgs @($kc) -Script "init`nexit`n"
    Assert-True "interactive init is no longer a command" ($r.ExitCode -eq 0 -and $r.Output -match "unknown command: init") $r.Output

    # --- change-password ---
    $before = (Get-ChildItem -LiteralPath $work -Filter "*.bak").Count

    $r = Invoke-Akc @($kc, "change-password", "--new-password", "newpass-456") -Password "wrong-pass"
    Assert-True "change-password rejects a wrong current password" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output
    $r = Invoke-Akc @($kc, "get", "alpha")
    Assert-True "failed change-password leaves the vault readable" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output
    Assert-True "failed change-password creates no backup" ((Get-ChildItem -LiteralPath $work -Filter "*.bak").Count -eq $before) "before=$before"

    $r = Invoke-Akc @($kc, "change-password", "--new-password", "newpass-456")
    Assert-True "change-password succeeds" ($r.ExitCode -eq 0 -and $r.Output -match "Changed the password") $r.Output

    $r = Invoke-Akc @($kc, "get", "alpha") -Password "newpass-456"
    Assert-True "secrets survive a password change" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @($kc, "list")
    Assert-True "old password no longer works" ($r.ExitCode -ne 0) $r.Output

    Assert-True "change-password created a backup" ((Get-ChildItem -LiteralPath $work -Filter "*.bak").Count -eq $before + 1) "before=$before"

    $r = Invoke-Akc @($kc, "change-password", "--new-password", "another-789") -Password "newpass-456"
    Assert-True "password can be changed again" ($r.ExitCode -eq 0) $r.Output

    # --- failed ops leave file untouched ---
    $r = Invoke-Akc @($kc, "get", "alpha") -Password "newpass-456"
    $sizeBefore = (Get-Item -LiteralPath $kc).Length
    $null = Invoke-Akc @($kc, "get", "missing") -Password "newpass-456"
    $null = Invoke-Akc @($kc, "delete", "missing") -Password "newpass-456"
    $null = Invoke-Akc @($kc, "set", "blank", "") -Password "newpass-456"
    $sizeAfter = (Get-Item -LiteralPath $kc).Length
    Assert-True "failed ops leave file untouched" ($sizeBefore -eq $sizeAfter) "$sizeBefore -> $sizeAfter"
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Results: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 }
exit 0