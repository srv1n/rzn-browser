$ErrorActionPreference = "Stop"

function Get-EnvOrDefault([string]$Name, [string]$DefaultValue) {
  $value = [Environment]::GetEnvironmentVariable($Name)
  if ([string]::IsNullOrWhiteSpace($value)) {
    return $DefaultValue
  }
  return $value
}

function Test-FullyQualifiedPath([string]$Path) {
  if (-not [System.IO.Path]::IsPathRooted($Path)) {
    return $false
  }
  $root = [System.IO.Path]::GetPathRoot($Path)
  if ([string]::IsNullOrWhiteSpace($root)) {
    return $false
  }
  $directorySeparator = [System.IO.Path]::DirectorySeparatorChar.ToString()
  $altDirectorySeparator = [System.IO.Path]::AltDirectorySeparatorChar.ToString()
  if ($root -eq $directorySeparator -or $root -eq $altDirectorySeparator) {
    return $true
  }
  return $root.EndsWith($directorySeparator) -or $root.EndsWith($altDirectorySeparator)
}

function Assert-SafeChildPath([string]$Path, [string]$ExpectedRoot, [string]$LeafPrefix) {
  if ([string]::IsNullOrWhiteSpace($Path) -or [string]::IsNullOrWhiteSpace($ExpectedRoot)) {
    throw "Refusing to remove empty path."
  }
  if (-not (Test-FullyQualifiedPath $Path) -or -not (Test-FullyQualifiedPath $ExpectedRoot)) {
    throw "Refusing to remove non-absolute path: $Path"
  }

  $fullPath = [System.IO.Path]::GetFullPath($Path)
  $fullRoot = [System.IO.Path]::GetFullPath($ExpectedRoot).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
  )
  $rootOnly = [System.IO.Path]::GetPathRoot($fullRoot).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
  )
  if ([string]::IsNullOrWhiteSpace($fullRoot) -or $fullRoot -eq $rootOnly) {
    throw "Refusing to remove path under unsafe root: $fullRoot"
  }

  $leaf = Split-Path -Leaf $fullPath
  if (-not $leaf.StartsWith($LeafPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove path without expected prefix '$LeafPrefix': $fullPath"
  }

  $rootWithSeparator = $fullRoot + [System.IO.Path]::DirectorySeparatorChar
  if (-not $fullPath.StartsWith($rootWithSeparator, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to remove path outside expected root '$fullRoot': $fullPath"
  }

  return $fullPath
}

function Remove-DirectorySafely([string]$Path, [string]$ExpectedRoot, [string]$LeafPrefix) {
  $safePath = Assert-SafeChildPath -Path $Path -ExpectedRoot $ExpectedRoot -LeafPrefix $LeafPrefix
  if (Test-Path -LiteralPath $safePath) {
    Remove-Item -Recurse -Force -LiteralPath $safePath
  }
}

function Verify-ArtifactSha256([string]$ArchivePath, [string]$SidecarPath, [string]$ArtifactName) {
  if (-not (Test-Path -LiteralPath $SidecarPath)) {
    throw "Missing sha256 sidecar: $SidecarPath"
  }

  $line = Get-Content -LiteralPath $SidecarPath -TotalCount 1
  if ([string]::IsNullOrWhiteSpace($line)) {
    throw "Empty sha256 sidecar: $SidecarPath"
  }

  $parts = $line.Trim() -split "\s+"
  $expected = $parts[0].ToLowerInvariant()
  if ($expected -notmatch "^[0-9a-f]{64}$") {
    throw "Invalid sha256 sidecar format: $SidecarPath"
  }

  if ($parts.Count -gt 1) {
    $sidecarArtifact = [System.IO.Path]::GetFileName($parts[1].TrimStart("*"))
    if ($sidecarArtifact -and -not [string]::Equals($sidecarArtifact, $ArtifactName, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "sha256 sidecar is for $sidecarArtifact, not $ArtifactName."
    }
  }

  $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $ArchivePath).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    throw "Checksum mismatch for ${ArtifactName}. Expected $expected, got $actual."
  }

  return $actual
}

$repo = Get-EnvOrDefault "RZN_INSTALL_REPO" "srv1n/rzn-browser"
$artifactName = Get-EnvOrDefault "RZN_INSTALL_ARTIFACT" "rzn-browser-windows-x64.zip"
if ([string]::IsNullOrWhiteSpace($artifactName) -or $artifactName -ne [System.IO.Path]::GetFileName($artifactName)) {
  throw "RZN_INSTALL_ARTIFACT must be a file name, got: $artifactName"
}
$version = Get-EnvOrDefault "RZN_INSTALL_VERSION" ""
if (-not [string]::IsNullOrWhiteSpace($version)) {
  if ($version -notmatch '^[A-Za-z0-9._-]+$') {
    throw "RZN_INSTALL_VERSION must be a release tag, got: $version"
  }
  $releasePath = "releases/download/$version"
} else {
  $releasePath = "releases/latest/download"
}
$baseUrl = Get-EnvOrDefault "RZN_INSTALL_BASE_URL" "https://github.com/$repo/$releasePath"
$artifactUrl = Get-EnvOrDefault "RZN_INSTALL_URL" "$baseUrl/$artifactName"
$sha256Url = Get-EnvOrDefault "RZN_INSTALL_SHA256_URL" "$artifactUrl.sha256"

$tempRoot = [System.IO.Path]::GetTempPath()
$workdir = Join-Path $tempRoot ("rzn-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $workdir | Out-Null

try {
  $archivePath = Join-Path $workdir $artifactName
  $sha256Path = "$archivePath.sha256"
  $extractDir = Join-Path $workdir "extract"

  Write-Host "[INFO] Downloading $artifactUrl"
  Invoke-WebRequest -Uri $artifactUrl -OutFile $archivePath

  Write-Host "[INFO] Downloading $sha256Url"
  try {
    Invoke-WebRequest -Uri $sha256Url -OutFile $sha256Path
  } catch {
    throw "Missing sha256 sidecar: $sha256Url"
  }

  $verifiedSha256 = Verify-ArtifactSha256 -ArchivePath $archivePath -SidecarPath $sha256Path -ArtifactName $artifactName
  Write-Host "[INFO] Verified sha256: $verifiedSha256"

  Expand-Archive -LiteralPath $archivePath -DestinationPath $extractDir -Force

  $bundleRoot = Get-ChildItem -LiteralPath $extractDir -Directory | Where-Object {
    Test-Path -LiteralPath (Join-Path $_.FullName "install.ps1")
  } | Select-Object -First 1

  if (-not $bundleRoot) {
    throw "Release artifact did not contain an install.ps1 payload."
  }

  Write-Host "[INFO] Running packaged installer from $($bundleRoot.FullName)"
  $env:RZN_INSTALL_ARTIFACT_SHA256_VERIFIED = "1"
  $env:RZN_INSTALL_ARTIFACT_SHA256 = $verifiedSha256
  & (Join-Path $bundleRoot.FullName "install.ps1")
} finally {
  Remove-DirectorySafely -Path $workdir -ExpectedRoot $tempRoot -LeafPrefix "rzn-install-"
}
