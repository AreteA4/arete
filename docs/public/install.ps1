#Requires -Version 5.1
<#
Arete CLI (a4) installer for Windows.
Source: https://github.com/AreteA4/arete/blob/main/docs/public/install.ps1

usage: irm https://arete.run/install.ps1 | iex
       & ([scriptblock]::Create((irm https://arete.run/install.ps1))) [-Version x.y.z] [-InstallDir DIR] [-NoModifyPath] [-Json]
env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH
       A4_MANIFEST_BASE_URL, A4_LATEST_URL (test overrides; same names as the a4 binary)

Steps: detect platform, resolve version, download asset + checksums.txt +
checksums.txt.minisig, verify the minisign signature over checksums.txt (signed
for release tag a4-cli-v<version>) with the key below, verify the asset's
SHA-256 against it, then hand over to `a4 self install`, which copies the
binary to %USERPROFILE%\.local\bin, writes %USERPROFILE%\.arete\receipt.json,
updates the user PATH and prints `A4_BIN=<path>` as its last-but-one stdout
line. Human output goes to stderr; stdout is reserved for `a4 self install`.
Errors are thrown (not `exit`) so `irm | iex` never closes the caller's shell.
The downloaded binary never runs unverified: the signature is checked by the
verifier embedded below, with no external tools.
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

# minisign verification in C# (no minisign.exe needed): BLAKE2b-512 prehash,
# Ed25519 (RFC 8032) and the trusted comment checked against the release tag.
$A4_MINISIGN_VERIFIER = @'
using System;
using System.Numerics;
using System.Security.Cryptography;
using System.Text;

// minisign verification: the prehashed (BLAKE2b-512) Ed25519 signature over
// the file, the global signature over the trusted comment, and the trusted
// comment against the expected release tag. Returns null when valid.
public static class A4Minisign
{
    static readonly BigInteger P = BigInteger.Pow(2, 255) - 19;
    static readonly BigInteger L = BigInteger.Pow(2, 252) + BigInteger.Parse("27742317777372353535851937790883648493");
    static readonly BigInteger D = Md(-121665 * Inv(121666));
    static readonly BigInteger I = BigInteger.ModPow(2, (P - 1) / 4, P);
    static readonly BigInteger[] B = Decode(Encode(new BigInteger[] { 0, Md(4 * Inv(5)), 1, 0 }, true));

    public static string Verify(string publicKey, byte[] message, string signatureText, string expected)
    {
        byte[] pk = Convert.FromBase64String(publicKey);
        if (pk.Length != 42 || pk[0] != 0x45 || pk[1] != 0x64) return "the public key is malformed";
        string[] lines = signatureText.Replace("\r", "").Split('\n');
        const string prefix = "trusted comment: ";
        if (lines.Length < 4 || !lines[2].StartsWith(prefix, StringComparison.Ordinal)) return "the signature file is malformed";
        byte[] sig, global;
        try
        {
            sig = Convert.FromBase64String(lines[1].Trim());
            global = Convert.FromBase64String(lines[3].Trim());
        }
        catch (FormatException)
        {
            return "the signature file is malformed";
        }
        string comment = lines[2].Substring(prefix.Length);
        if (sig.Length != 74 || sig[0] != 0x45 || sig[1] != 0x44) return "the signature is not a prehashed minisign signature";
        for (int i = 2; i < 10; i++) if (sig[i] != pk[i]) return "the signature is from a different key";
        byte[] key = Slice(pk, 10, 32), ed = Slice(sig, 10, 64);
        if (!Ed25519(key, ed, Blake2b(message))) return "the signature does not match the file";
        byte[] commentBytes = Encoding.UTF8.GetBytes(comment);
        byte[] globalMessage = new byte[64 + commentBytes.Length];
        Buffer.BlockCopy(ed, 0, globalMessage, 0, 64);
        Buffer.BlockCopy(commentBytes, 0, globalMessage, 64, commentBytes.Length);
        if (!Ed25519(key, global, globalMessage)) return "the trusted comment signature is invalid";
        if (comment.Trim() != expected) return "the file was signed for '" + comment.Trim() + "', not '" + expected + "'";
        return null;
    }

    static byte[] Slice(byte[] b, int offset, int length)
    {
        byte[] o = new byte[length];
        Buffer.BlockCopy(b, offset, o, 0, length);
        return o;
    }

    static readonly ulong[] IV = {
        0x6a09e667f3bcc908UL, 0xbb67ae8584caa73bUL, 0x3c6ef372fe94f82bUL, 0xa54ff53a5f1d36f1UL,
        0x510e527fade682d1UL, 0x9b05688c2b3e6c1fUL, 0x1f83d9abfb41bd6bUL, 0x5be0cd19137e2179UL };
    static readonly string[] Sigma = {
        "0123456789abcdef", "ea489fd61c02b753", "b8c052fdae367194", "7931dcbe265a40f8", "905724afe1bc683d",
        "2c6a0b834d75fe19", "c51fed4a0763928b", "db7ec13950f4862a", "6fe9b308c2d714a5", "a2847615fb9e3cd0" };

    static ulong Rotr(ulong x, int n) { return (x >> n) | (x << (64 - n)); }

    public static byte[] Blake2b(byte[] m)
    {
        ulong[] h = (ulong[])IV.Clone();
        h[0] ^= 0x01010040UL;
        int blocks = Math.Max(1, (m.Length + 127) / 128);
        ulong t = 0;
        for (int bi = 0; bi < blocks; bi++)
        {
            byte[] block = new byte[128];
            int length = Math.Min(128, m.Length - bi * 128);
            if (length > 0) Buffer.BlockCopy(m, bi * 128, block, 0, length);
            t += (ulong)Math.Max(length, 0);
            ulong[] w = new ulong[16];
            for (int i = 0; i < 16; i++) w[i] = BitConverterLe(block, i * 8);
            ulong[] v = new ulong[16];
            for (int i = 0; i < 8; i++) { v[i] = h[i]; v[i + 8] = IV[i]; }
            v[12] ^= t;
            if (bi == blocks - 1) v[14] = ~v[14];
            for (int r = 0; r < 12; r++)
            {
                string s = Sigma[r % 10];
                int[] x = new int[16];
                for (int i = 0; i < 16; i++) x[i] = Convert.ToInt32(s.Substring(i, 1), 16);
                G(v, 0, 4, 8, 12, w[x[0]], w[x[1]]); G(v, 1, 5, 9, 13, w[x[2]], w[x[3]]);
                G(v, 2, 6, 10, 14, w[x[4]], w[x[5]]); G(v, 3, 7, 11, 15, w[x[6]], w[x[7]]);
                G(v, 0, 5, 10, 15, w[x[8]], w[x[9]]); G(v, 1, 6, 11, 12, w[x[10]], w[x[11]]);
                G(v, 2, 7, 8, 13, w[x[12]], w[x[13]]); G(v, 3, 4, 9, 14, w[x[14]], w[x[15]]);
            }
            for (int i = 0; i < 8; i++) h[i] ^= v[i] ^ v[i + 8];
        }
        byte[] o = new byte[64];
        for (int i = 0; i < 8; i++) for (int j = 0; j < 8; j++) o[i * 8 + j] = (byte)(h[i] >> (8 * j));
        return o;
    }

    static ulong BitConverterLe(byte[] b, int offset)
    {
        ulong x = 0;
        for (int i = 7; i >= 0; i--) x = (x << 8) | b[offset + i];
        return x;
    }

    static void G(ulong[] v, int a, int b, int c, int d, ulong x, ulong y)
    {
        unchecked
        {
            v[a] = v[a] + v[b] + x; v[d] = Rotr(v[d] ^ v[a], 32);
            v[c] = v[c] + v[d]; v[b] = Rotr(v[b] ^ v[c], 24);
            v[a] = v[a] + v[b] + y; v[d] = Rotr(v[d] ^ v[a], 16);
            v[c] = v[c] + v[d]; v[b] = Rotr(v[b] ^ v[c], 63);
        }
    }

    // Ed25519 verification (RFC 8032 section 5.1.7), cofactorless:
    // encode([S]B - [k]A) == R.
    static BigInteger Md(BigInteger x) { x %= P; return x.Sign < 0 ? x + P : x; }
    static BigInteger Inv(BigInteger x) { return BigInteger.ModPow(Md(x), P - 2, P); }

    static BigInteger Le(byte[] b, int offset, int length)
    {
        byte[] o = new byte[length + 1];
        Buffer.BlockCopy(b, offset, o, 0, length);
        return new BigInteger(o);
    }

    static BigInteger[] Add(BigInteger[] p, BigInteger[] q)
    {
        BigInteger a = Md((p[1] - p[0]) * (q[1] - q[0])), b = Md((p[1] + p[0]) * (q[1] + q[0]));
        BigInteger c = Md(2 * p[3] * q[3] * D), d = Md(2 * p[2] * q[2]);
        BigInteger e = b - a, f = d - c, g = d + c, h = b + a;
        return new BigInteger[] { Md(e * f), Md(g * h), Md(f * g), Md(e * h) };
    }

    static BigInteger[] Mul(BigInteger s, BigInteger[] p)
    {
        BigInteger[] q = { 0, 1, 1, 0 };
        while (s.Sign > 0)
        {
            if (!s.IsEven) q = Add(q, p);
            p = Add(p, p);
            s >>= 1;
        }
        return q;
    }

    static BigInteger[] Decode(byte[] b)
    {
        BigInteger y = Le(b, 0, 32);
        int sign = (b[31] >> 7) & 1;
        y &= BigInteger.Pow(2, 255) - 1;
        if (y >= P) return null;
        BigInteger u = Md(y * y - 1), v = Md(D * y * y + 1);
        BigInteger x = BigInteger.ModPow(Md(u * Inv(v)), (P + 3) / 8, P);
        if (Md(v * x * x - u) != 0) x = Md(x * I);
        if (Md(v * x * x - u) != 0) return null;
        if (x.IsZero && sign == 1) return null;
        if ((int)(x & 1) != sign) x = P - x;
        return new BigInteger[] { x, y, 1, Md(x * y) };
    }

    // Encode a point; `yOnly` encodes (0, y) with a positive x for the base point.
    static byte[] Encode(BigInteger[] p, bool yOnly)
    {
        BigInteger zi = Inv(p[2]), x = Md(p[0] * zi), y = Md(p[1] * zi);
        if (!yOnly && !x.IsEven) y |= BigInteger.Pow(2, 255);
        byte[] raw = y.ToByteArray(), o = new byte[32];
        Buffer.BlockCopy(raw, 0, o, 0, Math.Min(32, raw.Length));
        return o;
    }

    static bool Ed25519(byte[] pk, byte[] sig, byte[] msg)
    {
        BigInteger[] a = Decode(pk);
        if (a == null || sig.Length != 64) return false;
        byte[] r = Slice(sig, 0, 32);
        BigInteger s = Le(sig, 32, 32);
        if (s >= L) return false;
        byte[] input = new byte[64 + msg.Length];
        Buffer.BlockCopy(r, 0, input, 0, 32);
        Buffer.BlockCopy(pk, 0, input, 32, 32);
        Buffer.BlockCopy(msg, 0, input, 64, msg.Length);
        byte[] digest;
        using (SHA512 sha = SHA512.Create()) digest = sha.ComputeHash(input);
        BigInteger k = Le(digest, 0, 64) % L;
        BigInteger[] neg = { Md(-a[0]), a[1], a[2], Md(-a[3]) };
        byte[] check = Encode(Add(Mul(s, B), Mul(k, neg)), false);
        for (int i = 0; i < 32; i++) if (check[i] != r[i]) return false;
        return true;
    }
}
'@

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

  # 4. minisign signature over checksums.txt, bound to this release's tag, so
  #    neither a tampered file nor another release's signed checksums pass.
  $tag = "a4-cli-v$Version"
  if (-not ('A4Minisign' -as [type])) {
    $references = @()
    if ($PSVersionTable.PSEdition -ne 'Core') { $references = @('System.Numerics') }
    try {
      Add-Type -TypeDefinition $A4_MINISIGN_VERIFIER -ReferencedAssemblies $references
    } catch {
      throw "cannot load the signature verifier ($($_.Exception.Message)); a4 is not installed. Install with: npx @usearete/a4"
    }
  }
  $failure = [A4Minisign]::Verify($MINISIGN_PUBLIC_KEY, [IO.File]::ReadAllBytes($checksums), [IO.File]::ReadAllText($signature), $tag)
  if ($null -ne $failure) {
    throw "signature verification of checksums.txt for $tag failed ($failure). Do not use this download; report at https://github.com/AreteA4/arete/issues"
  }
  Write-Log "Verified the minisign signature of checksums.txt for $tag"

  # 5. SHA-256 of the asset against the verified checksums.txt.
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
