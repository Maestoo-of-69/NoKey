<#
.SYNOPSIS
  OPTIONAL: signs the test .msix with a self-signed certificate so you can
  install it locally. NOT needed for the Microsoft Store (the Store signs the
  package itself).

.DESCRIPTION
  What this script does (all in your CurrentUser scope, no admin needed):
    1. Creates a self-signed code-signing certificate in Cert:\CurrentUser\My
       whose Subject equals -Publisher. The Subject MUST equal the Publisher in
       the package manifest, otherwise signing/installation fails.
    2. Exports the PUBLIC part to target\store\NoKeyTest.cer (no private key,
       no PFX, no password is written to disk).
    3. Signs the .msix with signtool (SHA256), selecting the certificate by
       thumbprint from the store.
    4. Prints the commands YOU must run in an elevated PowerShell to trust the
       certificate and install the package, and how to undo it all.

  What it deliberately does NOT do: it never imports anything into a trusted
  store (that is a security decision left to you) and never installs the app.

  Build the package first with the same -Publisher: build-msix.ps1
#>
[CmdletBinding()]
param(
    [string]$Publisher = 'CN=NoKey Dev',
    [string]$MsixPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$storeDir = Join-Path $projectRoot 'target\store'

if (-not $MsixPath) {
    $found = Get-ChildItem -LiteralPath $storeDir -Filter 'NoKey_*_x64.msix' -ErrorAction SilentlyContinue |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $found) { throw "No NoKey_*_x64.msix in $storeDir. Run build-msix.ps1 first." }
    $MsixPath = $found.FullName
}
if (-not (Test-Path -LiteralPath $MsixPath)) { throw "Package not found: $MsixPath" }

# Newest Windows SDK signtool.
$sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
$signtool = Get-ChildItem -LiteralPath $sdkRoot -Directory |
    Where-Object { $_.Name -match '^\d+\.\d+\.\d+\.\d+$' } |
    Sort-Object { [version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $signtool) { throw "signtool.exe not found under $sdkRoot." }

# 1. Self-signed code-signing certificate (valid 1 year); the private key stays in your store.
$cert = New-SelfSignedCertificate -Type Custom -Subject $Publisher `
    -KeyUsage DigitalSignature -FriendlyName 'NoKey test signing' `
    -CertStoreLocation 'Cert:\CurrentUser\My' `
    -TextExtension @('2.5.29.37={text}1.3.6.1.5.5.7.3.3', '2.5.29.19={text}') `
    -NotAfter (Get-Date).AddYears(1)

# 2. Export the public certificate only.
$cer = Join-Path $storeDir 'NoKeyTest.cer'
Export-Certificate -Cert $cert -FilePath $cer | Out-Null

# 3. Sign, picking the certificate by thumbprint.
& $signtool sign /fd SHA256 /sha1 $cert.Thumbprint $MsixPath
if ($LASTEXITCODE -ne 0) { throw "signtool failed with exit code $LASTEXITCODE." }

Write-Host ''
Write-Host "Signed:      $MsixPath"
Write-Host "Certificate: $cer (thumbprint $($cert.Thumbprint))"
Write-Host ''
Write-Host 'Nothing was trusted or installed. To install, run these yourself in an ELEVATED PowerShell:'
Write-Host ''
Write-Host "  Import-Certificate -FilePath '$cer' -CertStoreLocation Cert:\LocalMachine\TrustedPeople"
Write-Host "  Add-AppxPackage -Path '$MsixPath'"
Write-Host ''
Write-Host 'To remove everything afterwards (elevated PowerShell):'
Write-Host ''
Write-Host '  Get-AppxPackage -Name <IdentityName> | Remove-AppxPackage'
Write-Host "  Get-ChildItem Cert:\LocalMachine\TrustedPeople | Where-Object Thumbprint -eq '$($cert.Thumbprint)' | Remove-Item"
Write-Host "  Get-ChildItem Cert:\CurrentUser\My | Where-Object Thumbprint -eq '$($cert.Thumbprint)' | Remove-Item"
