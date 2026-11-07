# Enumerate all top-level windows of the smoke process (ASCII-only on purpose).
param([int]$ProcId)
Add-Type @'
using System; using System.Runtime.InteropServices; using System.Text;
public class WEnum {
  public delegate bool EnumProc(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  public static string Dump(uint target) {
    var sb = new System.Text.StringBuilder();
    EnumWindows((h, l) => {
      uint pid; GetWindowThreadProcessId(h, out pid);
      if (pid == target) {
        var t = new StringBuilder(256); GetWindowTextW(h, t, 256);
        var cn = new StringBuilder(256); GetClassNameW(h, cn, 256);
        sb.Append(h.ToString()).Append(" | class=").Append(cn).Append(" | visible=").Append(IsWindowVisible(h)).Append(" | title=[").Append(t).Append("]").AppendLine();
      }
      return true;
    }, IntPtr.Zero);
    return sb.ToString();
  }
}
'@
[WEnum]::Dump($ProcId)
