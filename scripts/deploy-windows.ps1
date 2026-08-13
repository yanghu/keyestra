[CmdletBinding()]
param(
    [switch]$Activate
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command
    )

    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE."
    }
}

function Get-PackageVersion {
    param([Parameter(Mandatory = $true)][string]$ManifestPath)

    $inPackage = $false
    foreach ($line in Get-Content -LiteralPath $ManifestPath) {
        if ($line -match '^\s*\[(.+)\]\s*$') {
            $inPackage = $Matches[1] -eq "package"
            continue
        }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"([^"]+)"') {
            return $Matches[1]
        }
    }
    throw "Could not find [package].version in $ManifestPath."
}

function Assert-RequiredFiles {
    param(
        [Parameter(Mandatory = $true)][string]$Root,
        [Parameter(Mandatory = $true)][string[]]$RelativePaths
    )

    foreach ($relativePath in $RelativePaths) {
        $path = Join-Path $Root $relativePath
        $item = Get-Item -LiteralPath $path -ErrorAction Stop
        if ($item.PSIsContainer -or $item.Length -le 0) {
            throw "Required deployment file is empty or invalid: $path"
        }
    }
}

function Write-CurrentMetadata {
    param(
        [Parameter(Mandatory = $true)][string]$MetadataPath,
        [Parameter(Mandatory = $true)][string]$Version,
        [Parameter(Mandatory = $true)][string]$Build,
        [Parameter(Mandatory = $true)][string]$ReleasePath,
        [Parameter(Mandatory = $true)][string]$SourceState
    )

    $metadata = [ordered]@{
        version = $Version
        build = $Build
        path = $ReleasePath
        source_state = $SourceState
    }
    $json = $metadata | ConvertTo-Json
    $temporaryPath = "$MetadataPath.tmp-$([Guid]::NewGuid().ToString('N'))"
    $backupPath = "$MetadataPath.backup-$([Guid]::NewGuid().ToString('N'))"
    $utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
    [IO.File]::WriteAllText($temporaryPath, "$json`r`n", $utf8WithoutBom)

    try {
        if (Test-Path -LiteralPath $MetadataPath) {
            [IO.File]::Replace($temporaryPath, $MetadataPath, $backupPath)
        }
        else {
            [IO.File]::Move($temporaryPath, $MetadataPath)
        }
    }
    finally {
        if (Test-Path -LiteralPath $temporaryPath) {
            Remove-Item -LiteralPath $temporaryPath -Force
        }
        if (Test-Path -LiteralPath $backupPath) {
            Remove-Item -LiteralPath $backupPath -Force
        }
    }
}

function Get-ProcessAtPath {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$ExecutablePath
    )

    $expected = [IO.Path]::GetFullPath($ExecutablePath)
    @(Get-Process -Name $Name -ErrorAction SilentlyContinue | Where-Object {
        try {
            [string]::Equals(
                [IO.Path]::GetFullPath($_.Path),
                $expected,
                [StringComparison]::OrdinalIgnoreCase
            )
        }
        catch {
            $false
        }
    })
}

function Activate-Release {
    param([Parameter(Mandatory = $true)][string]$ReleasePath)

    Write-Warning "Activating now stops Keyestra and legacy FP10 processes and discards any unsaved in-memory recorder buffer."
    foreach ($processName in @(
        "keyestra",
        "keyestra-tray",
        "keyestra-monitor",
        "fp10-map",
        "fp10-map-tray",
        "fp10-monitor-server"
    )) {
        Get-Process -Name $processName -ErrorAction SilentlyContinue |
            Stop-Process -Force -ErrorAction Stop
    }

    $trayPath = Join-Path $ReleasePath "keyestra-tray.exe"
    $monitorPath = Join-Path $ReleasePath "keyestra-monitor.exe"
    Start-Process -FilePath $trayPath

    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $tray = @(Get-ProcessAtPath -Name "keyestra-tray" -ExecutablePath $trayPath)
        $monitor = @(Get-ProcessAtPath -Name "keyestra-monitor" -ExecutablePath $monitorPath)
        if ($tray.Count -gt 0 -and $monitor.Count -gt 0) {
            return
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)

    throw "Activation did not leave the deployed tray and monitor running."
}

$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot ".."))
$manifestPath = Join-Path $workspace "Cargo.toml"
$cargoOutput = [IO.Path]::GetFullPath((Join-Path $workspace "target\release"))

if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw "LOCALAPPDATA is not set."
}
if ([string]::IsNullOrWhiteSpace($env:APPDATA)) {
    throw "APPDATA is not set."
}

