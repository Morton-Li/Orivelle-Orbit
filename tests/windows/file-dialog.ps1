param(
    [Parameter(Mandatory)][int]$OrbitProcess,
    [Parameter(Mandatory)][string]$FilePath
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Use this picker automation only on a GitHub-hosted Windows runner.'
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class OrbitTestPicker {
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder name, int size);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);

    public static IntPtr Find(int process) {
        IntPtr picker = IntPtr.Zero;
        EnumWindows((window, data) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != (uint)process || !IsWindowVisible(window)) return true;
            var name = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            if (name.ToString() != "#32770") return true;
            picker = window;
            return false;
        }, IntPtr.Zero);
        return picker;
    }
}
'@

$deadline = (Get-Date).AddSeconds(15)
do {
    $picker = [OrbitTestPicker]::Find($OrbitProcess)
    if ($picker -ne [IntPtr]::Zero) {
        if ([OrbitTestPicker]::GetForegroundWindow() -ne $picker) {
            [OrbitTestPicker]::SetForegroundWindow($picker) | Out-Null
        }
        if ([OrbitTestPicker]::GetForegroundWindow() -eq $picker) {
            # Alt+N targets the filename field in the Windows open/save dialog.
            [System.Windows.Forms.Clipboard]::SetText($FilePath)
            [System.Windows.Forms.SendKeys]::SendWait('%n')
            [System.Windows.Forms.SendKeys]::SendWait('^a^v')
            if ([OrbitTestPicker]::GetForegroundWindow() -ne $picker) {
                throw 'The Windows picker lost focus before confirmation.'
            }
            [System.Windows.Forms.SendKeys]::SendWait('{ENTER}')
            return
        }
    }
    Start-Sleep -Milliseconds 200
} while ((Get-Date) -lt $deadline)

throw 'The Orbit open or save dialog did not become active.'
