param([switch]$OrdinaryUser)
$ErrorActionPreference = 'Stop'
if (($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows' -or $env:RUNNER_ARCH -ne 'X64') -and $env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm') {
    throw 'Run the Panel fixture on a GitHub-hosted Windows x64 runner.'
}
if ($OrdinaryUser -and $env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm') {
    throw 'The interactive ordinary-user test requires an explicitly disposable VM.'
}

# The published application uses a fixed Panel origin. Route that origin to a
# short-lived HTTPS fixture on this disposable runner, using the real binary.
$panelHost = 'orivelle.powerbyte.io'
$hostsPath = Join-Path $env:SystemRoot 'System32\drivers\etc\hosts'
$originalHosts = [IO.File]::ReadAllBytes($hostsPath)
$debugPolicy = 'HKLM:\SOFTWARE\Policies\Microsoft\Edge\WebView2\AdditionalBrowserArguments'
$debugApp = 'orivelle-orbit.exe'
$fixtureDirectory = Join-Path $env:RUNNER_TEMP ('orbit-panel-' + [Guid]::NewGuid())
New-Item -ItemType Directory -Path $fixtureDirectory | Out-Null
$root = $null
$server = $null
$hostsChanged = $false
$debugAdded = $false
$userTask = $null
$inputTask = $null
$inputDirectory = $null

try {
    # Elevated hosts ignore CDP flags passed in the environment in WebView2 150+.
    if (-not (Test-Path -LiteralPath $debugPolicy)) {
        New-Item -Path $debugPolicy -Force | Out-Null
    }
    if ((Get-Item -LiteralPath $debugPolicy).GetValueNames() -contains $debugApp) {
        throw 'An Orbit WebView2 policy already exists on this runner.'
    }
    New-ItemProperty -LiteralPath $debugPolicy -Name $debugApp -PropertyType String -Value '--remote-debugging-address=127.0.0.1 --remote-debugging-port=9222' | Out-Null
    $debugAdded = $true

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

    if ($OrdinaryUser) {
        # Only fixture provisioning needs elevation. Run Orbit, both installed
        # clients and all native IPC assertions under the user's filtered token.
        $hostExecutable = Join-Path (Get-Location) 'runtime\hidden-test-host.exe'
        if (-not (Test-Path -LiteralPath $hostExecutable)) { throw 'Compile hidden-test-host.cs as a GUI executable first.' }
        $environment = @{}
        foreach ($entry in [Environment]::GetEnvironmentVariables().GetEnumerator()) {
            if ($entry.Key -like 'ORBIT_WINDOWS_*' -or $entry.Key -in @('PATH','RUNNER_ENVIRONMENT','RUNNER_OS','RUNNER_ARCH','RUNNER_TEMP','TEMP','TMP','NO_PROXY','ORBIT_BASELINE_TAG','ORBIT_FIXTURE_PFX','ORBIT_FIXTURE_PASSPHRASE')) {
                $environment[$entry.Key] = $entry.Value
            }
        }
        # This VM refuses SetCursorPos from the filtered task token. Keep
        # software and assertions unelevated; elevate the input robot only.
        $inputDirectory = Join-Path $fixtureDirectory 'native-input'
        New-Item -ItemType Directory -Path $inputDirectory | Out-Null
        $environment['ORBIT_WINDOWS_NATIVE_INPUT_DIRECTORY'] = $inputDirectory
        $environment['ORBIT_WINDOWS_PROCESS_TRACE_DIRECTORY'] = $inputDirectory
        $inputTask = 'OrbitLab-Input-' + [Guid]::NewGuid().ToString('N')
        $inputAction = New-ScheduledTaskAction -Execute $hostExecutable -Argument ('-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Directory "{1}" -Executable "{2}" -Scope disposable-vm' -f (Join-Path $PSScriptRoot 'input-driver.ps1'),$inputDirectory,$env:ORBIT_WINDOWS_EXECUTABLE)
        $inputPrincipal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value) -LogonType Interactive -RunLevel Highest
        Register-ScheduledTask -TaskName $inputTask -Action $inputAction -Principal $inputPrincipal | Out-Null
        Start-ScheduledTask -TaskName $inputTask
        $readyClock = [Diagnostics.Stopwatch]::StartNew()
        while (-not (Test-Path (Join-Path $inputDirectory 'ready'))) {
            if ($readyClock.Elapsed.TotalSeconds -gt 20) { throw 'The physical input driver did not become ready.' }
            Start-Sleep -Milliseconds 200
        }
        $contextPath = Join-Path $fixtureDirectory 'user-context.json'
        $userLog = Join-Path $fixtureDirectory 'user-test.log'
        @{directory=(Get-Location).Path;environment=$environment;log=$userLog} | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $contextPath -Encoding UTF8
        $userTask = 'OrbitLab-User-Fixture-' + [Guid]::NewGuid().ToString('N')
        $action = New-ScheduledTaskAction -Execute $hostExecutable -Argument ('-NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "{0}" -Context "{1}"' -f (Join-Path $PSScriptRoot 'fixture-user.ps1'),$contextPath)
        $principal = New-ScheduledTaskPrincipal -UserId ([Security.Principal.WindowsIdentity]::GetCurrent().User.Value) -LogonType Interactive -RunLevel Limited
        Register-ScheduledTask -TaskName $userTask -Action $action -Principal $principal | Out-Null
        Start-ScheduledTask -TaskName $userTask
        Start-Sleep -Seconds 2
        $clock = [Diagnostics.Stopwatch]::StartNew()
        while ((Get-ScheduledTask -TaskName $userTask).State -eq 'Running') {
            if ($clock.Elapsed.TotalMinutes -gt 20) { throw 'Ordinary-user acceptance exceeded twenty minutes.' }
            Start-Sleep -Seconds 3
        }
        $nativeExit = (Get-ScheduledTaskInfo -TaskName $userTask).LastTaskResult
        if (Test-Path -LiteralPath $userLog) { Get-Content -LiteralPath $userLog -Encoding UTF8 }
    } else {
        $nativePreference = $ErrorActionPreference
        try {
            # Windows PowerShell 5 turns native stderr into ErrorRecords. Let Node
            # finish its receipt/finally, then use its exit code as the result.
            $ErrorActionPreference = 'Continue'
            if ($env:ORBIT_WINDOWS_ACCEPTANCE_SCOPE -eq 'client-only') {
                node tests/windows/client-configuration.mjs
            } else {
                node tests/windows/desktop.mjs
            }
            $nativeExit = $LASTEXITCODE
        } finally { $ErrorActionPreference = $nativePreference }
    }
    if ($nativeExit -ne 0) {
        throw "Windows desktop integration failed with exit code $nativeExit."
    }
} finally {
    if ($inputTask) {
        if ($inputDirectory) { [IO.File]::WriteAllText((Join-Path $inputDirectory 'stop'),'stop') }
        $stopClock = [Diagnostics.Stopwatch]::StartNew()
        while ((Get-ScheduledTask -TaskName $inputTask -ErrorAction SilentlyContinue).State -eq 'Running' -and $stopClock.Elapsed.TotalSeconds -lt 10) { Start-Sleep -Milliseconds 200 }
        Stop-ScheduledTask -TaskName $inputTask -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $inputTask -Confirm:$false -ErrorAction SilentlyContinue
    }
    if ($userTask) {
        Stop-ScheduledTask -TaskName $userTask -ErrorAction SilentlyContinue
        Unregister-ScheduledTask -TaskName $userTask -Confirm:$false -ErrorAction SilentlyContinue
    }
    if ($debugAdded) {
        Remove-ItemProperty -LiteralPath $debugPolicy -Name $debugApp
    }
    if ($hostsChanged) {
        [IO.File]::WriteAllBytes($hostsPath, $originalHosts)
        Clear-DnsClientCache
    }
    if ($server) {
        Remove-Item "Cert:\LocalMachine\My\$($server.Thumbprint)" -DeleteKey
    }
    if ($root) {
        $trustedRoot = "Cert:\LocalMachine\Root\$($root.Thumbprint)"
        if (Test-Path -LiteralPath $trustedRoot) {
            Remove-Item -LiteralPath $trustedRoot
        }
        Remove-Item "Cert:\LocalMachine\My\$($root.Thumbprint)" -DeleteKey
    }
    Remove-Item -LiteralPath $fixtureDirectory -Recurse -Force
}
