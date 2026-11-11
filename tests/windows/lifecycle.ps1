param(
    [Parameter(Mandatory)][int]$OrbitProcess,
    [Parameter(Mandatory)][ValidateSet('state', 'close', 'double-click', 'menu', 'dialog')][string]$Action,
    [string]$Item,
    [string]$Screenshot
)
$ErrorActionPreference = 'Stop'
if ($env:RUNNER_ENVIRONMENT -ne 'github-hosted' -or $env:RUNNER_OS -ne 'Windows') {
    throw 'Use native lifecycle automation only on a GitHub-hosted Windows runner.'
}
Add-Type -AssemblyName System.Windows.Forms, System.Drawing, UIAutomationClient, UIAutomationTypes, Accessibility
Add-Type -ReferencedAssemblies ([Accessibility.IAccessible].Assembly.Location) -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using Accessibility;

public static class OrbitLifecycle {
    private delegate bool Callback(IntPtr window, IntPtr data);
    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct IconIdentifier {
        public uint Size; public IntPtr Window; public uint Id; public Guid Guid;
    }
    [DllImport("user32.dll")] private static extern bool EnumWindows(Callback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder name, int size);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder name, int size);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] private static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
    [DllImport("shell32.dll")] private static extern int Shell_NotifyIconGetRect(ref IconIdentifier identifier, out Rect rect);
    [DllImport("oleacc.dll")] private static extern int AccessibleObjectFromWindow(IntPtr window, uint objectId, ref Guid iid, [MarshalAs(UnmanagedType.Interface)] out IAccessible accessible);

    public static IntPtr Find(int process, string kind) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, data) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != (uint)process) return true;
            var name = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            string cls = name.ToString();
            GetWindowText(window, name, name.Capacity);
            if ((kind == "tray" && cls == "tray_icon_app") ||
                (kind == "main" && cls != "#32770" && name.ToString() == "Orivelle Orbit") ||
                (kind == "menu" && cls == "#32768" && IsWindowVisible(window)) ||
                (kind == "dialog" && cls == "#32770" && IsWindowVisible(window))) {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static bool OverflowVisible() {
        bool visible = false;
        EnumWindows((window, data) => {
            var name = new StringBuilder(256);
            GetClassName(window, name, name.Capacity);
            if (name.ToString() == "TopLevelWindowForOverflowXamlIsland" && IsWindowVisible(window)) {
                visible = true;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return visible;
    }
    public static Rect IconRect(int process) {
        var window = Find(process, "tray");
        if (window == IntPtr.Zero) throw new Exception("The native tray window was not created.");
        for (uint id = 1; id <= 32; id++) {
            var identifier = new IconIdentifier { Size = (uint)Marshal.SizeOf(typeof(IconIdentifier)), Window = window, Id = id };
            Rect rect;
            if (Shell_NotifyIconGetRect(ref identifier, out rect) == 0 && rect.Right > rect.Left && rect.Bottom > rect.Top) return rect;
        }
        throw new Exception("Windows Shell did not register the Orbit notification icon.");
    }
    public static void Close(int process) {
        var window = Find(process, "main");
        if (window == IntPtr.Zero || !PostMessage(window, 0x0010, IntPtr.Zero, IntPtr.Zero)) throw new Exception("Could not send the native close event.");
    }
    private static string MenuLabel(string name) {
        name = (name ?? "").Split('\t')[0];
        if (name.EndsWith("\u2026")) return name.Substring(0, name.Length - 1);
        if (name.EndsWith("...")) return name.Substring(0, name.Length - 3);
        return name;
    }
    private static bool FindMenuItem(IAccessible parent, object child, string requested, List<string> names, int depth, out Rect rect) {
        rect = new Rect();
        if (depth > 8) return false;
        string name = parent.get_accName(child);
        names.Add(name ?? "(unnamed)");
        if (Convert.ToInt32(parent.get_accRole(child)) == 12 && MenuLabel(name) == MenuLabel(requested) &&
            (Convert.ToInt32(parent.get_accState(child)) & 1) == 0) {
            int x, y, width, height;
            parent.accLocation(out x, out y, out width, out height, child);
            rect = new Rect { Left = x, Top = y, Right = x + width, Bottom = y + height };
            return width > 0 && height > 0;
        }
        if (!child.Equals(0)) return false;
        for (int id = 1; id <= parent.accChildCount; id++) {
            var accessible = parent.get_accChild(id) as IAccessible;
            if (FindMenuItem(accessible ?? parent, accessible == null ? (object)id : 0, requested, names, depth + 1, out rect)) return true;
        }
        return false;
    }
    public static Rect MenuItemRect(int process, string requested) {
        var menu = Find(process, "menu");
        var iid = new Guid("618736E0-3C3D-11CF-810C-00AA00389B71");
        IAccessible accessible;
        int result = AccessibleObjectFromWindow(menu, 0xFFFFFFFC, ref iid, out accessible);
        if (result != 0 || accessible == null) throw new Exception("Could not read the native popup menu through MSAA: " + result);
        var names = new List<string>();
        Rect rect;
        if (FindMenuItem(accessible, 0, requested, names, 0, out rect)) return rect;
        throw new Exception("The native menu item was not found: " + requested + ". MSAA items: " + string.Join(" | ", names));
    }
    public static void Click(Rect rect, bool twice, bool right) {
        if (!SetCursorPos((rect.Left + rect.Right) / 2, (rect.Top + rect.Bottom) / 2)) throw new Exception("Could not position the native pointer.");
        uint down = right ? 0x0008u : 0x0002u;
        uint up = right ? 0x0010u : 0x0004u;
        mouse_event(down, 0, 0, 0, UIntPtr.Zero); mouse_event(up, 0, 0, 0, UIntPtr.Zero);
        if (twice) {
            System.Threading.Thread.Sleep(80);
            mouse_event(down, 0, 0, 0, UIntPtr.Zero); mouse_event(up, 0, 0, 0, UIntPtr.Zero);
        }
    }
}
'@

function Save-Screen {
    if (-not $Screenshot) { return }
    $bounds = [Windows.Forms.Screen]::PrimaryScreen.Bounds
    $bitmap = New-Object Drawing.Bitmap $bounds.Width, $bounds.Height
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($bounds.Location, [Drawing.Point]::Empty, $bounds.Size)
        $bitmap.Save($Screenshot, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
}

if ($Action -eq 'close') { [OrbitLifecycle]::Close($OrbitProcess) }
if ($Action -eq 'double-click' -or $Action -eq 'menu') {
    # Newly registered icons may start in Windows' overflow panel.
    # Invoking an already-open panel closes it and leaves the icon rect stale.
    if (-not [OrbitLifecycle]::OverflowVisible()) {
        $buttons = [Windows.Automation.AutomationElement]::RootElement.FindAll(
            [Windows.Automation.TreeScope]::Descendants,
            (New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty), ([Windows.Automation.ControlType]::Button))
        )
        foreach ($button in $buttons) {
            if ($button.Current.AutomationId -eq 'SystemTrayIcon' -or $button.Current.Name -eq 'Show hidden icons') {
                $button.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
                $deadline = (Get-Date).AddSeconds(5)
                while (-not [OrbitLifecycle]::OverflowVisible() -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 100 }
                if (-not [OrbitLifecycle]::OverflowVisible()) { Save-Screen; throw 'The Windows tray overflow panel did not open.' }
                break
            }
        }
    }
    [OrbitLifecycle]::Click([OrbitLifecycle]::IconRect($OrbitProcess), $Action -eq 'double-click', $Action -eq 'menu')
}

if ($Action -eq 'menu') {
    $deadline = (Get-Date).AddSeconds(10)
    do {
        $menu = [OrbitLifecycle]::Find($OrbitProcess, 'menu')
        if ($menu -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    if ($menu -eq [IntPtr]::Zero) { Save-Screen; throw 'The native Orbit popup menu did not open.' }
    Save-Screen
    # Read the standard Win32 popup with MSAA and click the exact named item.
    # https://learn.microsoft.com/windows/win32/winauto/menu-item
    [OrbitLifecycle]::Click([OrbitLifecycle]::MenuItemRect($OrbitProcess, $Item), $false, $false)
    $selected = $true
    $name = $Item
}

if ($Action -eq 'dialog') {
    $deadline = (Get-Date).AddSeconds(10)
    $selected = $false
    $available = @()
    do {
        $root = [Windows.Automation.AutomationElement]::RootElement
        $dialog = [OrbitLifecycle]::Find($OrbitProcess, 'dialog')
        if ($dialog -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100; continue }
        $root = [Windows.Automation.AutomationElement]::FromHandle($dialog)
        $condition = New-Object Windows.Automation.PropertyCondition ([Windows.Automation.AutomationElement]::ControlTypeProperty), ([Windows.Automation.ControlType]::Button)
        $elements = $root.FindAll([Windows.Automation.TreeScope]::Descendants, $condition)
        $element = $null
        $available = @()
        foreach ($candidate in $elements) {
            $name = $candidate.Current.Name
            $available += $name
            # Win32 accessibility can expose an ellipsis as three periods or omit it.
            # Keep the control type and the rest of the label exact.
            $accessibleName = [regex]::Replace($name, '(?:\.{3}|\u2026)$', '')
            $requestedName = [regex]::Replace($Item, '(?:\.{3}|\u2026)$', '')
            if ($accessibleName -ceq $requestedName) { $element = $candidate; break }
        }
        if ($element -and $element.Current.IsEnabled) {
            Save-Screen
            $element.GetCurrentPattern([Windows.Automation.InvokePattern]::Pattern).Invoke()
            $selected = $true
            break
        }
        Start-Sleep -Milliseconds 100
    } while ((Get-Date) -lt $deadline)
    if (-not $selected) { Save-Screen; throw "The native $Action item '$Item' did not become available. Accessible items: $($available -join ' | ')" }
}
if ($Action -eq 'state') { Save-Screen }
$main = [OrbitLifecycle]::Find($OrbitProcess, 'main')
$tray = [OrbitLifecycle]::Find($OrbitProcess, 'tray')
@{
    main_exists = $main -ne [IntPtr]::Zero
    main_visible = $main -ne [IntPtr]::Zero -and [OrbitLifecycle]::IsWindowVisible($main)
    tray_exists = $tray -ne [IntPtr]::Zero
    selected_item = if ($selected) { $name } else { $null }
} | ConvertTo-Json -Compress
