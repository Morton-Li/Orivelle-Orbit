param([Parameter(Mandatory)][string]$OutputDirectory)
$ErrorActionPreference='Stop'
if($env:ORBIT_WINDOWS_TEST_SCOPE -ne 'disposable-vm'){throw 'Use the console observer only on an explicitly disposable VM.'}
Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;using System.Text;using System.Collections.Generic;using System.Runtime.InteropServices;
public static class OrbitConsoleEvents {
 delegate void Callback(IntPtr h,uint e,IntPtr w,int o,int c,uint t,uint m);
 [DllImport("user32.dll")] static extern IntPtr SetWinEventHook(uint a,uint b,IntPtr d,Callback c,uint p,uint t,uint f);
 [DllImport("user32.dll")] static extern bool UnhookWinEvent(IntPtr h);
 [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr w,out uint p);
 [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr w);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr w,StringBuilder s,int n);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] static extern int GetWindowText(IntPtr w,StringBuilder s,int n);
 public class Entry {public long Window;public uint Process;public uint Time;public string Class;public string Title;public bool VisibleAtObservation;}
 public static List<Entry> Entries=new List<Entry>();static Callback callback=Record;static IntPtr hook;
 public static void Start(){hook=SetWinEventHook(0x8002,0x8002,IntPtr.Zero,callback,0,0,0);if(hook==IntPtr.Zero)throw new Exception("Console hook failed");}
 static void Record(IntPtr h,uint e,IntPtr w,int o,int c,uint t,uint m){
  if(o!=0||c!=0||Entries.Count>=128)return;var cls=new StringBuilder(256);GetClassName(w,cls,256);
  if(cls.ToString()!="ConsoleWindowClass"&&cls.ToString()!="CASCADIA_HOSTING_WINDOW_CLASS")return;
  uint p;GetWindowThreadProcessId(w,out p);var title=new StringBuilder(512);GetWindowText(w,title,512);
  Entries.Add(new Entry{Window=w.ToInt64(),Process=p,Time=m,Class=cls.ToString(),Title=title.ToString(),VisibleAtObservation=IsWindowVisible(w)});
 }
 public static void Stop(){UnhookWinEvent(hook);GC.KeepAlive(callback);}
}
'@
$known=@{}
function Capture-Processes {Get-CimInstance Win32_Process | ForEach-Object {$known[[int]$_.ProcessId]=$_|Select-Object Name,ProcessId,ParentProcessId,SessionId,ExecutablePath}}
Capture-Processes
[OrbitConsoleEvents]::Start()
$clock=[Diagnostics.Stopwatch]::StartNew();$captured=0
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'console-trace.ready'),'ready')
try {
 while(-not (Test-Path (Join-Path $OutputDirectory 'console-trace.stop')) -and $clock.Elapsed.TotalMinutes -lt 30){
  [Windows.Forms.Application]::DoEvents()
  if([OrbitConsoleEvents]::Entries.Count -gt $captured){Capture-Processes;$captured=[OrbitConsoleEvents]::Entries.Count}
  Start-Sleep -Milliseconds 50
 }
} finally {
 [OrbitConsoleEvents]::Stop()
 $events=@([OrbitConsoleEvents]::Entries|ForEach-Object {$entry=$_;$id=[int]$entry.Process;$parents=@();for($i=0;$i -lt 6 -and $known.ContainsKey($id);$i++){$p=$known[$id];$parents+=$p;$id=[int]$p.ParentProcessId};[pscustomobject]@{window=$entry;ancestors=$parents}})
 $stopRequested=Test-Path (Join-Path $OutputDirectory 'console-trace.stop')
 [pscustomobject]@{scope='Entire interactive desktop during acceptance; console SHOW events, including short-lived windows';duration_ms=$clock.ElapsedMilliseconds;stop_requested=$stopRequested;console_events=$events;status=if(-not $stopRequested){'timed_out'}elseif($events.Count -eq 0){'passed'}else{'observed'}}|ConvertTo-Json -Depth 9|Set-Content (Join-Path $OutputDirectory 'console-trace.json') -Encoding UTF8
}
