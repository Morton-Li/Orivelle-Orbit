param([Parameter(Mandatory)][string]$Context)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = [Security.Principal.WindowsPrincipal]::new($identity)
if ($principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw 'The fixture client process must run without elevation.'
}
$contextData = Get-Content -LiteralPath $Context -Raw -Encoding UTF8 | ConvertFrom-Json
Set-Location -LiteralPath $contextData.directory
foreach ($entry in $contextData.environment.PSObject.Properties) {
    [Environment]::SetEnvironmentVariable($entry.Name, [string]$entry.Value, 'Process')
}
if ($env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm') { throw 'An explicitly disposable VM is required.' }
New-Item -ItemType Directory -Force runtime\logs | Out-Null
@{elevated=$false;user_sid=$identity.User.Value;session_id=(Get-Process -Id $PID).SessionId} | ConvertTo-Json | Set-Content runtime\logs\windows-user-test-principal.json -Encoding UTF8
$ErrorActionPreference = 'Continue'
if ($env:ORBIT_WINDOWS_ACCEPTANCE_SCOPE -eq 'client-only') {
    & node tests/windows/client-configuration.mjs *> $contextData.log
} else {
    & node tests/windows/desktop.mjs *> $contextData.log
}
exit $LASTEXITCODE
