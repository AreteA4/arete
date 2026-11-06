#!/bin/sh
# Arete CLI (a4) installer. Source: https://github.com/AreteA4/arete/blob/main/docs/public/install.sh
#
# usage: curl -fsSL https://arete.run/install.sh | sh [-s -- [VERSION] [--no-modify-path] [--install-dir DIR] [--json]]
# env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH
#        A4_MANIFEST_BASE_URL, A4_LATEST_URL (test overrides; same names as the a4 binary)
#
# Steps: detect platform, resolve version, download asset + checksums.txt +
# checksums.txt.minisig, verify the minisign signature over checksums.txt
# (signed for release tag a4-cli-v<version>) with the key below, verify the
# asset's sha256 against it, then hand over to `a4 self install`, which copies
# the binary to ~/.local/bin, writes ~/.arete/receipt.json and prints
# `A4_BIN=<path>` as its last-but-one stdout line. Human output goes to stderr;
# stdout is reserved for `a4 self install`.
set -eu

A4_MINISIGN_PUBLIC_KEY="RWRsiwmDW0371BZbcE1IWD6Y8/KIoAArUAp7mpyG6VweJ5rE3Lf3g5qA"
LATEST_URL="${A4_LATEST_URL:-https://docs.arete.run/a4/latest.json}"

log() { printf '%s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# 1. Platform key (<os>-<arch>), same table as the a4 binary and the npm package.
os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Darwin) os=darwin ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*|Windows_NT)
    die "Windows shell detected. Run in PowerShell instead: irm https://arete.run/install.ps1 | iex" ;;
  *) die "unsupported OS '$os' (supported: darwin, linux). Build from source: https://github.com/AreteA4/arete" ;;
esac
case "$arch" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64) arch=x64 ;;
  *) die "unsupported architecture '$arch' (supported: arm64, x64). Build from source: https://github.com/AreteA4/arete" ;;
esac
platform="$os-$arch"
asset="a4-$platform"

# 2. Version: first positional argument -> A4_VERSION -> latest.json.
#    Every other argument is forwarded verbatim to `a4 self install`.
version=""
n=$#
while [ "$n" -gt 0 ]; do
  arg=$1; shift; n=$((n - 1))
  case "$arg" in
    -h|--help)
      log "usage: curl -fsSL https://arete.run/install.sh | sh [-s -- [VERSION] [--no-modify-path] [--install-dir DIR] [--json]]"
      log "env:   A4_VERSION, A4_INSTALL_DIR, A4_NO_MODIFY_PATH"
      exit 0 ;;
    --install-dir)
      [ "$n" -gt 0 ] || die "--install-dir needs a directory argument"
      set -- "$@" "$arg" "$1"; shift; n=$((n - 1)) ;;
    -*) set -- "$@" "$arg" ;;
    *)
      if [ -z "$version" ]; then version=$arg; else set -- "$@" "$arg"; fi ;;
  esac
done
[ -n "$version" ] || version="${A4_VERSION:-}"

have curl || have wget || die "need curl or wget on PATH (e.g. apt-get install -y curl ca-certificates)"

# http_get URL DEST: sets http_status to the HTTP status (000 on transport failure).
http_get() {
  if have curl; then
    http_status=$(curl -sSL --retry 2 -o "$2" -w '%{http_code}' "$1" 2>/dev/null) || http_status="000"
  else
    # wget exits 8 on an HTTP error response (treated as 404) and 4 on a network failure.
    if wget -qO "$2" "$1" 2>/dev/null; then http_status=200; elif [ $? -eq 8 ]; then http_status=404; else http_status=000; fi
  fi
}

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t a4-install)
trap 'rm -rf "$tmp"' EXIT INT TERM HUP

if [ -z "$version" ]; then
  http_get "$LATEST_URL" "$tmp/latest.json"
  [ "$http_status" = 200 ] || die "could not fetch $LATEST_URL (HTTP $http_status). Pass a version: sh -s -- 0.13.0 or set A4_VERSION"
  version=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp/latest.json" | head -n 1)
  [ -n "$version" ] || die "no \"version\" field in $LATEST_URL. Pass a version: sh -s -- 0.13.0 or set A4_VERSION"
