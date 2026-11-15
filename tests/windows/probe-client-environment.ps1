param(
    [string]$OutputDirectory = (Join-Path $env:TEMP 'orbit-client-probe'),
    [switch]$AsJson
)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
if (-not [Environment]::Is64BitOperatingSystem) { throw 'Windows x64 is required.' }

# Read installation metadata and configuration field names only. Never export
# configuration values, registry policy values, tokens or credential contents.
$errors = [Collections.Generic.List[string]]::new()
function File-Fact([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return $null }
    try {
        $item = Get-Item -LiteralPath $Path -Force
        $fact = [ordered]@{
            path = $item.FullName
            directory = $item.PSIsContainer
            bytes = if ($item.PSIsContainer) { $null } else { $item.Length }
            attributes = [string]$item.Attributes
            modified_utc = $item.LastWriteTimeUtc.ToString('o')
            owner = (Get-Acl -LiteralPath $Path).Owner
        }
        if (-not $item.PSIsContainer -and $item.Extension -eq '.exe') {
            $signature = Get-AuthenticodeSignature -LiteralPath $Path
            $fact.signature_status = [string]$signature.Status
            $fact.signer = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null }
            $fact.product_version = $item.VersionInfo.ProductVersion
        }
        return [pscustomobject]$fact
    } catch {
        $errors.Add("metadata:${Path}:$($_.Exception.GetType().Name)")
        return $null
    }
}

function Configuration-Keys([string]$Path) {
    $fact = File-Fact $Path
    if (-not $fact -or $fact.directory) { return $fact }
    if ($fact.bytes -gt 1048576 -or $fact.attributes -match 'ReparsePoint') { return $fact }
    try {
        if ([IO.Path]::GetExtension($Path) -eq '.json') {
            $document = [IO.File]::ReadAllText($Path) | ConvertFrom-Json
            $fact | Add-Member -NotePropertyName top_level_keys -NotePropertyValue @($document.PSObject.Properties.Name | Sort-Object)
        } elseif ([IO.Path]::GetExtension($Path) -eq '.toml') {
            $names = foreach ($line in [IO.File]::ReadLines($Path)) {
                if ($line -match '^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=') { $Matches[1] }
            }
            $fact | Add-Member -NotePropertyName key_names -NotePropertyValue @($names | Sort-Object -Unique)
        }
    } catch { $errors.Add("keys:${Path}:$($_.Exception.GetType().Name)") }
    return $fact
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$os = Get-CimInstance Win32_OperatingSystem
$packages = @(Get-AppxPackage | Where-Object { $_.Name -match '(?i)OpenAI.*Codex|Anthropic.*Claude|^Claude$' } |
    Select-Object Name, Version, Publisher, PackageFamilyName, InstallLocation, SignatureKind, Status)
$programs = foreach ($base in @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
)) {
    if (Test-Path -LiteralPath $base) {
        Get-ChildItem -LiteralPath $base | ForEach-Object { Get-ItemProperty -LiteralPath $_.PSPath } |
            Where-Object { $_.DisplayName -match '(?i)\bCodex\b|\bClaude\b' } |
            Select-Object DisplayName, DisplayVersion, Publisher, InstallLocation
    }
}
$roots = @($packages.InstallLocation) + @($programs.InstallLocation) + @(
    (Join-Path $env:LOCALAPPDATA 'AnthropicClaude'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Claude'),
    (Join-Path $env:LOCALAPPDATA 'Programs\Codex')
)
$installation = foreach ($root in @($roots | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Sort-Object -Unique)) {
    # Fixed shallow locations avoid walking caches or unrelated user files.
    foreach ($relative in @('', 'app', 'resources', 'app\resources', 'resources\codex-cli', 'app\resources\codex-cli')) {
        $directory = if ($relative) { Join-Path $root $relative } else { $root }
        if (-not (Test-Path -LiteralPath $directory)) { continue }
        Get-ChildItem -LiteralPath $directory -File -Force | Where-Object {
            $_.Name -match '(?i)^(codex|claude).*\.exe$|^app\.asar$|^codex-package\.json$|^package\.json$|^models\.json$'
        } | Select-Object -First 64 | ForEach-Object { File-Fact $_.FullName }
    }
}
$configuration = foreach ($root in @(
    (Join-Path $env:USERPROFILE '.codex'),
    (Join-Path $env:LOCALAPPDATA 'Claude-3p'),
    (Join-Path $env:APPDATA 'Claude-3p'),
    (Join-Path $env:APPDATA 'Claude')
)) {
    $fact = File-Fact $root
    if (-not $fact) { continue }
    $children = foreach ($relative in @('config.toml', 'claude_desktop_config.json', 'configLibrary\_meta.json')) {
        Configuration-Keys (Join-Path $root $relative)
    }
    [pscustomobject]@{ root = $fact; files = @($children | Where-Object { $_ }) }
}
$tools = foreach ($name in @('node.exe', 'npm.cmd', 'cargo.exe', 'rustc.exe', 'git.exe', 'cl.exe', 'msbuild.exe', 'winget.exe')) {
    $command = Get-Command $name -ErrorAction SilentlyContinue | Select-Object -First 1
    [pscustomobject]@{ name = $name; path = if ($command) { $command.Source } else { $null } }
}
$report = [ordered]@{
    schema_version = 1
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
    os = @{ caption = $os.Caption; version = $os.Version; build = $os.BuildNumber; architecture = $os.OSArchitecture }
    user = @{ name = $identity.Name; sid = $identity.User.Value; profile = $env:USERPROFILE; local_app_data = $env:LOCALAPPDATA; app_data = $env:APPDATA }
    packages = $packages
    programs = @($programs)
    installation = @($installation | Where-Object { $_ })
    configuration = @($configuration)
    tools = @($tools)
    errors = $errors.ToArray()
}
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$destination = Join-Path $OutputDirectory 'environment.json'
$json = $report | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText($destination, $json, [Text.UTF8Encoding]::new($false))
if ($AsJson) { Write-Output $json } else { Write-Output $destination }
