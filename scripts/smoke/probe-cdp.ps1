# Probe: does WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS open a CDP port on this
# machine's WebView2 stack? ASCII-only on purpose.
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9223'
$p = Start-Process -FilePath 'E:\rust\targets\shared\release\zap.exe' -PassThru
Start-Sleep -Seconds 7
$open = $false
try {
  $r = Invoke-WebRequest -UseBasicParsing 'http://127.0.0.1:9223/json/version' -TimeoutSec 3
  $open = $true
  Write-Output "PORT_OPEN: $($r.Content.Substring(0, [Math]::Min(120, $r.Content.Length)))"
} catch {
  Write-Output "PORT_DEAD: $($_.Exception.Message)"
}
Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
if ($open) { exit 0 } else { exit 1 }