fi
version=${version#v}

# 3. Download asset, checksums.txt, checksums.txt.minisig.
if [ -n "${A4_MANIFEST_BASE_URL:-}" ]; then
  base=${A4_MANIFEST_BASE_URL%/}
  case "$base" in
    *"{version}"*) base=$(printf '%s' "$base" | sed "s/{version}/$version/g") ;;
    *) base="$base/a4-cli-v$version" ;;
  esac
else
  base="https://github.com/AreteA4/arete/releases/download/a4-cli-v$version"
fi

log "Installing a4 $version ($platform) from $base"
for file in "$asset" checksums.txt checksums.txt.minisig; do
  dest="$tmp/$file"
  [ "$file" = "$asset" ] && dest="$tmp/a4"
  http_get "$base/$file" "$dest"
  case "$http_status" in
    200) ;;
    404) die "Release $version is still publishing; retry in a few minutes (missing $base/$file)" ;;
    *) die "download of $base/$file failed (HTTP $http_status). Check network/proxy settings and retry" ;;
  esac
done

# 4. minisign signature over checksums.txt, bound to this release's tag, so
#    neither a tampered file nor another release's signed checksums pass.
tag="a4-cli-v$version"
verify_failed() {
  die "signature verification of checksums.txt for $tag failed ($1). Do not use this download; report at https://github.com/AreteA4/arete/issues"
}
# openssl_verify KEY FILE SIG TAG DIR: minisign verification with OpenSSL 3
# (BLAKE2b-512 prehash, Ed25519). 0 verified, 1 rejected, 2 OpenSSL can't.
openssl_verify() {
  d=$5
  openssl pkeyutl -help 2>&1 | grep -q -- -rawin || return 2
  printf '' | openssl dgst -blake2b512 -binary >/dev/null 2>&1 || return 2
  printf '%s' "$1" | openssl base64 -d -A > "$d/pk" 2>/dev/null || return 1
  sed -n 2p "$3" | tr -d '\r' | openssl base64 -d -A > "$d/sig" 2>/dev/null || return 1
  sed -n 4p "$3" | tr -d '\r' | openssl base64 -d -A > "$d/global" 2>/dev/null || return 1
  comment=$(sed -n 3p "$3" | tr -d '\r')
  case "$comment" in "trusted comment: "*) comment=${comment#trusted comment: } ;; *) return 1 ;; esac
  [ "$(wc -c < "$d/pk" | tr -d ' ')" = 42 ] && [ "$(wc -c < "$d/sig" | tr -d ' ')" = 74 ] || return 1
  [ "$(dd if="$d/pk" bs=1 count=2 2>/dev/null)" = Ed ] && [ "$(dd if="$d/sig" bs=1 count=2 2>/dev/null)" = ED ] || return 1
  dd if="$d/pk" bs=1 skip=2 count=8 of="$d/pk.id" 2>/dev/null
  dd if="$d/sig" bs=1 skip=2 count=8 of="$d/sig.id" 2>/dev/null
  cmp -s "$d/pk.id" "$d/sig.id" || return 1
  { printf '\060\052\060\005\006\003\053\145\160\003\041\000'; dd if="$d/pk" bs=1 skip=10 count=32 2>/dev/null; } > "$d/pk.der"
  dd if="$d/sig" bs=1 skip=10 count=64 of="$d/sig.ed" 2>/dev/null
  openssl dgst -blake2b512 -binary "$2" > "$d/digest" || return 1
  openssl pkeyutl -verify -pubin -keyform DER -inkey "$d/pk.der" -rawin -in "$d/digest" -sigfile "$d/sig.ed" >/dev/null 2>&1 || return 1
  { cat "$d/sig.ed"; printf '%s' "$comment"; } > "$d/global.msg"
  openssl pkeyutl -verify -pubin -keyform DER -inkey "$d/pk.der" -rawin -in "$d/global.msg" -sigfile "$d/global" >/dev/null 2>&1 || return 1
  [ "$comment" = "$4" ]
}
if have minisign; then
  comment=$(minisign -VQ -P "$A4_MINISIGN_PUBLIC_KEY" -m "$tmp/checksums.txt" -x "$tmp/checksums.txt.minisig" 2>/dev/null) \
    || verify_failed "minisign rejected it"
  [ "$comment" = "$tag" ] || verify_failed "signed for '$comment'"
  verifier=minisign
