# Does a second-instance summon actually SHOW the native main window?
# Spawns the smoke app, probes visibility, summons, then polls 6s.
# ASCII-only on purpose.
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9224'
$exe = 'E:\rust\targets\shared\release\zap.exe'
$visScript = Join-Path $PSScriptRoot 'visibility.ps1'

$app = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 5
Write-Output ("pre-summon: " + (& $visScript))

Start-Process -FilePath $exe  # second instance = toggle
for ($i = 1; $i -le 15; $i++) {
  Start-Sleep -Milliseconds 400
  $t = [math]::Round($i * 0.4, 1)
  Write-Output ("t+${t}s: " + (& $visScript))
}

Get-Process zap -ErrorAction SilentlyContinue |
  Where-Object { $_.Path -eq $exe } |
  Stop-Process -Force
