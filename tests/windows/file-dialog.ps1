param(
    [Parameter(Mandatory)][int]$OrbitProcess,
    [Parameter(Mandatory)][string]$FilePath
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

public static class OrbitTestWindows {
    private delegate bool EnumCallback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumCallback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);

    public static IntPtr[] VisibleForProcess(int process) {
        var windows = new List<IntPtr>();
        EnumWindows((window, data) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == (uint)process && IsWindowVisible(window)) windows.Add(window);
            return true;
        }, IntPtr.Zero);
        return windows.ToArray();
    }
}
'@

$scope = [System.Windows.Automation.TreeScope]
$element = [System.Windows.Automation.AutomationElement]
$fileNameIds = [System.Windows.Automation.OrCondition]::new([System.Windows.Automation.Condition[]]@(
    [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1148'),
    [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1001'),
    [System.Windows.Automation.PropertyCondition]::new($element::NameProperty, 'File name:')
))
$valueCondition = [System.Windows.Automation.PropertyCondition]::new($element::IsValuePatternAvailableProperty, $true)
$fileNameCondition = [System.Windows.Automation.AndCondition]::new($fileNameIds, $valueCondition)
$acceptIds = [System.Windows.Automation.OrCondition]::new([System.Windows.Automation.Condition[]]@(
    [System.Windows.Automation.PropertyCondition]::new($element::AutomationIdProperty, '1'),
    [System.Windows.Automation.PropertyCondition]::new($element::NameProperty, 'Save'),
    [System.Windows.Automation.PropertyCondition]::new($element::NameProperty, 'Open')
))
$invokeCondition = [System.Windows.Automation.PropertyCondition]::new($element::IsInvokePatternAvailableProperty, $true)
$acceptCondition = [System.Windows.Automation.AndCondition]::new($acceptIds, $invokeCondition)

$deadline = (Get-Date).AddSeconds(15)
$dialog = $null
do {
    $windowNames = @()
    foreach ($handle in [OrbitTestWindows]::VisibleForProcess($OrbitProcess)) {
        $window = $element::FromHandle($handle)
        $windowNames += "$($window.Current.Name) [$($window.Current.ClassName)]"
        if ($window.Current.ClassName -ne '#32770' -and $window.Current.Name -notin @('Open', 'Save As')) {
            continue
        }
        $dialog = $window
        $fileName = $dialog.FindFirst($scope::Descendants, $fileNameCondition)
        $accept = $dialog.FindFirst($scope::Descendants, $acceptCondition)
        if ($fileName -and $accept) {
            $fileName.GetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern).SetValue($FilePath)
            $accept.GetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern).Invoke()
            return
        }
    }
    Start-Sleep -Milliseconds 200
} while ((Get-Date) -lt $deadline)

if ($dialog) {
    $controls = $dialog.FindAll($scope::Descendants, [System.Windows.Automation.Condition]::TrueCondition)
    foreach ($control in $controls) {
        if ($control.Current.ControlType -in @([System.Windows.Automation.ControlType]::Edit, [System.Windows.Automation.ControlType]::ComboBox, [System.Windows.Automation.ControlType]::Button)) {
            Write-Output "Control $($control.Current.ControlType.ProgrammaticName) [$($control.Current.AutomationId)] $($control.Current.Name)"
        }
    }
}
throw "The Orbit file dialog did not expose a filename field and an accept button. Visible Orbit windows: $($windowNames -join ', ')"
