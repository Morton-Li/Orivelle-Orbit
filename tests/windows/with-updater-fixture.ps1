$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Run updater installation acceptance only on a GitHub-hosted Windows runner.'
}
$directory = Join-Path $PWD 'runtime/updater-acceptance'
$hostsPath = Join-Path $env:SystemRoot 'System32/drivers/etc/hosts'
$hostsBefore = [IO.File]::ReadAllBytes($hostsPath)
$policy = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
$app = 'orivelle-orbit.exe'
$root = $null
$server = $null
$policyAdded = $false
$hostsChanged = $false
try {
    if (-not (Test-Path $policy)) { New-Item $policy -Force | Out-Null }
    if ((Get-Item $policy).GetValueNames() -contains $app) { throw 'An existing Orbit debug policy must not be replaced.' }
    New-ItemProperty -Path $policy -Name $app -PropertyType String -Value '--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222' | Out-Null
    $policyAdded = $true
    $root = New-SelfSignedCertificate -Type Custom -Subject 'CN=Orbit updater acceptance root' -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -KeyUsage CertSign,CRLSign -TextExtension '2.5.29.19={critical}{text}ca=1&pathlength=0' -NotAfter (Get-Date).AddHours(2) -CertStoreLocation Cert:\LocalMachine\My
    $server = New-SelfSignedCertificate -Type SSLServerAuthentication -DnsName github.com -Signer $root -KeyExportPolicy Exportable -NotAfter (Get-Date).AddHours(1) -CertStoreLocation Cert:\LocalMachine\My
    $certificate = Join-Path $directory 'root.cer'
    Export-Certificate -Cert $root -FilePath $certificate | Out-Null
    Import-Certificate -FilePath $certificate -CertStoreLocation Cert:\LocalMachine\Root | Out-Null
    $env:ORBIT_UPDATER_PFX = Join-Path $directory 'fixture.pfx'
    $env:ORBIT_UPDATER_PASSPHRASE = [Guid]::NewGuid().ToString('N')
    $password = ConvertTo-SecureString $env:ORBIT_UPDATER_PASSPHRASE -AsPlainText -Force
    Export-PfxCertificate -Cert $server -FilePath $env:ORBIT_UPDATER_PFX -Password $password -CryptoAlgorithmOption AES256_SHA256 | Out-Null
    $hostsChanged = $true
    [IO.File]::AppendAllText($hostsPath, "`r`n127.0.0.1 github.com # Orbit updater acceptance`r`n", [Text.Encoding]::ASCII)
    Clear-DnsClientCache
    $addresses = [Net.Dns]::GetHostAddresses('github.com')
    if (-not $addresses -or ($addresses | Where-Object { $_.IPAddressToString -ne '127.0.0.1' })) { throw 'Updater requests would leave the controlled HTTPS peer.' }
    $env:NO_PROXY = 'github.com,127.0.0.1,localhost'
    $installation = Join-Path $env:RUNNER_TEMP 'Orbit updater acceptance'
    $install = Start-Process -FilePath (Join-Path $directory 'base-setup.exe') -ArgumentList '/S',"/D=$installation" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Baseline installation failed: $($install.ExitCode)." }
    $env:ORBIT_UPDATER_EXECUTABLE = Join-Path $installation $app
    & node tests/windows/updater.mjs
    if ($LASTEXITCODE -ne 0) { throw "Updater acceptance failed: $LASTEXITCODE" }
} finally {
    # This runner belongs to this test alone; stop only the installed test app.
    if ($env:ORBIT_UPDATER_EXECUTABLE) {
        Get-Process orivelle-orbit -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $env:ORBIT_UPDATER_EXECUTABLE } | Stop-Process -Force
    }
    if ($hostsChanged) { [IO.File]::WriteAllBytes($hostsPath, $hostsBefore); Clear-DnsClientCache }
    if ($policyAdded) { Remove-ItemProperty -Path $policy -Name $app -ErrorAction SilentlyContinue }
    if ($server) { Remove-Item ("Cert:\LocalMachine\My\" + $server.Thumbprint) -Force -ErrorAction SilentlyContinue }
    if ($root) {
        Remove-Item ("Cert:\LocalMachine\Root\" + $root.Thumbprint) -Force -ErrorAction SilentlyContinue
        Remove-Item ("Cert:\LocalMachine\My\" + $root.Thumbprint) -Force -ErrorAction SilentlyContinue
    }
    Remove-Item Env:ORBIT_UPDATER_PASSPHRASE -ErrorAction SilentlyContinue
    Remove-Item (Join-Path $directory 'fixture.pfx') -Force -ErrorAction SilentlyContinue
}
