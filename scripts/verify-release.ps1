param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$ProjectRoot = "",
    [string]$ReleaseDir = ""
)

$ErrorActionPreference = "Stop"
$Version = $Version.Trim().TrimStart("v")
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = Join-Path $PSScriptRoot ".."
}
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).ProviderPath
if ([string]::IsNullOrWhiteSpace($ReleaseDir)) {
    $ReleaseDir = Join-Path $ProjectRoot "dist/synchub-desktop-$Version"
}
$ReleaseDir = (Resolve-Path -LiteralPath $ReleaseDir).ProviderPath
$archiveName = "synchub-desktop-$Version-windows-x64.zip"
$archivePath = Join-Path $ReleaseDir $archiveName
$checksumPath = Join-Path $ReleaseDir "SHA256SUMS.txt"
if (-not (Test-Path -LiteralPath $archivePath)) { throw "release archive is missing: $archivePath" }
if (-not (Test-Path -LiteralPath $checksumPath)) { throw "checksum file is missing: $checksumPath" }
$line = (Get-Content -LiteralPath $checksumPath | Where-Object { $_.Trim() } | Select-Object -First 1)
if ($line -notmatch '^(?<hash>[0-9a-fA-F]{64})\s+(?<name>[^/\\]+)$') {
    throw "invalid checksum line: $line"
}
if ($Matches.name -ne $archiveName) { throw "checksum references unexpected archive: $($Matches.name)" }
$actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
if ($actual -ne $Matches.hash.ToLowerInvariant()) { throw "checksum mismatch for $archiveName" }

$temp = Join-Path ([System.IO.Path]::GetTempPath()) "synchub-desktop-release-$([guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $temp
    foreach ($name in @("synchub-desktop.exe", "README.md")) {
        if (-not (Test-Path -LiteralPath (Join-Path $temp $name))) {
            throw "$archiveName is missing $name"
        }
    }
}
finally {
    Remove-Item -LiteralPath $temp -Recurse -Force
}
Write-Output "desktop release artifacts verified: $ReleaseDir"
