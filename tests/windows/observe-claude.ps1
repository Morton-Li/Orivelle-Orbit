param(
    [Parameter(Mandatory)][string]$AppPath,
    [Parameter(Mandatory)][string]$EvidenceDirectory
)
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
if($env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm'){throw 'Explicit disposable Windows VM scope is required.'}
New-Item -ItemType Directory -Force $EvidenceDirectory | Out-Null
$exe=Join-Path $AppPath 'Claude.exe'
$signature=Get-AuthenticodeSignature -LiteralPath $exe
if($signature.Status -ne 'Valid' -or $signature.SignerCertificate.GetNameInfo([Security.Cryptography.X509Certificates.X509NameType]::SimpleName,$false) -ne 'Anthropic, PBC'){throw 'Claude signature invalid.'}
# Reload the real installed client after the confirmed profile save.
Get-Process -Name Claude -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force
$source='Orbit-Claude-Helper-'+[Guid]::NewGuid().ToString('N')
$subscription=$null
$screenReaderChanged=$false
$originalScreenReader=0
$externalTrace=$env:ORBIT_WINDOWS_PROCESS_TRACE_DIRECTORY
Add-Type -AssemblyName System.Windows.Forms,System.Drawing,UIAutomationClient,UIAutomationTypes
Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public static class OrbitClaudeInput {
    private delegate bool WindowCallback(IntPtr window,IntPtr data);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr window,WindowCallback callback,IntPtr data);
    [DllImport("user32.dll",EntryPoint="SystemParametersInfoW")] public static extern bool GetSetting(uint action,uint param,ref int value,uint flags);
    [DllImport("user32.dll",EntryPoint="SystemParametersInfoW")] public static extern bool SetSetting(uint action,uint param,IntPtr value,uint flags);
    [DllImport("oleacc.dll")] private static extern int AccessibleObjectFromWindow(IntPtr window,uint id,ref Guid iid,[MarshalAs(UnmanagedType.Interface)] out object accessible);
    public static void EnableAccessibility(IntPtr window) {
        WindowCallback enable=(child,data)=>{var iid=new Guid("618736e0-3c3d-11cf-810c-00aa00389b71");object accessible;
            AccessibleObjectFromWindow(child,0xfffffffcu,ref iid,out accessible);
            if(accessible!=null && Marshal.IsComObject(accessible))Marshal.ReleaseComObject(accessible);return true;};
        enable(window,IntPtr.Zero);EnumChildWindows(window,enable,IntPtr.Zero);
    }
}
'@
$proof=[ordered]@{ helper_started_by_installed_client=$false; client_version=(Get-Item $exe).VersionInfo.ProductVersion; app_path=$AppPath; ui=@() }
try {
    if(-not $externalTrace){$subscription=Register-CimIndicationEvent -Query 'SELECT * FROM Win32_ProcessStartTrace' -SourceIdentifier $source}
    $trace=@{}
    $observeAfter=[DateTime]::UtcNow
    if(-not [OrbitClaudeInput]::GetSetting(0x46,0,[ref]$originalScreenReader,0)){throw 'Could not read the transient accessibility state.'}
    if($originalScreenReader -eq 0){
        if(-not [OrbitClaudeInput]::SetSetting(0x47,1,[IntPtr]::Zero,2)){throw 'Could not enable transient accessibility.'}
        $screenReaderChanged=$true
    }
    $package=Get-AppxPackage -Name Claude -ErrorAction SilentlyContinue | Where-Object { $AppPath.StartsWith($_.InstallLocation,[StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
    if($package){
        $manifest=Get-AppxPackageManifest -Package $package.PackageFullName
        $app=$manifest.Package.Applications.Application | Where-Object { $_.Executable -like '*claude.exe' } | Select-Object -First 1
        if(-not $app){throw 'Claude application ID unavailable.'}
        Start-Process explorer.exe -ArgumentList ('shell:AppsFolder\'+$package.PackageFamilyName+'!'+$app.Id)
    }else{Start-Process -FilePath $exe}
    $proof.driver=[ordered]@{submitted=$false;response_rendered=$false}
    try {
        $editor=$null;$root=$null
        $deadline=[DateTime]::UtcNow.AddSeconds(30)
        while([DateTime]::UtcNow -lt $deadline -and -not $editor){
            foreach($process in @(Get-Process Claude -ErrorAction SilentlyContinue | Where-Object {$_.Path -and $_.Path.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase)})){
                if($process.MainWindowHandle -eq 0){continue}
                [void][OrbitClaudeInput]::SetForegroundWindow($process.MainWindowHandle)
                [OrbitClaudeInput]::EnableAccessibility($process.MainWindowHandle)
                $root=[Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
                $condition=[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Edit)
                $editors=$root.FindAll([Windows.Automation.TreeScope]::Descendants,$condition)
                $editor=$editors | Where-Object {-not $_.Current.IsOffscreen -and $_.Current.IsEnabled -and $_.Current.IsKeyboardFocusable -and $_.Current.BoundingRectangle.Width -gt 200} | Select-Object -First 1
                if($editor){break}
            }
            if(-not $editor){Start-Sleep -Milliseconds 500}
        }
        if(-not $editor){throw 'Claude chat editor was not exposed through native accessibility.'}
        $editor.SetFocus()
        [Windows.Forms.SendKeys]::SendWait('Windows_native_configuration_check')
        $start=$null;$deadline=[DateTime]::UtcNow.AddSeconds(5)
        while([DateTime]::UtcNow -lt $deadline -and -not $start){
            $buttons=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.PropertyCondition]::new([Windows.Automation.AutomationElement]::ControlTypeProperty,[Windows.Automation.ControlType]::Button))
            $start=$buttons | Where-Object {$_.Current.Name -eq 'Start task' -and $_.Current.IsEnabled -and -not $_.Current.IsOffscreen} | Select-Object -First 1
            if(-not $start){Start-Sleep -Milliseconds 200}
        }
        if(-not $start){throw 'The observed Start task control was not enabled.'}
        ([Windows.Automation.InvokePattern]$start.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern)).Invoke()
        $proof.driver.submitted=$true
        $deadline=[DateTime]::UtcNow.AddSeconds(60)
        while([DateTime]::UtcNow -lt $deadline -and -not $proof.driver.response_rendered){
            $elements=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
            $proof.driver.response_rendered=@($elements | Where-Object {$_.Current.Name -like '*Windows native configuration verified.*'}).Count -gt 0
            if(-not $proof.driver.response_rendered){Start-Sleep -Milliseconds 500}
        }
        if(-not $proof.driver.response_rendered){throw 'Claude did not render the controlled response.'}
    } catch {$proof.driver.error=$_.Exception.Message}
    $deadline=[DateTime]::UtcNow.AddSeconds(10)
    while([DateTime]::UtcNow -lt $deadline -and -not $proof.helper_started_by_installed_client){
        if($externalTrace){
            foreach($file in @(Get-ChildItem -LiteralPath $externalTrace -Filter 'helper-*.json')){
                $observation=Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8|ConvertFrom-Json
                if([DateTime]::Parse($observation.started_utc).ToUniversalTime() -lt $observeAfter){continue}
                $ancestor=$observation.ancestors|Where-Object {$_.path -and $_.path.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase)}|Select-Object -First 1
                if($ancestor){$proof.helper_started_by_installed_client=$true;$proof.helper_ancestor=$ancestor.name;$proof.helper_observation=$observation;break}
            }
            if(-not $proof.helper_started_by_installed_client){Start-Sleep -Milliseconds 200}
            continue
        }
        $event=Wait-Event -SourceIdentifier $source -Timeout 1
        if(-not $event){continue}
        $started=$event.SourceEventArgs.NewEvent
        if($trace.Count -gt 1000){throw 'Process observation exceeded its bound.'}
        $trace[[int]$started.ProcessID]=$started
        if($started.ProcessName -eq 'orbit-credential-helper.exe'){
            $parentId=[int]$started.ParentProcessID
            for($depth=0;$depth -lt 8 -and $parentId -ne 0;$depth++){
                $parent=Get-CimInstance Win32_Process -Filter "ProcessId=$parentId" -ErrorAction SilentlyContinue
                if($parent.ExecutablePath -and $parent.ExecutablePath.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase)){
                    $proof.helper_started_by_installed_client=$true
                    $proof.helper_ancestor=$parent.Name
                    break
                }
                if($trace.ContainsKey($parentId)){$parentId=[int]$trace[$parentId].ParentProcessID}
                elseif($parent){$parentId=[int]$parent.ParentProcessId}
                else{break}
            }
        }
        Remove-Event -EventIdentifier $event.EventIdentifier
    }
    $processes=@(Get-Process -Name Claude -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase) })
    foreach($process in $processes){
        if($process.MainWindowHandle -eq 0){continue}
        $root=[Windows.Automation.AutomationElement]::FromHandle($process.MainWindowHandle)
        $elements=$root.FindAll([Windows.Automation.TreeScope]::Descendants,[Windows.Automation.Condition]::TrueCondition)
        $proof.ui+=@($elements | Select-Object -First 150 | ForEach-Object { [pscustomobject]@{name=$_.Current.Name;control=$_.Current.ControlType.ProgrammaticName} })
    }
    $bounds=[Windows.Forms.SystemInformation]::VirtualScreen
    $bitmap=[Drawing.Bitmap]::new($bounds.Width,$bounds.Height)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    try{$graphics.CopyFromScreen($bounds.Location,[Drawing.Point]::Empty,$bounds.Size);$bitmap.Save((Join-Path $EvidenceDirectory 'claude-loaded.png'))}finally{$graphics.Dispose();$bitmap.Dispose()}
    $proof | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $EvidenceDirectory 'claude-loaded.json') -Encoding UTF8
    $proof | ConvertTo-Json -Depth 5 -Compress
} finally {
    if($screenReaderChanged){[void][OrbitClaudeInput]::SetSetting(0x47,[uint32]$originalScreenReader,[IntPtr]::Zero,2)}
    if($subscription){Unregister-Event -SourceIdentifier $source -ErrorAction SilentlyContinue;Remove-Job -Id $subscription.Id -Force -ErrorAction SilentlyContinue}
    Get-Event -SourceIdentifier $source -ErrorAction SilentlyContinue | Remove-Event -ErrorAction SilentlyContinue
    Get-Process -Name Claude -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith($AppPath,[StringComparison]::OrdinalIgnoreCase) } | Stop-Process -Force -ErrorAction SilentlyContinue
}