else
  mkdir "$tmp/openssl"
  set +e
  openssl_status=2
  have openssl && { openssl_verify "$A4_MINISIGN_PUBLIC_KEY" "$tmp/checksums.txt" "$tmp/checksums.txt.minisig" "$tag" "$tmp/openssl"; openssl_status=$?; }
  set -e
  case "$openssl_status" in
    0) verifier=openssl ;;
    1) verify_failed "openssl rejected it" ;;
    *)
      cat > "$tmp/verify.js" <<'A4_VERIFY_JS'
// minisign verifier: checks checksums.txt.minisig over checksums.txt with the
// embedded release key and requires the trusted comment to be the release tag.
// Pure JavaScript (BigInt), so it runs under node or macOS's
// `osascript -l JavaScript` when neither minisign nor OpenSSL 3 is installed.
// usage: verify.js <public-key-base64> <file> <file.minisig> <trusted-comment>
var M64 = (1n << 64n) - 1n;
function b64(s) {
  var a = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", o = [], v = 0, n = 0;
  s = s.replace(/[\s=]/g, "");
  for (var i = 0; i < s.length; i++) {
    var c = a.indexOf(s[i]);
    if (c < 0) throw new Error("bad base64");
    v = (v << 6) | c; n += 6;
    if (n >= 8) { n -= 8; o.push((v >> n) & 255); }
  }
  return o;
}
function le(b, o, n) { var x = 0n; for (var i = n - 1; i >= 0; i--) x = (x << 8n) | BigInt(b[o + i]); return x; }
function be(b, o) { var x = 0n; for (var i = 0; i < 8; i++) x = (x << 8n) | BigInt(b[o + i]); return x; }
function rotr(x, n) { return ((x >> n) | (x << (64n - n))) & M64; }
var IV = ["6a09e667f3bcc908", "bb67ae8584caa73b", "3c6ef372fe94f82b", "a54ff53a5f1d36f1",
  "510e527fade682d1", "9b05688c2b3e6c1f", "1f83d9abfb41bd6b", "5be0cd19137e2179"].map(function (h) { return BigInt("0x" + h); });
var SIGMA = ["0123456789abcdef", "ea489fd61c02b753", "b8c052fdae367194", "7931dcbe265a40f8",
  "905724afe1bc683d", "2c6a0b834d75fe19", "c51fed4a0763928b", "db7ec13950f4862a", "6fe9b308c2d714a5",
  "a2847615fb9e3cd0"].map(function (r) { return r.split("").map(function (c) { return parseInt(c, 16); }); });
