# Install Silas on Windows (nightly channel):
#   irm https://raw.githubusercontent.com/baney75/silas/main/install.ps1 | iex
# Remove it (your dictionary and history stay):
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/baney75/silas/main/install.ps1))) -Uninstall
param([switch]$Uninstall)
$ErrorActionPreference = "Stop"
# The progress bar makes Invoke-WebRequest many times slower in Windows PowerShell 5.1.
$ProgressPreference = "SilentlyContinue"
$dir = Join-Path $env:LOCALAPPDATA "Silas"
$exe = Join-Path $dir "silas.exe"

function Stop-Silas {
  Get-Process silas -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
  Start-Sleep -Milliseconds 500
}

if ($Uninstall) {
  $uninstaller = Join-Path $dir "uninstall.exe"
  if (-not (Test-Path $uninstaller)) { throw "Silas isn't installed in $dir." }
  Stop-Silas
  $run = Start-Process $uninstaller -ArgumentList "/S" -Wait -PassThru
  if ($run.ExitCode -ne 0) { throw "The Silas uninstaller failed (exit code $($run.ExitCode))." }
  # NSIS uninstallers relaunch from a temp copy, so the wait above returns early.
  for ($i = 0; $i -lt 30 -and (Test-Path $exe); $i++) { Start-Sleep -Milliseconds 500 }
  if (Test-Path $exe) { throw "The Silas uninstaller didn't finish; $exe is still there." }
  Write-Host "Removed Silas. Your dictionary, history and voice model are still in $env:APPDATA\Silas." -ForegroundColor Green
  Write-Host "Delete that folder too if you want them gone."
  return
}

if (-not [Environment]::Is64BitOperatingSystem) { throw "Silas needs 64-bit Windows." }
Write-Host "Finding the latest Silas..." -ForegroundColor Yellow
# GITHUB_TOKEN, when set, lifts the API's 60-requests-an-hour limit for shared IPs (CI).
$headers = @{}
if ($env:GITHUB_TOKEN) { $headers.Authorization = "Bearer $env:GITHUB_TOKEN" }
$release = Invoke-RestMethod "https://api.github.com/repos/baney75/silas/releases/tags/nightly" -Headers $headers
$asset = $release.assets | Where-Object { $_.name -like "*-setup.exe" } | Select-Object -First 1
if (-not $asset) { throw "The nightly release is being rebuilt. Try again in a few minutes." }
$file = Join-Path $env:TEMP $asset.name
Write-Host "Downloading Silas ($([math]::Round($asset.size / 1MB)) MB)..." -ForegroundColor Yellow
Invoke-WebRequest $asset.browser_download_url -OutFile $file -UseBasicParsing
Stop-Silas
$setup = Start-Process $file -ArgumentList "/S" -Wait -PassThru
Remove-Item $file -ErrorAction SilentlyContinue
if ($setup.ExitCode -ne 0) { throw "The Silas installer failed (exit code $($setup.ExitCode))." }
if (-not (Test-Path $exe)) { throw "Silas installed, but $exe is missing. Start Silas from the Start menu." }
if ($env:SILAS_NO_OPEN) { Write-Host "Silas is installed in $dir." -ForegroundColor Green; return }
Start-Process $exe
Write-Host "Silas is installed. Hold Right Ctrl, speak, and let go." -ForegroundColor Green
