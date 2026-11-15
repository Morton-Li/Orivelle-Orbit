using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Text;
using System.Threading.Tasks;

// Compile as a Windows GUI executable. Task Scheduler otherwise creates a
// console before PowerShell can process its -WindowStyle Hidden argument.
internal static class HiddenTestHost {
    private static string Quote(string value) {
        var result = new StringBuilder("\"");
        int slashes = 0;
        foreach (char character in value) {
            if (character == '\\') { slashes++; continue; }
            result.Append('\\', slashes * (character == '"' ? 2 : 1));
            slashes = 0;
            if (character == '"') result.Append('\\');
            result.Append(character);
        }
        result.Append('\\', slashes * 2).Append('"');
        return result.ToString();
    }

    private static int Main(string[] arguments) {
        if (arguments.Length == 0) return 2;
        var quoted = new List<string>();
        foreach (string argument in arguments) quoted.Add(Quote(argument));
        var start = new ProcessStartInfo {
            FileName = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.System), "WindowsPowerShell\\v1.0\\powershell.exe"),
            Arguments = String.Join(" ", quoted),
            UseShellExecute = false,
            CreateNoWindow = true,
            WindowStyle = ProcessWindowStyle.Hidden,
            RedirectStandardOutput = true,
            RedirectStandardError = true
        };
        try {
            using (var process = Process.Start(start)) {
                var output = process.StandardOutput.ReadToEndAsync();
                var error = process.StandardError.ReadToEndAsync();
                process.WaitForExit();
                // A GUI descendant can inherit a pipe and outlive the script.
                // Its lifetime must not keep a completed task running forever.
                try { Task.WaitAll(new Task[] { output, error }, 5000); }
                catch (AggregateException) { }
                try {
                    File.AppendAllText(Path.Combine(AppDomain.CurrentDomain.BaseDirectory, "hidden-test-host.log"),
                        DateTime.UtcNow.ToString("o") + " exit=" + process.ExitCode + Environment.NewLine +
                        (output.Status == TaskStatus.RanToCompletion ? output.Result : "") +
                        (error.Status == TaskStatus.RanToCompletion ? error.Result : ""));
                } catch (IOException) { } catch (UnauthorizedAccessException) { }
                return process.ExitCode;
            }
        } catch { return 1; }
    }
}
