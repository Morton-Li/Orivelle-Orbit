$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Install this candidate only on a GitHub-hosted Windows runner.'
}
$installation = Split-Path -Parent $env:ORBIT_WINDOWS_EXECUTABLE
$install = Start-Process -FilePath $env:ORBIT_WINDOWS_INSTALLER -ArgumentList '/S', "/D=$installation" -Wait -PassThru
if ($install.ExitCode -ne 0) { throw "Candidate installation failed: $($install.ExitCode)." }
$built = Join-Path $PWD 'runtime/target/x86_64-pc-windows-msvc/release/orivelle-orbit.exe'
node "$PSScriptRoot/../../scripts/windows-package-evidence.mjs" $built $env:ORBIT_WINDOWS_EXECUTABLE (Join-Path $PWD 'runtime/logs/windows-integration/installed-proof.json')
if ($LASTEXITCODE -ne 0) { throw 'The installed candidate did not match the compiled NSIS program.' }
$expected = (Get-Content -LiteralPath 'runtime/logs/windows-integration/installed-proof.json' -Raw | ConvertFrom-Json).expected_packaged_sha256
$actual = (Get-FileHash $env:ORBIT_WINDOWS_EXECUTABLE -Algorithm SHA256).Hash
if ($actual -ne $expected) { throw "The installed candidate differs from the built executable: $actual != $expected." }
$receiptPath = Join-Path $PWD 'runtime/logs/windows-integration/installers.json'
$receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json -AsHashtable
$receipt.installed_sha256 = $actual
$receipt.installed_version = (Get-Item $env:ORBIT_WINDOWS_EXECUTABLE).VersionInfo.ProductVersion
$expectedVersion = (Get-Content -LiteralPath 'src-tauri/tauri.conf.json' -Raw | ConvertFrom-Json).version
if ($receipt.installed_version -ne $expectedVersion) { throw 'The installed candidate has the wrong release version.' }
$receipt.archive_extraction_matches_installation = $receipt.archive_extracted_sha256 -eq $actual
$receipt | ConvertTo-Json | Set-Content -LiteralPath $receiptPath -Encoding utf8
Write-Output "Installed candidate $($receipt.installed_version) matches every compiled byte with the expected Tauri NSIS marker."
