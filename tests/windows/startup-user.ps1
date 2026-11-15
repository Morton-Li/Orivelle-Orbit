param(
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][string]$OutputDirectory,
    [string]$ExpectedClientVersion
)
$ErrorActionPreference = 'Stop'
if ($env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm') {
    throw 'Run this installed-program check only on an explicitly disposable VM.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'This acceptance must run without administrator elevation.'
}
if (Get-Process orivelle-orbit -ErrorAction SilentlyContinue) {
    throw 'Finish other Orbit acceptance processes before this check.'
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$receipt = [ordered]@{
    scope = 'Installed release executable; ordinary interactive user; no fixture or debugger'
    elevated = $false
    executable_sha256 = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
}
if ($ExpectedClientVersion) {
    $receipt.scope = 'Ordinary interactive user; transient loopback inspector for native backend readiness; no HTTPS fixture'
}
$app = $null
try {
    $startupJson = & powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -STA -File "$PSScriptRoot\lifecycle.ps1" -OrbitProcess 0 -Action startup -StartExecutable $Executable -Screenshot "$OutputDirectory\startup-user.png"
    if ($LASTEXITCODE -ne 0) { throw 'Startup observation failed.' }
    $receipt.startup = $startupJson | ConvertFrom-Json
    $app = Get-Process -Id $receipt.startup.process_id
    if (-not $receipt.startup.main_seen -or $receipt.startup.maximum_consecutive_unresponsive -ge 3 -or $receipt.startup.visible_console) {
        throw 'The ordinary-user startup did not stay responsive without a console.'
    }
    $stateJson = & powershell.exe -NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -STA -File "$PSScriptRoot\lifecycle.ps1" -OrbitProcess $app.Id -Action state
    if ($LASTEXITCODE -ne 0) { throw 'Native window observation failed.' }
    $receipt.window = $stateJson | ConvertFrom-Json
    if (-not $receipt.window.main_responsive -or -not $receipt.window.tray_exists) {
        throw 'The ordinary-user main window and tray are unavailable.'
    }
    if ($ExpectedClientVersion) {
        # The optional backend pass uses a transient loopback inspector. Keep
        # its scope distinct from the preceding inspector-free startup pass.
        $nativePreference = $ErrorActionPreference
        try {
            $ErrorActionPreference = 'Continue'
            $result = & node "$PSScriptRoot\probe-user-backend.mjs" $ExpectedClientVersion
            $probeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $nativePreference }
        if ($probeExit -ne 0) { throw 'The ordinary-user native backend probe failed; inspect startup-user.log.' }
        $receipt.backend = $result | ConvertFrom-Json
        $receipt.native_backend_verified = $true
        $receipt.startup_to_backend_ready_ms = [int]([DateTime]::UtcNow - $app.StartTime.ToUniversalTime()).TotalMilliseconds
    }
    $receipt.status = 'passed'
} catch {
    $receipt.status = 'failed'
    $receipt.error = $_.Exception.Message
    throw
} finally {
    [IO.File]::WriteAllText((Join-Path $OutputDirectory 'startup-user.json'), ($receipt | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    # The VM check requires no pre-existing Orbit instance. If observation fails
    # before returning its PID, the sole instance still belongs to this check.
    if (-not $app) { $app = Get-Process orivelle-orbit -ErrorAction SilentlyContinue }
    if ($app -and -not $app.HasExited) { Stop-Process -Id $app.Id }
}
