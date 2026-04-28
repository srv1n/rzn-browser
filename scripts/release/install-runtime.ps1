$ErrorActionPreference = "Stop"

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$hostName = if ($env:RZN_NATIVE_HOST_NAME) { $env:RZN_NATIVE_HOST_NAME } else { "com.rzn.browser.broker" }
$extensionId = if ($env:RZN_CHROME_EXTENSION_ID) { $env:RZN_CHROME_EXTENSION_ID } else { "bogjdnehdficgkhklinmnbgiiofbamji" }
$installRoot = if ($env:RZN_RUNTIME_DIR) { $env:RZN_RUNTIME_DIR } else { Join-Path $env:LOCALAPPDATA "RZN" }
$binDir = Join-Path $installRoot "bin"
$extensionDir = Join-Path $installRoot "extension\dist-chrome"
$manifestDir = Join-Path $installRoot "native-host"
$manifestPath = Join-Path $manifestDir "$hostName.json"

function Ensure-Directory([string]$Path) {
  if (-not (Test-Path -LiteralPath $Path)) {
    New-Item -ItemType Directory -Force -Path $Path | Out-Null
  }
}

function Add-ToUserPath([string]$PathEntry) {
  $currentPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $segments = @()
  if ($currentPath) {
    $segments = $currentPath.Split(";") | Where-Object { $_ -and $_.Trim().Length -gt 0 }
  }

  if ($segments -contains $PathEntry) {
    return
  }

  $updated = if ($segments.Count -eq 0) { $PathEntry } else { ($segments + $PathEntry) -join ";" }
  [Environment]::SetEnvironmentVariable("Path", $updated, "User")
}

foreach ($required in @(
  (Join-Path $scriptDir "bin\rzn-browser.exe"),
  (Join-Path $scriptDir "bin\rzn-browser-worker.exe"),
  (Join-Path $scriptDir "bin\rzn-native-host.exe"),
  (Join-Path $scriptDir "extension\dist-chrome\manifest.json")
)) {
  if (-not (Test-Path -LiteralPath $required)) {
    throw "Missing packaged file: $required"
  }
}

Ensure-Directory $binDir
Ensure-Directory (Split-Path -Parent $extensionDir)
Ensure-Directory $manifestDir

Write-Host "[INFO] Installing binaries into: $binDir"
Copy-Item -Force (Join-Path $scriptDir "bin\rzn-browser.exe") (Join-Path $binDir "rzn-browser.exe")
Copy-Item -Force (Join-Path $scriptDir "bin\rzn-browser-worker.exe") (Join-Path $binDir "rzn-browser-worker.exe")
Copy-Item -Force (Join-Path $scriptDir "bin\rzn-native-host.exe") (Join-Path $binDir "rzn-native-host.exe")

Write-Host "[INFO] Installing stable extension copy into: $extensionDir"
if (Test-Path -LiteralPath $extensionDir) {
  Remove-Item -Recurse -Force $extensionDir
}
Copy-Item -Recurse -Force (Join-Path $scriptDir "extension\dist-chrome") $extensionDir

$manifestPayload = @{
  name = $hostName
  description = "RZN Browser Host"
  path = (Join-Path $binDir "rzn-native-host.exe")
  type = "stdio"
  allowed_origins = @("chrome-extension://$extensionId/")
}
$manifestPayload | ConvertTo-Json -Depth 4 | Set-Content -NoNewline -Encoding utf8 $manifestPath

Write-Host "[INFO] Writing Chrome native host registry entry"
& reg.exe ADD "HKCU\Software\Google\Chrome\NativeMessagingHosts\$hostName" /ve /t REG_SZ /d $manifestPath /f | Out-Null

$cliPath = Join-Path $binDir "rzn-browser.exe"
Write-Host "[INFO] Refreshing bundled workflows/examples into: $installRoot\workflows\builtin"
$env:RZN_RUNTIME_DIR = $installRoot
& $cliPath workflow pull --repo-root $scriptDir

Add-ToUserPath $binDir

Write-Host ""
Write-Host "[OK] Installed RZN Browser"
Write-Host "  - runtime: $installRoot"
Write-Host "  - cli: $cliPath"
Write-Host "  - worker: $(Join-Path $binDir 'rzn-browser-worker.exe')"
Write-Host "  - native host: $(Join-Path $binDir 'rzn-native-host.exe')"
Write-Host "  - extension: $extensionDir"
Write-Host "  - native host manifest: $manifestPath"
Write-Host ""
Write-Host "Next:"
Write-Host "1. Restart any open Chrome windows"
Write-Host "2. Open chrome://extensions"
Write-Host "3. Enable Developer mode"
Write-Host "4. Load unpacked from: $extensionDir"
Write-Host "5. Open a new terminal after PATH refresh, then run: rzn-browser workflow list google"
