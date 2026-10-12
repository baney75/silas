# Install Silas on Windows (nightly channel):
#   irm https://raw.githubusercontent.com/baney75/silas/main/install.ps1 | iex
$ErrorActionPreference = "Stop"
$release = Invoke-RestMethod "https://api.github.com/repos/baney75/silas/releases/tags/nightly"
$asset = $release.assets | Where-Object { $_.name -like "*-setup.exe" } | Select-Object -First 1
if (-not $asset) { throw "No Windows build found in the nightly release." }
$file = Join-Path $env:TEMP $asset.name
Write-Host "Downloading Silas..." -ForegroundColor Yellow
Invoke-WebRequest $asset.browser_download_url -OutFile $file
Start-Process $file -ArgumentList "/S" -Wait
Start-Process "$env:LOCALAPPDATA\Silas\silas.exe"
Write-Host "Silas is installed. Hold Right Ctrl and speak." -ForegroundColor Green
