# zap smoke isolation diagnostics (2026-11-07).
# Re-runs the smoke exe, waits longer, then reports BOTH data dirs
# (smoke + real) with recent file mtimes, plus the temp panic log.
# ASCII-only on purpose (GBK console + UTF-8 PS1 encoding hazard).
param([string]$Exe = "")

if (-not $Exe) {
  # cargo target dir is machine-specific (CARGO_TARGET_DIR, e.g. E:\rust\targets\shared
  # on the dev box); fall back to the repo-local default when unset
  $base = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$PSScriptRoot\..\..\src-tauri\target" }
  $Exe = Join-Path $base 'release\zap.exe'
}

$before = Get-Date
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 10
$smokeDir = Join-Path $env:APPDATA 'com.ais.zap.smoke'
$realDir  = Join-Path $env:APPDATA 'com.ais.zap'
$panicLog = Join-Path $env:TEMP 'zap-panic.log'

Write-Output "process_alive=$(-not $p.HasExited)"
if ($p.HasExited) { Write-Output "exit_code=$($p.ExitCode)" }

Write-Output "--- smoke dir ---"
if (Test-Path $smokeDir) {
  Get-ChildItem $smokeDir -ErrorAction SilentlyContinue |
    ForEach-Object { Write-Output ("{0}  {1}" -f $_.LastWriteTime.ToString('s'), $_.Name) }
} else { Write-Output "MISSING" }

Write-Output "--- real dir (files touched after start would implicate the smoke app) ---"
Get-ChildItem $realDir -File -ErrorAction SilentlyContinue |
  ForEach-Object { $touched = $_.LastWriteTime -gt $before; Write-Output ("{0}  {1}  touched_after_start={2}" -f $_.LastWriteTime.ToString('s'), $_.Name, $touched) }

Write-Output "--- temp panic log ---"
if (Test-Path $panicLog) {
  Write-Output ("mtime={0}" -f ((Get-Item $panicLog).LastWriteTime.ToString('s')))
  Get-Content $panicLog -Tail 5
} else { Write-Output "MISSING" }

Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