function blake2b(m) {
  var h = IV.slice(); h[0] ^= 0x01010040n;
  var t = 0n, blocks = Math.max(1, Math.ceil(m.length / 128));
  for (var bi = 0; bi < blocks; bi++) {
    var block = m.slice(bi * 128, bi * 128 + 128), last = bi === blocks - 1;
    t += BigInt(block.length);
    while (block.length < 128) block.push(0);
    var w = []; for (var i = 0; i < 16; i++) w.push(le(block, i * 8, 8));
    var v = h.concat(IV); v[12] ^= t & M64; v[13] ^= t >> 64n; if (last) v[14] ^= M64;
    var G = function (a, b, c, d, x, y) {
      v[a] = (v[a] + v[b] + x) & M64; v[d] = rotr(v[d] ^ v[a], 32n);
      v[c] = (v[c] + v[d]) & M64; v[b] = rotr(v[b] ^ v[c], 24n);
      v[a] = (v[a] + v[b] + y) & M64; v[d] = rotr(v[d] ^ v[a], 16n);
      v[c] = (v[c] + v[d]) & M64; v[b] = rotr(v[b] ^ v[c], 63n);
    };
    for (var r = 0; r < 12; r++) {
      var s = SIGMA[r % 10];
      G(0, 4, 8, 12, w[s[0]], w[s[1]]); G(1, 5, 9, 13, w[s[2]], w[s[3]]);
      G(2, 6, 10, 14, w[s[4]], w[s[5]]); G(3, 7, 11, 15, w[s[6]], w[s[7]]);
      G(0, 5, 10, 15, w[s[8]], w[s[9]]); G(1, 6, 11, 12, w[s[10]], w[s[11]]);
      G(2, 7, 8, 13, w[s[12]], w[s[13]]); G(3, 4, 9, 14, w[s[14]], w[s[15]]);
    }
    for (i = 0; i < 8; i++) h[i] ^= v[i] ^ v[i + 8];
  }
  var out = []; for (i = 0; i < 8; i++) for (var j = 0n; j < 64n; j += 8n) out.push(Number((h[i] >> j) & 255n));
  return out;
}
var K512 = [];
(function () {
  // The first 64 bits of the fractional parts of the cube roots of the first 80 primes.
  var primes = [], n = 2;
  while (primes.length < 80) { var p = true; for (var i = 2; i * i <= n; i++) if (n % i === 0) p = false; if (p) primes.push(n); n++; }
  function icbrt(x) { var r = 1n << ((BigInt(x.toString(2).length) + 2n) / 3n); for (;;) { var s = (2n * r + x / (r * r)) / 3n; if (s >= r) break; r = s; } while (r * r * r > x) r--; while ((r + 1n) ** 3n <= x) r++; return r; }
  for (i = 0; i < 80; i++) K512.push(icbrt(BigInt(primes[i]) << 192n) & M64);
})();
var H512 = ["6a09e667f3bcc908", "bb67ae8584caa73b", "3c6ef372fe94f82b", "a54ff53a5f1d36f1",
  "510e527fade682d1", "9b05688c2b3e6c1f", "1f83d9abfb41bd6b", "5be0cd19137e2179"].map(function (h) { return BigInt("0x" + h); });
function sha512(m) {
  var bits = BigInt(m.length) * 8n; m = m.concat([0x80]);
  while (m.length % 128 !== 112) m.push(0);
  for (var i = 15; i >= 0; i--) m.push(Number((bits >> BigInt(i * 8)) & 255n));
  var h = H512.slice();
  for (var o = 0; o < m.length; o += 128) {
    var w = []; for (i = 0; i < 16; i++) w.push(be(m, o + i * 8));
    for (i = 16; i < 80; i++) {
      var s0 = rotr(w[i - 15], 1n) ^ rotr(w[i - 15], 8n) ^ (w[i - 15] >> 7n);
      var s1 = rotr(w[i - 2], 19n) ^ rotr(w[i - 2], 61n) ^ (w[i - 2] >> 6n);
      w.push((w[i - 16] + s0 + w[i - 7] + s1) & M64);
    }
    var a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], k = h[7];
    for (i = 0; i < 80; i++) {
      var t1 = (k + (rotr(e, 14n) ^ rotr(e, 18n) ^ rotr(e, 41n)) + ((e & f) ^ (~e & M64 & g)) + K512[i] + w[i]) & M64;
      var t2 = ((rotr(a, 28n) ^ rotr(a, 34n) ^ rotr(a, 39n)) + ((a & b) ^ (a & c) ^ (b & c))) & M64;
      k = g; g = f; f = e; e = (d + t1) & M64; d = c; c = b; b = a; a = (t1 + t2) & M64;
    }
    h = [a, b, c, d, e, f, g, k].map(function (x, j) { return (x + h[j]) & M64; });
  }
  var out = []; h.forEach(function (x) { for (var j = 56n; j >= 0n; j -= 8n) out.push(Number((x >> j) & 255n)); });
  return out;
}

