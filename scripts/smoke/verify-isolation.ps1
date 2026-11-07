# zap smoke isolation verifier (scripts/bench plan, 2026-11-07).
# Starts the smoke-built exe, waits for setup, asserts the data dir is the
# smoke identifier (com.ais.zap.smoke), then kills ONLY the process we
# started. ASCII-only on purpose (GBK console + UTF-8 PS1 encoding hazard).
param([string]$Exe = "")

if (-not $Exe) {
  # cargo target dir is machine-specific (CARGO_TARGET_DIR, e.g. E:\rust\targets\shared
  # on the dev box); fall back to the repo-local default when unset
  $base = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$PSScriptRoot\..\..\src-tauri\target" }
  $Exe = Join-Path $base 'release\zap.exe'
}

if (-not (Test-Path $Exe)) {
  Write-Output "FAIL exe not found: $Exe"
  exit 1
}
$p = Start-Process -FilePath $Exe -PassThru
Start-Sleep -Seconds 5
$smokeDir = Join-Path $env:APPDATA 'com.ais.zap.smoke'
$result = [ordered]@{
  pid           = $p.Id
  smoke_dir     = (Test-Path $smokeDir)
  smoke_zap_log = (Test-Path (Join-Path $smokeDir 'zap.log'))
  smoke_db      = (Test-Path (Join-Path $smokeDir 'zap.db'))
}
if ($p.HasExited) { $result.exited_early = $true }
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
$result | ConvertTo-Json -Compress
