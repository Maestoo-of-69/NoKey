<#
.SYNOPSIS
  Builds the Microsoft Store package (.msix and .msixupload) for NoKey.

.DESCRIPTION
  Steps: build NoKey.exe with the "store" feature, create a clean layout in
  target\store\layout, generate the Store icons, fill AppxManifest.xml from
  the template, index the icons into resources.pri (makepri), pack the .msix
  (makeappx) and zip it as .msixupload.

  The identity values (-IdentityName, -Publisher, -PublisherDisplayName) must
  be copied from Partner Center > your app > Product management > Product
  identity. The defaults are placeholders: such a package is for local
  testing only and the Store will reject it.

  Outputs land in target\store\. The portable build in Release\ is not touched.
#>
[CmdletBinding()]
param(
    [string]$IdentityName = 'NoKey.Dev',
    [string]$Publisher = 'CN=NoKey Dev',
    [string]$PublisherDisplayName = 'NoKey Dev',
    [string]$DisplayName = 'NoKey',
    [string]$Version,
    [switch]$SkipBuild
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$storeDir = Join-Path $projectRoot 'target\store'
$layout = Join-Path $storeDir 'layout'
$listingDir = Join-Path $storeDir 'listing'
$tempDir = Join-Path $storeDir 'tmp'
$cargoTarget = Join-Path $storeDir 'cargo'

function Invoke-Tool {
    param([string]$Name, [string]$Exe, [string[]]$Arguments)
    Write-Host ">> $Name"
    & $Exe @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE."
    }
}

function Get-PackageVersion {
    $toml = Join-Path $projectRoot 'Cargo.toml'
    $inPackage = $false
    foreach ($line in Get-Content -LiteralPath $toml) {
        if ($line -match '^\s*\[(.+)\]\s*$') { $inPackage = ($Matches[1] -eq 'package'); continue }
        if ($inPackage -and $line -match '^\s*version\s*=\s*"(\d+\.\d+\.\d+)"') {
            return "$($Matches[1]).0"
        }
    }
    throw 'Could not find version = "x.y.z" in the [package] section of Cargo.toml.'
}

function Assert-StoreVersion([string]$Value) {
    $parts = $Value.Split('.')
    if ($parts.Count -ne 4) { throw "Version '$Value' must have 4 parts (major.minor.build.revision)." }
    foreach ($p in $parts) {
        $n = 0
        if (-not ($p -match '^\d+$') -or -not [int]::TryParse($p, [ref]$n) -or $n -lt 0 -or $n -gt 65535) {
            throw "Version '$Value': every part must be a number from 0 to 65535."
        }
    }
    if ([int]$parts[0] -eq 0) { throw "Version '$Value': the first part must not be 0." }
    if ([int]$parts[3] -ne 0) { throw "Version '$Value': the last part must be 0 (reserved by the Store)." }
}

function Find-SdkBin {
    $root = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (-not (Test-Path -LiteralPath $root)) { throw "Windows SDK not found at $root." }
    $candidates = Get-ChildItem -LiteralPath $root -Directory |
        Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
        Sort-Object { [version]$_.Name } -Descending
    foreach ($dir in $candidates) {
        $bin = Join-Path $dir.FullName 'x64'
        if ((Test-Path (Join-Path $bin 'makeappx.exe')) -and (Test-Path (Join-Path $bin 'makepri.exe'))) {
            return $bin
        }
    }
    throw "No Windows SDK under $root contains both makeappx.exe and makepri.exe (x64)."
}

function ConvertTo-XmlText([string]$Value) {
    return [System.Security.SecurityElement]::Escape($Value)
}

# --- Inputs ---------------------------------------------------------------
if (-not $Version) { $Version = Get-PackageVersion }
Assert-StoreVersion $Version

if ($IdentityName -eq 'NoKey.Dev' -or $Publisher -eq 'CN=NoKey Dev' -or $PublisherDisplayName -eq 'NoKey Dev') {
    Write-Warning 'Placeholder identity in use: this package is for local testing only and the Store will reject it.'
}

$sdkBin = Find-SdkBin
$makeappx = Join-Path $sdkBin 'makeappx.exe'
$makepri = Join-Path $sdkBin 'makepri.exe'
Write-Host "Windows SDK tools: $sdkBin"
Write-Host "Package version:   $Version"

