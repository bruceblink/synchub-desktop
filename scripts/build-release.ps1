param(
    [Parameter(Mandatory = $true)]
    [string]$Version,
    [string]$ProjectRoot = "",
    [string]$OutputDir = ""
)

$ErrorActionPreference = "Stop"

function Invoke-Checked {
    param([string]$FilePath, [string[]]$Arguments)
    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath $($Arguments -join ' ') failed with exit code $LASTEXITCODE"
    }
}

$Version = $Version.Trim().TrimStart("v")
if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw "version must use semantic version format"
}
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = Join-Path $PSScriptRoot ".."
}
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).ProviderPath
$manifest = Get-Content -Raw -LiteralPath (Join-Path $ProjectRoot "Cargo.toml")
if ($manifest -notmatch '(?m)^version\s*=\s*"(?<version>[^"]+)"') {
    throw "Cargo.toml package version was not found"
}
if ($Matches.version -ne $Version) {
    throw "release version $Version does not match Cargo.toml version $($Matches.version)"
}
if ([string]::IsNullOrWhiteSpace($OutputDir)) {
    $OutputDir = Join-Path $ProjectRoot "dist"
}
elseif (-not [System.IO.Path]::IsPathRooted($OutputDir)) {
    $OutputDir = Join-Path $ProjectRoot $OutputDir
}
New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
$OutputDir = (Resolve-Path -LiteralPath $OutputDir).ProviderPath
$releaseRoot = Join-Path $OutputDir "synchub-desktop-$Version"
if (Test-Path -LiteralPath $releaseRoot) {
    throw "release output already exists: $releaseRoot"
}

Push-Location $ProjectRoot
try {
    New-Item -ItemType Directory -Path $releaseRoot | Out-Null
    Invoke-Checked "cargo" @("build", "--release", "--locked")
    $artifactName = "synchub-desktop-$Version-windows-x64"
    $staging = Join-Path $releaseRoot $artifactName
    New-Item -ItemType Directory -Path $staging | Out-Null
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "target/release/synchub-desktop.exe") -Destination $staging
    Copy-Item -LiteralPath (Join-Path $ProjectRoot "README.md") -Destination $staging
    $archiveName = "$artifactName.zip"
    $archivePath = Join-Path $releaseRoot $archiveName
    Compress-Archive -Path (Join-Path $staging "*") -DestinationPath $archivePath
    Remove-Item -LiteralPath $staging -Recurse -Force
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $archivePath).Hash.ToLowerInvariant()
    "$hash  $archiveName" | Set-Content -LiteralPath (Join-Path $releaseRoot "SHA256SUMS.txt") -Encoding ascii
    Write-Output "desktop release artifacts written: $releaseRoot"
}
catch {
    if (Test-Path -LiteralPath $releaseRoot) {
        Remove-Item -LiteralPath $releaseRoot -Recurse -Force
    }
    throw
}
finally {
    Pop-Location
}
