param([Parameter(Mandatory)][string]$Directory,[Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][ValidateSet('disposable-vm')][string]$Scope)
$ErrorActionPreference='Stop'
$env:ORBIT_WINDOWS_TEST_SCOPE=$Scope
$principal=[Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
if(-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Only the physical input driver requires elevation.'}
$allowed=@('click','right-click','double-click','main-click','menu-item','dialog')
$requests=0
$actions=[Collections.Generic.List[object]]::new()
$traceSource='OrbitLab-Process-'+[Guid]::NewGuid().ToString('N')
$subscription=Register-CimIndicationEvent -Query 'SELECT * FROM Win32_ProcessStartTrace' -SourceIdentifier $traceSource
$trace=@{}
$helperObservations=0
$clock=[Diagnostics.Stopwatch]::StartNew()
[IO.File]::WriteAllText((Join-Path $Directory 'ready'),'ready')
try {
    while(-not (Test-Path (Join-Path $Directory 'stop')) -and $clock.Elapsed.TotalMinutes -lt 25){
        $helpers=@()
        foreach($event in @(Get-Event -SourceIdentifier $traceSource -ErrorAction SilentlyContinue)){
            $started=$event.SourceEventArgs.NewEvent
            if($trace.Count -ge 4096){throw 'Process observation exceeded its bound.'}
            $id=[int]$started.ProcessID
            $live=Get-CimInstance Win32_Process -Filter "ProcessId=$id" -ErrorAction SilentlyContinue
            $row=@{process_id=$id;parent_id=[int]$started.ParentProcessID;name=[string]$started.ProcessName;path=$live.ExecutablePath;started_utc=[DateTime]::FromFileTimeUtc([long]$started.TIME_CREATED).ToString('o')}
            $trace[$id]=$row
            if($row.name -eq 'orbit-credential-helper.exe'){$helpers+=,$row}
            Remove-Event -EventIdentifier $event.EventIdentifier
        }
        foreach($helper in $helpers){
            $ancestors=@();$parentId=$helper.parent_id
            for($depth=0;$depth -lt 8 -and $parentId -ne 0;$depth++){
                $live=Get-CimInstance Win32_Process -Filter "ProcessId=$parentId" -ErrorAction SilentlyContinue
                $parent=if($trace.ContainsKey($parentId)){$trace[$parentId]}elseif($live){@{process_id=$parentId;parent_id=[int]$live.ParentProcessId;name=$live.Name;path=$live.ExecutablePath}}else{$null}
                if(-not $parent){break}
                if($live -and $live.ExecutablePath){$parent.path=$live.ExecutablePath}
                $ancestors+=,$parent;$parentId=$parent.parent_id
            }
            $observation=@{started_utc=$helper.started_utc;process_id=$helper.process_id;ancestors=$ancestors;observer_elevated=$true}
            $name=Join-Path $Directory ('helper-'+[Guid]::NewGuid().ToString('N')+'.json')
            [IO.File]::WriteAllText(($name+'.tmp'),($observation|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
            Move-Item -LiteralPath ($name+'.tmp') -Destination $name
            $helperObservations++
        }
        foreach($file in @(Get-ChildItem -LiteralPath $Directory -Filter '*.request.json')){
            $responsePath=$file.FullName.Replace('.request.json','.response.json')
            if(Test-Path -LiteralPath $responsePath){continue}
            $response=@{ok=$false;stdout='';stderr=''}
            try {
                $request=Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8|ConvertFrom-Json
                if($request.action -notin $allowed){throw 'Unsupported physical input action.'}
                $app=Get-Process -Id ([int]$request.process)
                if($app.Path -ne $Executable){throw 'Physical input target is not the installed Orbit process.'}
                $quote={param($value) "'"+([string]$value).Replace("'","''")+"'"}
                $command="[Console]::OutputEncoding=[Text.UTF8Encoding]::new(`$false); & "+(& $quote (Join-Path $PSScriptRoot 'lifecycle.ps1'))+' -OrbitProcess '+[int]$request.process+' -Action '+(& $quote $request.action)
                if($request.item){$command+=' -Item '+(& $quote $request.item)}
                if($request.screenshot){$command+=' -Screenshot '+(& $quote $request.screenshot)}
                $start=[Diagnostics.ProcessStartInfo]::new()
                $start.FileName=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
                $start.Arguments='-NoLogo -NoProfile -NonInteractive -STA -ExecutionPolicy Bypass -EncodedCommand '+[Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command))
                $start.WorkingDirectory=Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
                $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
                $start.StandardOutputEncoding=[Text.UTF8Encoding]::new($false);$start.StandardErrorEncoding=[Text.UTF8Encoding]::new($false)
                $child=[Diagnostics.Process]::Start($start)
                $stdout=$child.StandardOutput.ReadToEndAsync();$stderr=$child.StandardError.ReadToEndAsync()
                if(-not $child.WaitForExit(30000)){$child.Kill();throw 'Physical input exceeded thirty seconds.'}
                $response=@{ok=($child.ExitCode -eq 0);stdout=$stdout.Result;stderr=$stderr.Result}
                $child.Dispose();$requests++
            } catch {$response.stderr=$_.Exception.Message}
            $actions.Add(@{action=$request.action;process=$request.process;response=$response})
            $temporary=$responsePath+'.tmp'
            [IO.File]::WriteAllText($temporary,($response|ConvertTo-Json -Compress),[Text.UTF8Encoding]::new($false))
            Move-Item -LiteralPath $temporary -Destination $responsePath
        }
        Start-Sleep -Milliseconds 100
    }
} finally {
    # Publish the completed input/ancestry evidence before cancelling WMI's
    # subscription; provider cancellation can outlast task teardown on this VM.
    @{scope='Elevated physical input and passive process observation only; Orbit and client processes remain ordinary user';elevated=$true;requests=$requests;helper_observations=$helperObservations;duration_ms=$clock.ElapsedMilliseconds;actions=$actions}|ConvertTo-Json -Depth 8|Set-Content (Join-Path (Split-Path $PSScriptRoot -Parent|Split-Path -Parent) 'runtime\logs\windows-input-driver.json') -Encoding UTF8
    Unregister-Event -SourceIdentifier $traceSource -ErrorAction SilentlyContinue
    Remove-Job -Id $subscription.Id -Force -ErrorAction SilentlyContinue
    Get-Event -SourceIdentifier $traceSource -ErrorAction SilentlyContinue|Remove-Event -ErrorAction SilentlyContinue
}