# --- Build ----------------------------------------------------------------
Push-Location $projectRoot
try {
    if (-not $SkipBuild) {
        # Own target dir, so target\release\NoKey.exe stays the GitHub (non-store) build.
        Invoke-Tool 'cargo build --release --features store' 'cargo' @('build', '--release', '--features', 'store', '--target-dir', $cargoTarget)
    }
    $exe = Join-Path $cargoTarget 'release\NoKey.exe'
    if (-not (Test-Path -LiteralPath $exe)) { throw "Missing $exe. Run without -SkipBuild." }

    # --- Layout -----------------------------------------------------------
    foreach ($dir in @($layout, $listingDir, $tempDir)) {
        if (Test-Path -LiteralPath $dir) { Remove-Item -LiteralPath $dir -Recurse -Force }
        New-Item -ItemType Directory -Path $dir | Out-Null
    }
    Copy-Item -LiteralPath $exe -Destination (Join-Path $layout 'NoKey.exe')

    $assets = Join-Path $layout 'Assets'
    Invoke-Tool 'store_assets example' 'cargo' @('run', '--release', '--target-dir', $cargoTarget, '--example', 'store_assets', '--', $assets, $listingDir)

    $template = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'AppxManifest.template.xml') -Raw -Encoding UTF8
    $manifest = $template.
        Replace('__IDENTITY_NAME__', (ConvertTo-XmlText $IdentityName)).
        Replace('__PUBLISHER_DISPLAY_NAME__', (ConvertTo-XmlText $PublisherDisplayName)).
        Replace('__PUBLISHER__', (ConvertTo-XmlText $Publisher)).
        Replace('__VERSION__', $Version).
        Replace('__DISPLAY_NAME__', (ConvertTo-XmlText $DisplayName))
    $manifestPath = Join-Path $layout 'AppxManifest.xml'
    [System.IO.File]::WriteAllText($manifestPath, $manifest, (New-Object System.Text.UTF8Encoding($false)))

    # --- resources.pri ----------------------------------------------------
    $priConfig = Join-Path $tempDir 'priconfig.xml'
    Invoke-Tool 'makepri createconfig' $makepri @('createconfig', '/cf', $priConfig, '/dq', 'en-US', '/pv', '10.0.0', '/o')
    # One resources.pri with every scale: a single (non-bundle) package does not use split PRI files.
    [xml]$priXml = Get-Content -LiteralPath $priConfig -Raw
    $packagingNode = $priXml.SelectSingleNode('//packaging')
    if ($packagingNode) { [void]$packagingNode.ParentNode.RemoveChild($packagingNode) }
    $priXml.Save($priConfig)
    Invoke-Tool 'makepri new' $makepri @('new', '/pr', $layout, '/cf', $priConfig, '/mn', $manifestPath, '/of', (Join-Path $layout 'resources.pri'), '/o')

    # --- Pack -------------------------------------------------------------
    $msix = Join-Path $storeDir "NoKey_${Version}_x64.msix"
    $upload = Join-Path $storeDir "NoKey_${Version}_x64.msixupload"
    foreach ($old in @($msix, $upload)) {
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force }
    }
    Invoke-Tool 'makeappx pack' $makeappx @('pack', '/v', '/h', 'SHA256', '/o', '/d', $layout, '/p', $msix)

    # .msixupload is a zip that contains only the .msix
    $zip = [System.IO.Path]::ChangeExtension($upload, '.zip')
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    Compress-Archive -LiteralPath $msix -DestinationPath $zip
    Move-Item -LiteralPath $zip -Destination $upload

    Remove-Item -LiteralPath $tempDir -Recurse -Force
}
finally {
    Pop-Location
}

Write-Host ''
Write-Host 'Done.'
Write-Host "  MSIX:        $msix"
Write-Host "  MSIX upload: $upload"
Write-Host "  Listing icon (300x300, for the Store listing): $(Join-Path $listingDir 'store-listing-icon-300.png')"
Write-Warning 'Identity values must match Partner Center (Product identity). Defaults are for local testing only.'
