$ErrorActionPreference = "Stop"

function Get-EnvOrDefault([string]$Name, [string]$DefaultValue) {
  $value = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($value)) {
    return $DefaultValue
  }
  return $value
}

$repo = Get-EnvOrDefault "RZN_INSTALL_REPO" "srv1n/rzn-browser"
$artifactName = Get-EnvOrDefault "RZN_INSTALL_ARTIFACT" "rzn-browser-windows-x64.zip"
$baseUrl = Get-EnvOrDefault "RZN_INSTALL_BASE_URL" "https://github.com/$repo/releases/latest/download"
$artifactUrl = Get-EnvOrDefault "RZN_INSTALL_URL" "$baseUrl/$artifactName"

$workdir = Join-Path ([System.IO.Path]::GetTempPath()) ("rzn-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $workdir | Out-Null

try {
  $archivePath = Join-Path $workdir $artifactName
  $extractDir = Join-Path $workdir "extract"

  Write-Host "[INFO] Downloading $artifactUrl"
  Invoke-WebRequest -Uri $artifactUrl -OutFile $archivePath

  Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir -Force

  $bundleRoot = Get-ChildItem -LiteralPath $extractDir -Directory | Where-Object {
    Test-Path -LiteralPath (Join-Path $_.FullName "install.ps1")
  } | Select-Object -First 1

  if (-not $bundleRoot) {
    throw "Release artifact did not contain an install.ps1 payload."
  }

  Write-Host "[INFO] Running packaged installer from $($bundleRoot.FullName)"
  & (Join-Path $bundleRoot.FullName "install.ps1")
} finally {
  if (Test-Path -LiteralPath $workdir) {
    Remove-Item -Recurse -Force $workdir
  }
}
