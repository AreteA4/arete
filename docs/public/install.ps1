#Requires -Version 5.1
<#
Arete CLI (a4) installer for Windows.
Source: https://github.com/AreteA4/arete/blob/main/docs/public/install.ps1

usage: irm https://arete.run/install.ps1 | iex
       & ([scriptblock]::Create((irm https://arete.run/install.ps1))) [-Version x.y.z] [-InstallDir DIR] [-NoModifyPath] [-Json]
env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH
       A4_MANIFEST_BASE_URL, A4_LATEST_URL (test overrides; same names as the a4 binary)

Steps: detect platform, resolve version, download asset + checksums.txt +
checksums.txt.minisig, verify SHA-256 (and the minisign signature when
minisign.exe is on PATH), then hand over to `a4 self install`, which copies the
binary to %USERPROFILE%\.local\bin, writes %USERPROFILE%\.arete\receipt.json,
updates the user PATH and prints `A4_BIN=<path>` as its last-but-one stdout
line. Human output goes to stderr; stdout is reserved for `a4 self install`.
Errors are thrown (not `exit`) so `irm | iex` never closes the caller's shell.
#>
[CmdletBinding()]
param(
  [string]$Version = '',
  [string]$InstallDir = '',
  [switch]$NoModifyPath,
  [switch]$Json
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$MINISIGN_PUBLIC_KEY = 'RWRsiwmDW0371BZbcE1IWD6Y8/KIoAArUAp7mpyG6VweJ5rE3Lf3g5qA'
$LatestUrl = if ($env:A4_LATEST_URL) { $env:A4_LATEST_URL } else { 'https://docs.arete.run/a4/latest.json' }

function Write-Log([string]$Message) { [Console]::Error.WriteLine($Message) }

function Get-HttpStatus($ErrorRecord) {
  try {
    $response = $ErrorRecord.Exception.Response
    if ($null -ne $response) { return [int]$response.StatusCode }
  } catch { }
  return 0
}

function Get-Download([string]$Url, [string]$Dest, [string]$ReleaseVersion) {
  try {
    Invoke-WebRequest -Uri $Url -OutFile $Dest -UseBasicParsing
  } catch {
    $status = Get-HttpStatus $_
    if ($status -eq 404) {
      throw "Release $ReleaseVersion is still publishing; retry in a few minutes (missing $Url)"
    }
    throw "download of $Url failed (HTTP $status): $($_.Exception.Message). Check network/proxy settings and retry"
  }
}

# 1. Platform key. Only win32-x64 is built; Windows on ARM runs it under emulation.
if ($PSVersionTable.PSVersion.Major -ge 6 -and -not $IsWindows) {
  throw 'install.ps1 is for Windows. On macOS/Linux run: curl -fsSL https://arete.run/install.sh | sh'
}
$cpu = $env:PROCESSOR_ARCHITEW6432
if (-not $cpu) { $cpu = $env:PROCESSOR_ARCHITECTURE }
switch ($cpu) {
  'AMD64' { }
  'ARM64' { Write-Log 'note: Windows on ARM detected; installing the x64 build (runs under emulation)' }
  default { throw "unsupported architecture '$cpu' (supported: AMD64, ARM64). Build from source: https://github.com/AreteA4/arete" }
}
$platform = 'win32-x64'
$asset = "a4-$platform.exe"

# 2. Version: -Version -> A4_VERSION -> latest.json.
if (-not $Version -and $env:A4_VERSION) { $Version = $env:A4_VERSION }
if (-not $Version) {
  try {
    $latest = Invoke-WebRequest -Uri $LatestUrl -UseBasicParsing
  } catch {
    throw "could not fetch $LatestUrl (HTTP $(Get-HttpStatus $_)). Pass -Version x.y.z or set A4_VERSION"
  }
  $match = [regex]::Match([string]$latest.Content, '"version"\s*:\s*"([^"]+)"')
  if (-not $match.Success) { throw "no ""version"" field in $LatestUrl. Pass -Version x.y.z or set A4_VERSION" }
  $Version = $match.Groups[1].Value
}
$Version = $Version.TrimStart('v')

# 3. Download asset, checksums.txt, checksums.txt.minisig into a temp dir.
if ($env:A4_MANIFEST_BASE_URL) {
  $base = $env:A4_MANIFEST_BASE_URL.TrimEnd('/')
  if ($base.Contains('{version}')) { $base = $base.Replace('{version}', $Version) } else { $base = "$base/a4-cli-v$Version" }
} else {
  $base = "https://github.com/AreteA4/arete/releases/download/a4-cli-v$Version"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("a4-install-" + [IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $exe = Join-Path $tmp 'a4.exe'
  $checksums = Join-Path $tmp 'checksums.txt'
  $signature = Join-Path $tmp 'checksums.txt.minisig'
  Write-Log "Installing a4 $Version ($platform) from $base"
  Get-Download "$base/$asset" $exe $Version
  Get-Download "$base/checksums.txt" $checksums $Version
  Get-Download "$base/checksums.txt.minisig" $signature $Version

  # 4. SHA-256 of the asset against checksums.txt.
  $expected = $null
  foreach ($line in Get-Content $checksums) {
    $m = [regex]::Match($line, '^([0-9a-fA-F]{64})\s+\*?(.+?)\s*$')
    if ($m.Success -and $m.Groups[2].Value -eq $asset) { $expected = $m.Groups[1].Value.ToLowerInvariant(); break }
  }
  if (-not $expected) { throw "checksums.txt has no entry for $asset; the release may be incomplete, retry in a few minutes" }
  $actual = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    throw "sha256 mismatch for $asset (expected $expected, got $actual). Retry; if it persists, report at https://github.com/AreteA4/arete/issues"
  }

  # 5. minisign signature over checksums.txt (a4 self install always verifies it too).
  $minisign = Get-Command 'minisign.exe' -ErrorAction SilentlyContinue
  if ($minisign) {
    & $minisign.Source -Vq -P $MINISIGN_PUBLIC_KEY -m $checksums -x $signature 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
      throw 'minisign signature verification of checksums.txt failed. Do not use this download; report at https://github.com/AreteA4/arete/issues'
    }
  } else {
    Write-Log 'note: minisign not found; signature will be verified by a4 self install'
  }

  # 6. Hand over to the downloaded binary. It copies itself out of $tmp.
  $selfInstallArgs = @('self', 'install', '--source', 'ps1', '--checksums', $checksums, '--signature', $signature)
  if ($InstallDir) { $selfInstallArgs += @('--install-dir', $InstallDir) }
  if ($NoModifyPath) { $selfInstallArgs += '--no-modify-path' }
  if ($Json) { $selfInstallArgs += '--json' }
  & $exe @selfInstallArgs
  if ($LASTEXITCODE -ne 0) { throw "a4 self install failed with exit code $LASTEXITCODE" }
} finally {
  Remove-Item -Recurse -Force -Path $tmp -ErrorAction SilentlyContinue
}