$deploymentRoot = [IO.Path]::GetFullPath((Join-Path $env:LOCALAPPDATA "keyestra"))
$releasesRoot = Join-Path $deploymentRoot "releases"
$startupPath = [IO.Path]::GetFullPath(
    (Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Startup\keyestra-tray.vbs")
)

Push-Location $workspace
try {
    $snapshotTimestamp = [DateTime]::UtcNow.ToString("yyyyMMddTHHmmssZ")
    $revision = $null
    try {
        $revisionOutput = git rev-parse --short=8 HEAD 2>$null
        if ($LASTEXITCODE -eq 0 -and -not [string]::IsNullOrWhiteSpace($revisionOutput)) {
            $revision = $revisionOutput.Trim()
        }
    }
    catch {
        $revision = $null
    }

    if ($null -eq $revision) {
        $build = "snapshot-$snapshotTimestamp"
        $sourceState = "snapshot"
    }
    else {
        $worktreeStatus = @(git status --porcelain 2>$null)
        $statusAvailable = $LASTEXITCODE -eq 0
        if (-not $statusAvailable) {
            $build = "$revision-state-unknown-$snapshotTimestamp"
            $sourceState = "unknown"
        }
        elseif ($worktreeStatus.Count -gt 0) {
            $build = "$revision-dirty-$snapshotTimestamp"
            $sourceState = "dirty"
        }
        else {
            $build = $revision
            $sourceState = "clean"
        }
    }
    $version = Get-PackageVersion -ManifestPath $manifestPath

    Invoke-CheckedCommand "cargo fmt --check" { cargo fmt --check }
    Invoke-CheckedCommand "cargo test" { cargo test }

    $previousBuildOverride = $env:KEYESTRA_BUILD_ID_OVERRIDE
    try {
        $env:KEYESTRA_BUILD_ID_OVERRIDE = $build
        Invoke-CheckedCommand "cargo build --release" {
            cargo build --release --bin keyestra --bin keyestra-tray --bin keyestra-monitor
        }
    }
    finally {
        $env:KEYESTRA_BUILD_ID_OVERRIDE = $previousBuildOverride
    }
}
finally {
    Pop-Location
}

$releaseName = "$version-$build"
$releaseDir = Join-Path $releasesRoot $releaseName
$requiredFiles = @(
    "keyestra.exe",
    "keyestra-tray.exe",
    "keyestra-monitor.exe",
    "examples\curve.toml",
    "examples\curve-mid-control.toml",
    "examples\reaper-pianos.toml",
    "scripts\reaper\keyestra-piano-compare-bootstrap.lua"
)

if (Test-Path -LiteralPath $releaseDir) {
    Assert-RequiredFiles -Root $releaseDir -RelativePaths $requiredFiles
    foreach ($relativePath in $requiredFiles) {
        $sourceRoot = if (
            $relativePath.StartsWith("examples\") -or
            $relativePath.StartsWith("scripts\")
        ) { $workspace } else { $cargoOutput }
        $sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $sourceRoot $relativePath)).Hash
        $deployedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $releaseDir $relativePath)).Hash
        if ($sourceHash -ne $deployedHash) {
            throw "Immutable release exists with different content: $releaseDir"
        }
    }
    Write-Host "Reusing verified immutable release: $releaseDir"
}
else {
    New-Item -ItemType Directory -Path $releasesRoot -Force | Out-Null
    $stagingDir = Join-Path $releasesRoot (".staging-$releaseName-$([Guid]::NewGuid().ToString('N'))")
    $published = $false

    try {
        New-Item -ItemType Directory -Path (Join-Path $stagingDir "examples") -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $stagingDir "scripts\reaper") -Force | Out-Null
        foreach ($binary in @("keyestra.exe", "keyestra-tray.exe", "keyestra-monitor.exe")) {
            Copy-Item -LiteralPath (Join-Path $cargoOutput $binary) -Destination (Join-Path $stagingDir $binary)
        }
        foreach ($curve in @("curve.toml", "curve-mid-control.toml", "reaper-pianos.toml")) {
            Copy-Item -LiteralPath (Join-Path $workspace "examples\$curve") -Destination (Join-Path $stagingDir "examples\$curve")
        }
        Copy-Item -LiteralPath (Join-Path $workspace "scripts\reaper\keyestra-piano-compare-bootstrap.lua") -Destination (Join-Path $stagingDir "scripts\reaper\keyestra-piano-compare-bootstrap.lua")

        Assert-RequiredFiles -Root $stagingDir -RelativePaths $requiredFiles
        Move-Item -LiteralPath $stagingDir -Destination $releaseDir
        $published = $true
    }
    finally {
        if (-not $published -and (Test-Path -LiteralPath $stagingDir)) {
            Remove-Item -LiteralPath $stagingDir -Recurse -Force
        }
    }
}

Assert-RequiredFiles -Root $releaseDir -RelativePaths $requiredFiles
$currentPath = Join-Path $deploymentRoot "current.json"
Write-CurrentMetadata -MetadataPath $currentPath -Version $version -Build $build -ReleasePath $releaseDir -SourceState $sourceState

$deployedTray = Join-Path $releaseDir "keyestra-tray.exe"
$startupInstaller = Start-Process -FilePath $deployedTray -ArgumentList "--install-startup" -Wait -PassThru
if ($startupInstaller.ExitCode -ne 0) {
    throw "Startup installation failed with exit code $($startupInstaller.ExitCode)."
}
if (-not (Test-Path -LiteralPath $startupPath)) {
    throw "Startup installation did not create $startupPath."
}
$startupScript = Get-Content -Raw -LiteralPath $startupPath
if ($startupScript.IndexOf($deployedTray, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
    throw "Startup does not point to the deployed tray: $deployedTray"
}

$activationPending = -not $Activate
if ($Activate) {
    Activate-Release -ReleasePath $releaseDir
}

Write-Host "Deployed tray: $deployedTray"
Write-Host "Startup script: $startupPath"
Write-Host "Version: $version"
Write-Host "Build ID: $build"
Write-Host "Source state: $sourceState"
Write-Host "Activation pending: $activationPending"
