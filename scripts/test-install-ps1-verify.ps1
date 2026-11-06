# Test the release signature check embedded in docs/public/install.ps1: it
# accepts the fixture signature for its tag and rejects another tag, another
# key, another key under the release key id, a tampered file and a forged
# trusted comment. Runs under Windows PowerShell 5.1 and PowerShell 7.
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$fixtures = Join-Path $root 'cli/tests/fixtures/selfhost'

$installer = Get-Content -Raw (Join-Path $root 'docs/public/install.ps1')
$match = [regex]::Match($installer, "(?s)\`$A4_MINISIGN_VERIFIER = @'\r?\n(.*?)\r?\n'@")
if (-not $match.Success) { throw 'no $A4_MINISIGN_VERIFIER here-string in install.ps1' }
$references = @()
if ($PSVersionTable.PSEdition -ne 'Core') { $references = @('System.Numerics') }
Add-Type -TypeDefinition $match.Groups[1].Value -ReferencedAssemblies $references

$key = (Get-Content (Join-Path $fixtures 'test-minisign.pub'))[1]
$tag = 'a4-cli-v0.0.0-test'
$checksums = [IO.File]::ReadAllBytes((Join-Path $fixtures 'checksums.txt'))
$good = [IO.File]::ReadAllText((Join-Path $fixtures 'checksums.txt.minisig'))
$wrongKey = [IO.File]::ReadAllText((Join-Path $fixtures 'checksums.txt.wrong-key.minisig'))
$tampered = [byte[]]($checksums + [byte]0x0a)
$forged = $good -replace 'trusted comment: .*', 'trusted comment: a4-cli-v9.9.9'
$lines = $wrongKey -split "`n"
$sig = [Convert]::FromBase64String($lines[1])
[Array]::Copy([Convert]::FromBase64String($key), 2, $sig, 2, 8)
$lines[1] = [Convert]::ToBase64String($sig)
$relabelled = $lines -join "`n"

$cases = @(
  @('accepts the signature for its tag', $checksums, $good, $tag, $true),
  @('rejects another tag', $checksums, $good, 'a4-cli-v0.0.1', $false),
  @('rejects another key', $checksums, $wrongKey, $tag, $false),
  @('rejects another key under the release key id', $checksums, $relabelled, $tag, $false),
  @('rejects a tampered file', $tampered, $good, $tag, $false),
  @('rejects a forged trusted comment', $checksums, $forged, 'a4-cli-v9.9.9', $false)
)
$failures = 0
foreach ($case in $cases) {
  $failure = [A4Minisign]::Verify($key, $case[1], $case[2], $case[3])
  if (($null -eq $failure) -eq $case[4]) {
    Write-Output "ok   install.ps1 verifier $($case[0])"
  } else {
    Write-Output "FAIL install.ps1 verifier $($case[0]): $failure"
    $failures++
  }
}
if ($failures -ne 0) { throw "$failures check(s) failed" }
Write-Output "All install.ps1 signature checks passed ($($PSVersionTable.PSEdition) $($PSVersionTable.PSVersion))"