// Ed25519 verification (RFC 8032 §5.1.7), cofactorless: encode([S]B - [k]A) == R.
var P = (1n << 255n) - 19n, L = (1n << 252n) + 27742317777372353535851937790883648493n;
function md(x) { x %= P; return x < 0n ? x + P : x; }
function pw(b, e) { var r = 1n; b = md(b); while (e > 0n) { if (e & 1n) r = r * b % P; b = b * b % P; e >>= 1n; } return r; }
function inv(x) { return pw(x, P - 2n); }
var D = md(-121665n * inv(121666n)), I = pw(2n, (P - 1n) / 4n);
function add(p, q) {
  var a = md((p[1] - p[0]) * (q[1] - q[0])), b = md((p[1] + p[0]) * (q[1] + q[0]));
  var c = md(2n * p[3] * q[3] * D), d = md(2n * p[2] * q[2]);
  var e = b - a, f = d - c, g = d + c, h = b + a;
  return [md(e * f), md(g * h), md(f * g), md(e * h)];
}
function mul(s, p) { var q = [0n, 1n, 1n, 0n]; while (s > 0n) { if (s & 1n) q = add(q, p); p = add(p, p); s >>= 1n; } return q; }
function decode(b) {
  var y = le(b, 0, 32), sign = y >> 255n; y &= (1n << 255n) - 1n;
  if (y >= P) return null;
  var u = md(y * y - 1n), v = md(D * y * y + 1n), x = pw(u * inv(v), (P + 3n) / 8n);
  if (md(v * x * x - u) !== 0n) x = md(x * I);
  if (md(v * x * x - u) !== 0n) return null;
  if (x === 0n && sign === 1n) return null;
  if ((x & 1n) !== sign) x = P - x;
  return [x, y, 1n, md(x * y)];
}
function encode(p) {
  var zi = inv(p[2]), x = md(p[0] * zi), y = md(p[1] * zi), out = [];
  y |= (x & 1n) << 255n;
  for (var i = 0n; i < 256n; i += 8n) out.push(Number((y >> i) & 255n));
  return out;
}
var BY = md(4n * inv(5n));
var B = decode((function () { var o = []; for (var i = 0n; i < 256n; i += 8n) o.push(Number((BY >> i) & 255n)); return o; })());
function ed25519(pk, sig, msg) {
  var A = decode(pk);
  if (!A || sig.length !== 64) return false;
  var R = sig.slice(0, 32), S = le(sig, 32, 32);
  if (S >= L) return false;
  var k = le(sha512(R.concat(pk, msg)), 0, 64) % L;
  var neg = [md(-A[0]), A[1], A[2], md(-A[3])];
  var check = encode(add(mul(S, B), mul(k, neg)));
  for (var i = 0; i < 32; i++) if (check[i] !== R[i]) return false;
  return true;
}
function utf8(s) { var o = []; s = unescape(encodeURIComponent(s)); for (var i = 0; i < s.length; i++) o.push(s.charCodeAt(i)); return o; }
function eq(a, b) { if (a.length !== b.length) return false; for (var i = 0; i < a.length; i++) if (a[i] !== b[i]) return false; return true; }

