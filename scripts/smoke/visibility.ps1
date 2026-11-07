# Native window visibility check for the smoke run (ASCII-only on purpose).
# Main window: class=Tauri Window, title='zap'（diag-windows.ps1 枚举实证）。
# ⚠️ class 参数必须走 IntPtr.Zero 过载：PowerShell 会把 $null 强转成空串传给
# string 参数，FindWindowW(空类名) 匹配不到任何窗口（两轮 notfound 的真因）。
# 另：单实例插件另有一个 class=com.ais.zap.smoke-sic / title=<identifier>-siw
# 的信使窗口（恒 visible），不是主窗口，别拿它当探针目标。
param([string]$Title = 'zap')
$sig = @'
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern System.IntPtr FindWindowW(System.IntPtr cls, string title);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(System.IntPtr h);
'@
Add-Type -MemberDefinition $sig -Name U -Namespace W
$h = [W.U]::FindWindowW([System.IntPtr]::Zero, $Title)
if ($h -eq [System.IntPtr]::Zero) { Write-Output 'notfound' }
elseif ([W.U]::IsWindowVisible($h)) { Write-Output 'visible' }
else { Write-Output 'hidden' }
