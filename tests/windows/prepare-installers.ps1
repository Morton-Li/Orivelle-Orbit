$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Prepare these executables only on a GitHub-hosted Windows runner.'
}

$archiveTool = Join-Path $env:ProgramFiles '7-Zip\7z.exe'
$evidence = Join-Path $PWD 'runtime/logs/windows-integration'
New-Item -ItemType Directory -Force -Path $evidence | Out-Null

function Read-Installer([string]$Installer, [string]$Destination) {
    New-Item -ItemType Directory -Path $Destination -Force | Out-Null
    & $archiveTool t -tNsis $Installer
    if ($LASTEXITCODE -ne 0) { throw 'The installer failed its integrity check.' }
    & $archiveTool e -tNsis -y "-o$Destination" $Installer '-ir!orivelle-orbit.exe'
    if ($LASTEXITCODE -ne 0) { throw 'The installer payload could not be read.' }
    $executable = Join-Path $Destination 'orivelle-orbit.exe'
    $binary = [IO.File]::ReadAllBytes($executable)
    $peHeader = [BitConverter]::ToInt32($binary, 0x3c)
    if ([BitConverter]::ToUInt32($binary, $peHeader) -ne 0x4550 -or [BitConverter]::ToUInt16($binary, $peHeader + 4) -ne 0x8664) {
        throw 'The installer does not contain a Windows x64 executable.'
    }
    return $executable
}

$installers = @(Get-ChildItem -LiteralPath 'runtime/github-release' -Filter '*-setup.exe' -File)
if ($installers.Count -ne 1) { throw 'Expected one candidate installer.' }
# Suppress 7-Zip stdout in this function's pipeline; retain its exit status.
$current = Read-Installer $installers[0].FullName (Join-Path $env:RUNNER_TEMP 'orbit-current-payload') | Select-Object -Last 1
$built = Join-Path $PWD 'runtime/target/x86_64-pc-windows-msvc/release/orivelle-orbit.exe'
if ((Get-FileHash $current -Algorithm SHA256).Hash -ne (Get-FileHash $built -Algorithm SHA256).Hash) {
    node "$PSScriptRoot/compare-executables.mjs" $built $current (Join-Path $evidence 'binary-difference')
    throw 'The installer payload differs from the built program; binary evidence has been retained.'
}
$installation = Join-Path $env:RUNNER_TEMP 'orbit-native-installation'
$installed = Join-Path $installation 'orivelle-orbit.exe'
"ORBIT_WINDOWS_EXECUTABLE=$installed" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
"ORBIT_WINDOWS_INSTALLER=$($installers[0].FullName)" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
$receipt = @{
    candidate_version = (Get-Item $current).VersionInfo.ProductVersion
    built_sha256 = (Get-FileHash $built -Algorithm SHA256).Hash
    archive_extracted_sha256 = (Get-FileHash $current -Algorithm SHA256).Hash
    scope = 'Real NSIS installation, native OS behavior and controlled HTTPS peer; no real upstream calls'
}

if ($env:ORBIT_BASELINE_TAG) {
    $tag = $env:ORBIT_BASELINE_TAG
    if ($tag -notmatch '^v\d+\.\d+\.\d+$') { throw 'Expected a stable baseline tag.' }
    $release = Invoke-RestMethod -Uri "https://api.github.com/repos/$env:GITHUB_REPOSITORY/releases/tags/$tag"
    if ($release.draft -or $release.prerelease) { throw 'The baseline must be a published stable release.' }
    $name = "Orivelle-Orbit-$tag-windows-x64-setup.exe"
    $asset = @($release.assets | Where-Object { $_.name -eq $name })
    $sums = @($release.assets | Where-Object { $_.name -eq 'SHA256SUMS' })
    if ($asset.Count -ne 1 -or $sums.Count -ne 1) { throw 'The baseline installer or checksums are missing.' }
    $baseline = Join-Path $env:RUNNER_TEMP $name
    Invoke-WebRequest -Uri $asset[0].browser_download_url -OutFile $baseline
    $checksumPath = Join-Path $env:RUNNER_TEMP 'orbit-baseline-SHA256SUMS'
    Invoke-WebRequest -Uri $sums[0].browser_download_url -OutFile $checksumPath
    $checksums = Get-Content -LiteralPath $checksumPath -Raw
    $entry = @($checksums -split "`n" | Where-Object { $_.TrimEnd().EndsWith($name) })
    if ($entry.Count -ne 1 -or $entry[0] -notmatch '^([a-fA-F0-9]{64})\s+') { throw 'Missing baseline checksum.' }
    if ((Get-FileHash $baseline -Algorithm SHA256).Hash -ne $Matches[1]) { throw 'The baseline checksum is invalid.' }
    & $archiveTool t -tNsis $baseline
    if ($LASTEXITCODE -ne 0) { throw 'The baseline installer failed its integrity check.' }
    $install = Start-Process -FilePath $baseline -ArgumentList '/S', "/D=$installation" -Wait -PassThru
    if ($install.ExitCode -ne 0) { throw "Baseline installation failed: $($install.ExitCode)." }
    if ((Get-Item $installed).VersionInfo.ProductVersion -ne $tag.Substring(1)) { throw 'Incorrect installed baseline version.' }
    "ORBIT_BASELINE_EXECUTABLE=$installed" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    "ORBIT_BASELINE_TAG=$tag" | Out-File -FilePath $env:GITHUB_ENV -Append -Encoding utf8
    $receipt.baseline_tag = $tag
    $receipt.baseline_sha256 = (Get-FileHash $installed -Algorithm SHA256).Hash
}
$receipt | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $evidence 'installers.json') -Encoding utf8
if (-not $env:ORBIT_BASELINE_TAG) {
    $env:ORBIT_WINDOWS_EXECUTABLE = $installed
    $env:ORBIT_WINDOWS_INSTALLER = $installers[0].FullName
    & "$PSScriptRoot/install-candidate.ps1"
}
