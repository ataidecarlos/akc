# Post-build feature tests for akc.
# Run from the project root: .\test\test.ps1
# Exit code 0 = all passed, 1 = one or more failures.
#
# Mirrors test/test.sh. Both suites must be kept in step: this one is the only
# end-to-end coverage on Windows.

$ErrorActionPreference = "Stop"
Set-Location -LiteralPath (Join-Path $PSScriptRoot "..")

# Set AKC to test a specific build, e.g. a downloaded release asset:
#   $env:AKC = "C:\downloads\akc-windows-x86_64.exe"; .\test\test.ps1
$exe = $env:AKC
if ($exe) {
    if (-not (Test-Path -LiteralPath $exe)) {
        Write-Host "FAIL: AKC=$exe does not exist"
        exit 1
    }
    $null = & $exe --version 2>&1
    if ($LASTEXITCODE -ne 0) {
        Write-Host "FAIL: $exe cannot run on this host"
        exit 1
    }
} else {
    # The first candidate that exists AND runs here. Cross-built artifacts land
    # under target/ too, so presence alone is not enough to choose one.
    $candidates = @("target\release\akc.exe", "target/release/akc", "target\debug\akc.exe", "target/debug/akc")
    $exe = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Where-Object {
        $null = & $_ --version 2>&1
        $LASTEXITCODE -eq 0
    } | Select-Object -First 1
    if (-not $exe) {
        Write-Host "No build found; running cargo build --release..."
        cargo build --release
        if ($LASTEXITCODE -ne 0) { Write-Host "FAIL: build"; exit 1 }
        $exe = @("target\release\akc.exe", "target/release/akc") |
            Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
    if (-not $exe) { Write-Host "FAIL: no binary found"; exit 1 }
}
Write-Host "Using binary: $exe"

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

# Runs akc with no password flag, so the key comes from the config's key file.
# Deliberately failing invocations are expected, so $ErrorActionPreference is
# relaxed for the duration rather than aborting the run.
function Invoke-Akc {
    param([string[]]$AkcArgs)
    $ErrorActionPreference = "Continue"
    # Piping an empty string keeps an interactive `--delete` confirmation from
    # waiting on the console when the suite is run from a terminal.
    $output = "" | & $exe @AkcArgs 2>&1 | Out-String
    [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = $output.Trim() }
}

# Runs akc with an explicit literal password instead of the key file.
function Invoke-AkcPw {
    param([string[]]$AkcArgs, [string]$Password)
    $ErrorActionPreference = "Continue"
    $output = "" | & $exe @AkcArgs --password $Password 2>&1 | Out-String
    [pscustomobject]@{ ExitCode = $LASTEXITCODE; Output = $output.Trim() }
}

# Runs akc with AKC_PASSWORD set instead of --password.
function Invoke-AkcEnv {
    param([string[]]$AkcArgs, [string]$Password = "test-pass-123")
    $ErrorActionPreference = "Continue"
    $previous = $env:AKC_PASSWORD
    $env:AKC_PASSWORD = $Password
    try {
        $output = "" | & $exe @AkcArgs 2>&1 | Out-String
        $code = $LASTEXITCODE
    }
    finally {
        if ($null -eq $previous) { Remove-Item Env:\AKC_PASSWORD -ErrorAction SilentlyContinue }
        else { $env:AKC_PASSWORD = $previous }
    }
    [pscustomobject]@{ ExitCode = $code; Output = $output.Trim() }
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) "akc-test-$PID"
New-Item -ItemType Directory -Path $work -Force | Out-Null

# The config file is what lets every command below run with no path and no
# prompt, which is the whole point of the tool. Tests needing a different
# keychain or key pass --keychain / --key-file explicitly.
$cfgDir = Join-Path $work "config"
New-Item -ItemType Directory -Path $cfgDir -Force | Out-Null
$env:AKC_CONFIG_DIR = $cfgDir
$kc = Join-Path $work "secrets.akc"
$key = Join-Path $cfgDir "key"
Set-Content -LiteralPath $key -Value "test-pass-123" -NoNewline
Set-Content -LiteralPath (Join-Path $cfgDir "config") -Value @(
    "# test config"
    "keychain = $kc"
    "key_file = $key"
)

try {
    # --- init ---
    $r = Invoke-Akc @("--init")
    Assert-True "init creates a keychain" ($r.ExitCode -eq 0 -and (Test-Path -LiteralPath $kc)) $r.Output

    $size = (Get-Item -LiteralPath $kc).Length
    Assert-True "init file is small binary" ($size -gt 40 -and $size -lt 200) "size=$size"

    $r = Invoke-Akc @("--init")
    Assert-True "init refuses an existing keychain" ($r.ExitCode -ne 0 -and $r.Output -match "already exists") $r.Output
    Assert-True "init refusal points at --rotate" ($r.Output -match "--rotate") $r.Output

    $r = Invoke-AkcPw @("--init") -Password ""
    Assert-True "init rejects an empty password" ($r.ExitCode -ne 0) $r.Output

    # --- set / get ---
    $r = Invoke-Akc @("zeta", "--set", "last-value")
    Assert-True "set stores a value" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @("alpha", "--set", "first-value")
    Assert-True "set adds a second secret" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @("alpha", "--set", "updated-value")
    Assert-True "set overwrites an existing secret" ($r.ExitCode -eq 0) $r.Output

    $r = Invoke-Akc @("alpha")
    Assert-True "get returns the value" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @("missing_key")
    Assert-True "get of a missing key fails" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    $r = Invoke-Akc @()
    Assert-True "get with no name fails" ($r.ExitCode -ne 0) $r.Output

    # --- value normalization ---
    $r = Invoke-Akc @("padded", "--set", "  padded-value  ")
    Assert-True "set strips padding" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @("padded")
    Assert-True "stored value has no padding" ($r.ExitCode -eq 0 -and $r.Output -eq "padded-value") $r.Output

    $r = Invoke-Akc @("blank", "--set", "   ")
    Assert-True "blank value is rejected" ($r.ExitCode -ne 0 -and $r.Output -match "must not be empty") $r.Output
    Assert-True "empty-value error explains the rule" ($r.Output -match "whitespace is removed") $r.Output
    $r = Invoke-Akc @("blank")
    Assert-True "rejected value was not stored" ($r.ExitCode -ne 0) $r.Output

    # --- value from stdin: the agent path, and it stays out of argv ---
    $ErrorActionPreference = "Continue"
    $piped = "piped-value" | & $exe piped --set 2>&1 | Out-String
    $ErrorActionPreference = "Stop"
    $r = Invoke-Akc @("piped")
    Assert-True "set reads the value from stdin" ($r.ExitCode -eq 0 -and $r.Output -eq "piped-value") "$piped / $($r.Output)"

    # --- get several names in one open ---
    $r = Invoke-Akc @("alpha", "zeta")
    $lines = $r.Output -split "`r?`n"
    Assert-True "get accepts several names" ($r.ExitCode -eq 0 -and $lines[0] -eq "updated-value" -and $lines[1] -eq "last-value") $r.Output

    $r = Invoke-Akc @("alpha", "missing_key")
    Assert-True "one missing name aborts the whole read" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    # --- json ---
    $r = Invoke-Akc @("alpha", "--json")
    Assert-True "get --json emits an object" ($r.ExitCode -eq 0 -and $r.Output -match '"alpha"' -and $r.Output -match '"updated-value"') $r.Output

    # --- list ---
    $r = Invoke-Akc @("--list")
    $lines = $r.Output -split "`r?`n"
    Assert-True "list shows sorted keys" ($r.ExitCode -eq 0 -and $lines[0] -eq "alpha" -and $lines -contains "zeta") $r.Output
    Assert-True "list never shows values" (-not ($r.Output -match "updated-value")) $r.Output

    $r = Invoke-Akc @("--list", "alpha", "absent")
    Assert-True "list filters to the names given" ($r.ExitCode -eq 0 -and $r.Output -eq "alpha") $r.Output

    $r = Invoke-Akc @("--list", "--json")
    Assert-True "list --json emits an array" ($r.ExitCode -eq 0 -and $r.Output -match '"alpha"') $r.Output

    # --- delete, including the aliases ---
    $r = Invoke-Akc @("zeta", "--delete")
    Assert-True "delete removes a key" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @("zeta")
    Assert-True "deleted key is gone" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-Akc @("zeta", "--delete")
    Assert-True "delete of a missing key fails" ($r.ExitCode -ne 0 -and $r.Output -match "not found") $r.Output

    $r = Invoke-Akc @("piped", "--drop")
    Assert-True "--drop is an alias for --delete" ($r.ExitCode -eq 0) $r.Output
    $r = Invoke-Akc @("padded", "--remove")
    Assert-True "--remove is an alias for --delete" ($r.ExitCode -eq 0) $r.Output

    # --- command flags are mutually exclusive ---
    $r = Invoke-Akc @("--list", "--set")
    Assert-True "conflicting command flags are rejected" ($r.ExitCode -ne 0) $r.Output

    $r = Invoke-Akc @("a", "b", "--set")
    Assert-True "--set takes exactly one name" ($r.ExitCode -ne 0 -and $r.Output -match "exactly one NAME") $r.Output

    # --- wrong password ---
    $r = Invoke-AkcPw @("alpha") -Password "wrong-pass"
    Assert-True "wrong password fails" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output
    $r = Invoke-AkcPw @("--list") -Password "wrong-pass"
    Assert-True "wrong password list fails" ($r.ExitCode -ne 0) $r.Output
    $r = Invoke-AkcPw @("evil", "--set", "x") -Password "wrong-pass"
    Assert-True "wrong password set fails" ($r.ExitCode -ne 0) $r.Output

    # --- exit codes and truncated containers ---
    $short = Join-Path $work "short.akc"
    [System.IO.File]::WriteAllBytes($short, [System.Text.Encoding]::ASCII.GetBytes("AKCSTORE"))
    $r = Invoke-Akc @("--keychain", $short, "--key-file", $key, "anything")
    Assert-True "truncated container fails instead of panicking" ($r.ExitCode -eq 1 -and $r.Output -match "truncated") $r.Output

    $r = Invoke-Akc @("missing_key")
    Assert-True "missing secret exits 4" ($r.ExitCode -eq 4) $r.Output

    $r = Invoke-AkcPw @("alpha") -Password "wrong-pass"
    Assert-True "wrong password exits 3" ($r.ExitCode -eq 3) $r.Output

    $r = Invoke-Akc @("alpha", "--json", "--set", "x")
    Assert-True "--json is rejected for writes" ($r.ExitCode -eq 2 -and $r.Output -match "--json only applies") $r.Output

    # --- the config file is what makes the path-free form work ---
    $r = Invoke-Akc @("alpha")
    Assert-True "config supplies the keychain and key with no flags" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @("--keychain", $kc, "--key-file", $key, "alpha")
    Assert-True "--keychain and --key-file override the config" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    $r = Invoke-Akc @("--keychain", (Join-Path $work "absent.akc"), "--key-file", $key, "alpha")
    Assert-True "a keychain that does not exist fails" ($r.ExitCode -ne 0) $r.Output

    $r = Invoke-Akc @("--key-file", (Join-Path $work "no-such-key"), "alpha")
    Assert-True "a missing key file fails clearly" ($r.ExitCode -ne 0 -and $r.Output -match "key file not found") $r.Output

    # --- environment password ---
    $r = Invoke-AkcEnv @("alpha")
    Assert-True "AKC_PASSWORD works" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output
    $r = Invoke-AkcEnv @("alpha") -Password "wrong-pass"
    Assert-True "AKC_PASSWORD wrong value fails" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output

    # --- portability ---
    $copy = Join-Path $work "copied.akc"
    Copy-Item -LiteralPath $kc -Destination $copy
    $r = Invoke-Akc @("--keychain", $copy, "alpha")
    Assert-True "copied file still works" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    # --- tamper detection ---
    $bytes = [System.IO.File]::ReadAllBytes($copy)
    $bytes[$bytes.Length - 1] = $bytes[$bytes.Length - 1] -bxor 0xFF
    [System.IO.File]::WriteAllBytes($copy, $bytes)
    $r = Invoke-Akc @("--keychain", $copy, "alpha")
    Assert-True "tampered file rejected" ($r.ExitCode -ne 0) $r.Output

    # --- container format ---
    $raw = [System.IO.File]::ReadAllBytes($kc)
    $magic = [System.Text.Encoding]::ASCII.GetString($raw[0..7])
    Assert-True "keychain starts with the AKCSTORE magic" ($magic -eq "AKCSTORE") "magic=$magic"
    $version = [System.BitConverter]::ToUInt32($raw, 8)
    Assert-True "keychain records container version 2" ($version -eq 2) "version=$version"

    # --- rotate ---
    $before = (Get-ChildItem -LiteralPath $work -Filter "*.bak").Count

    # --password supplies the CURRENT key, --new-password the replacement. A
    # wrong current key cannot decrypt, so nothing may be written or backed up.
    $r = Invoke-AkcPw @("--rotate", "--new-password", "irrelevant") -Password "wrong-pass"
    Assert-True "rotate with a wrong current key fails" ($r.ExitCode -ne 0 -and $r.Output -match "wrong password") $r.Output

    $r = Invoke-Akc @("alpha")
    Assert-True "a failed rotate leaves the vault readable" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output
    Assert-True "a failed rotate creates no backup" ((Get-ChildItem -LiteralPath $work -Filter "*.bak").Count -eq $before) "before=$before"

    $r = Invoke-Akc @("--rotate", "--new-password", "rotated-456")
    Assert-True "rotate succeeds" ($r.ExitCode -eq 0) $r.Output

    $r = Invoke-Akc @("alpha")
    Assert-True "the old key no longer opens the vault" ($r.ExitCode -ne 0) $r.Output

    $r = Invoke-AkcPw @("alpha") -Password "rotated-456"
    Assert-True "secrets survive a rotation" ($r.ExitCode -eq 0 -and $r.Output -eq "updated-value") $r.Output

    Assert-True "rotate created a backup" ((Get-ChildItem -LiteralPath $work -Filter "*.bak").Count -gt $before) "before=$before"

    # --- failed ops leave file untouched ---
    $sizeBefore = (Get-Item -LiteralPath $kc).Length
    $null = Invoke-AkcPw @("missing_key") -Password "rotated-456"
    $null = Invoke-AkcPw @("missing_key", "--delete") -Password "rotated-456"
    $null = Invoke-AkcPw @("blank", "--set", "") -Password "rotated-456"
    $null = Invoke-AkcPw @("blank", "--set", "   ") -Password "rotated-456"
    $sizeAfter = (Get-Item -LiteralPath $kc).Length
    Assert-True "failed ops leave file untouched" ($sizeBefore -eq $sizeAfter) "$sizeBefore -> $sizeAfter"

    # --- concurrent writers ---
    # Regression: the vault is one blob that every write rewrites wholesale, so
    # two writers that both read before either saved would clobber each other.
    # Before the write lock, ten concurrent `set` calls left a single key.
    $race = Join-Path $work "race.akc"
    $null = Invoke-Akc @("--keychain", $race, "--init")

    $jobs = 1..10 | ForEach-Object {
        $i = $_
        Start-Job -ScriptBlock {
            param($exePath, $vault, $n)
            $env:AKC_PASSWORD = "test-pass-123"
            & $exePath --keychain $vault "race$n" --set "value$n" 2>&1 | Out-Null
        } -ArgumentList $exe, $race, $i
    }
    $null = $jobs | Wait-Job -Timeout 120
    $jobs | Remove-Job -Force

    $r = Invoke-Akc @("--keychain", $race, "--list")
    $raceKeys = ($r.Output -split "`r?`n" | Where-Object { $_ -like "race*" }).Count
    Assert-True "concurrent writers lose no secrets" ($raceKeys -eq 10) "stored $raceKeys of 10"

    Assert-True "a write lock is kept beside the vault" (Test-Path -LiteralPath "$race.lock") "no $race.lock"
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "Results: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 }
exit 0
