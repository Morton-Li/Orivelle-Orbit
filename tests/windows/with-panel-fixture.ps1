$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or $env:RUNNER_ARCH -ne 'X64') {
    throw 'Run the Panel fixture on a GitHub-hosted Windows x64 runner.'
}

# The published application uses a fixed Panel origin. Route that origin to a
# short-lived HTTPS fixture on this disposable runner, using the real binary.
$panelHost = 'orivelle.powerbyte.io'
$hostsPath = Join-Path $env:SystemRoot 'System32\drivers\etc\hosts'
$originalHosts = [IO.File]::ReadAllBytes($hostsPath)
$fixtureDirectory = Join-Path $env:RUNNER_TEMP ('orbit-panel-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
$root = $null
$server = $null
$hostsChanged = $false

try {
    $root = New-SelfSignedCertificate -Type Custom -Subject 'CN=Orbit CI fixture root' -KeyAlgorithm RSA -KeyLength 2048 -HashAlgorithm SHA256 -KeyUsage CertSign, CRLSign -TextExtension '2.5.29.19={critical}{text}ca=1&pathlength=0' -NotAfter (Get-Date).AddHours(2) -CertStoreLocation 'Cert:\LocalMachine\My'
    $server = New-SelfSignedCertificate -Type SSLServerAuthentication -DnsName $panelHost -Signer $root -KeyExportPolicy Exportable -NotAfter (Get-Date).AddHours(1) -CertStoreLocation 'Cert:\LocalMachine\My'

    $certificate = Join-Path $fixtureDirectory 'root.cer'
    Export-Certificate -Cert $root -FilePath $certificate | Out-Null
    Import-Certificate -FilePath $certificate -CertStoreLocation 'Cert:\LocalMachine\Root' | Out-Null

    $env:ORBIT_FIXTURE_PFX = Join-Path $fixtureDirectory 'panel.pfx'
    $env:ORBIT_FIXTURE_PASSPHRASE = [Guid]::NewGuid().ToString('N')
    $password = ConvertTo-SecureString $env:ORBIT_FIXTURE_PASSPHRASE -AsPlainText -Force
    Export-PfxCertificate -Cert $server -FilePath $env:ORBIT_FIXTURE_PFX -Password $password -CryptoAlgorithmOption AES256_SHA256 | Out-Null

    $hostsChanged = $true
    [IO.File]::AppendAllText($hostsPath, "`r`n127.0.0.1 $panelHost # Orbit Windows fixture`r`n", [Text.Encoding]::ASCII)
    Clear-DnsClientCache
    $addresses = [Net.Dns]::GetHostAddresses($panelHost)
    if ($addresses.Count -eq 0 -or ($addresses | Where-Object { $_.IPAddressToString -ne '127.0.0.1' })) {
        throw 'The fixture hostname did not resolve exclusively to loopback.'
    }
    $env:NO_PROXY = "$panelHost,127.0.0.1,localhost"

    node tests/windows/desktop.mjs
    if ($LASTEXITCODE -ne 0) {
        throw "Windows desktop integration failed with exit code $LASTEXITCODE."
    }
} finally {
    if ($hostsChanged) {
        [IO.File]::WriteAllBytes($hostsPath, $originalHosts)
        Clear-DnsClientCache
    }
    if ($server) {
        Remove-Item "Cert:\LocalMachine\My\$($server.Thumbprint)" -DeleteKey
    }
    if ($root) {
        Remove-Item "Cert:\LocalMachine\Root\$($root.Thumbprint)"
        Remove-Item "Cert:\LocalMachine\My\$($root.Thumbprint)" -DeleteKey
    }
    Remove-Item -LiteralPath $fixtureDirectory -Recurse -Force
}