// Returns null when the signature is valid for `message` and `expected`, else the reason.
function minisign(publicKey, signatureText, message, expected) {
  var pk = b64(publicKey);
  if (pk.length !== 42 || pk[0] !== 0x45 || pk[1] !== 0x64) return "the public key is malformed";
  var lines = signatureText.replace(/\r/g, "").split("\n");
  var prefix = "trusted comment: ";
  if (lines.length < 4 || lines[2].indexOf(prefix) !== 0) return "the signature file is malformed";
  var sig = b64(lines[1]), comment = lines[2].slice(prefix.length), global = b64(lines[3]);
  if (sig.length !== 74 || sig[0] !== 0x45 || sig[1] !== 0x44) return "the signature is not a prehashed minisign signature";
  if (!eq(sig.slice(2, 10), pk.slice(2, 10))) return "the signature is from a different key";
  if (!ed25519(pk.slice(10), sig.slice(10), blake2b(message))) return "the signature does not match the file";
  if (!ed25519(pk.slice(10), global, sig.slice(10).concat(utf8(comment)))) return "the trusted comment signature is invalid";
  if (comment.trim() !== expected) return "the file was signed for '" + comment.trim() + "', not '" + expected + "'";
  return null;
}
function text(b) { var s = ""; for (var i = 0; i < b.length; i++) s += String.fromCharCode(b[i]); return s; }
function check(argv, read) {
  if (argv.length !== 4) return "usage: verify.js <public-key> <file> <file.minisig> <trusted-comment>";
  return minisign(argv[0], text(read(argv[2])), read(argv[1]), argv[3]);
}
if (typeof module !== "undefined" && require.main !== module) {
  module.exports = { blake2b: blake2b, sha512: sha512, minisign: minisign };
} else if (typeof process !== "undefined") {
  var failure = check(process.argv.slice(2), function (p) { return Array.prototype.slice.call(require("fs").readFileSync(p)); });
  if (failure) { console.error(failure); process.exit(1); }
}
// osascript -l JavaScript calls run(argv).
function run(argv) {
  var failure = check(argv, function (p) {
    var data = $.NSData.dataWithContentsOfFile(p);
    if (data.isNil()) throw new Error("cannot read " + p);
    return b64(ObjC.unwrap(data.base64EncodedStringWithOptions(0)));
  });
  if (failure) throw new Error(failure);
  return "";
}
A4_VERIFY_JS
      if have node; then
        node "$tmp/verify.js" "$A4_MINISIGN_PUBLIC_KEY" "$tmp/checksums.txt" "$tmp/checksums.txt.minisig" "$tag" >&2 \
          || verify_failed "node rejected it"
        verifier=node
      elif [ "$os" = darwin ] && have osascript; then
        osascript -l JavaScript "$tmp/verify.js" "$A4_MINISIGN_PUBLIC_KEY" "$tmp/checksums.txt" "$tmp/checksums.txt.minisig" "$tag" >/dev/null \
          || verify_failed "osascript rejected it"
        verifier=osascript
      else
        die "cannot verify the release signature: install minisign (https://jedisct1.github.io/minisign/), OpenSSL 3 or Node.js and retry, or install with: npx @usearete/a4"
      fi ;;
  esac
fi
log "Verified the minisign signature of checksums.txt for $tag ($verifier)"

# 5. SHA-256 of the asset against the verified checksums.txt.
if have sha256sum; then actual=$(sha256sum "$tmp/a4" | cut -d' ' -f1)
elif have shasum; then actual=$(shasum -a 256 "$tmp/a4" | cut -d' ' -f1)
else die "need sha256sum or shasum on PATH"; fi
expected=$(awk -v a="$asset" '$2 == a || $2 == "*" a { print tolower($1); exit }' "$tmp/checksums.txt")
[ -n "$expected" ] || die "checksums.txt has no entry for $asset; the release may be incomplete, retry in a few minutes"
[ "$actual" = "$expected" ] || die "sha256 mismatch for $asset (expected $expected, got $actual). Retry; if it persists, report at https://github.com/AreteA4/arete/issues"

# 6. Hand over. `exec` would replace this shell and skip the EXIT trap, leaving
#    $tmp behind; `a4 self install` copies the binary out, so run it in the
#    foreground, clean up, and exit with its status.
chmod +x "$tmp/a4"
set +e
"$tmp/a4" self install --source sh --checksums "$tmp/checksums.txt" --signature "$tmp/checksums.txt.minisig" "$@"
status=$?
set -e
[ "$status" -ne 126 ] || log "hint: $tmp is not executable (noexec mount?). Retry with TMPDIR=\$HOME/.cache"
exit "$status"
