param(
    [Parameter(Mandatory = $true)]
    [string]$Tag,
    [string]$OutputPath = "CHANGELOG.md",
    [string]$RepositoryURL = "https://github.com/bruceblink/synchub-desktop"
)

$ErrorActionPreference = "Stop"
$Tag = $Tag.Trim()
if ($Tag -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw "tag must use v-prefixed semantic version format"
}

& git rev-parse --verify --quiet "refs/tags/$Tag" *> $null
if ($LASTEXITCODE -ne 0) {
    throw "tag does not exist: $Tag"
}

$versionTags = @(& git tag --merged $Tag --list "v*" --sort=-version:refname)
if ($LASTEXITCODE -ne 0) {
    throw "git tag failed with exit code $LASTEXITCODE"
}
$previousTag = $versionTags | Where-Object { $_ -ne $Tag } | Select-Object -First 1
$range = $Tag
if ($previousTag) {
    $range = "$previousTag..$Tag"
}

$commits = @(& git log $range --no-merges --format='- %s (`%h`)')
if ($LASTEXITCODE -ne 0) {
    throw "git log failed with exit code $LASTEXITCODE"
}
if ($commits.Count -eq 0) {
    $commits = @("- No user-visible changes.")
}

$lines = @(
    "# SyncHub Desktop $Tag",
    "",
    "## Changes",
    ""
)
$lines += $commits
if ($previousTag) {
    $lines += @(
        "",
        "**Full Changelog:** [$previousTag...$Tag]($RepositoryURL/compare/$previousTag...$Tag)"
    )
}

$parent = Split-Path -Parent $OutputPath
if ($parent -and -not (Test-Path -LiteralPath $parent)) {
    New-Item -ItemType Directory -Force -Path $parent | Out-Null
}
$lines | Set-Content -LiteralPath $OutputPath -Encoding utf8
Write-Output "changelog written: $OutputPath"
