#!/usr/bin/env bash
# Bootstrap a new npm package with the existing token, then prefer OIDC.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
package_dir="${1:-$root/typescript/protocol}"
dry_run="${2:-}"
[[ -z "$dry_run" || "$dry_run" == '--dry-run' ]] || { echo 'Expected --dry-run' >&2; exit 2; }
node "$root/scripts/check-protocol-release-dependency.mjs" "$package_dir"
hash_version="$(node -e 'const fs = require("node:fs"); const path = require("node:path"); console.log(JSON.parse(fs.readFileSync(path.resolve(process.argv[1], "../hash/package.json"), "utf8")).version)' "$package_dir")"
hash_status="$(curl --silent --show-error --retry 3 --output /dev/null --write-out '%{http_code}' "https://registry.npmjs.org/@usearete%2fhash/$hash_version")"
[[ "$hash_status" == 200 ]] || { echo "Required hash release is unavailable (HTTP $hash_status); refusing to publish protocol." >&2; exit 1; }
status="$(curl --silent --show-error --retry 3 --output /dev/null --write-out '%{http_code}' 'https://registry.npmjs.org/@usearete%2fprotocol')"
case "$status" in
  200)
    env -u NPM_TOKEN -u NODE_AUTH_TOKEN "$root/scripts/publish-npm-package.sh" "$package_dir" "$dry_run"
    ;;
  404)
    if [[ -z "${NPM_TOKEN:-}" ]]; then
      echo 'First protocol publish requires the repository NPM_TOKEN secret with package creation access. Configure it before releasing.' >&2
      exit 1
    fi
    NODE_AUTH_TOKEN="$NPM_TOKEN" "$root/scripts/publish-npm-package.sh" "$package_dir" "$dry_run"
    ;;
  *)
    echo "Cannot determine protocol package existence (HTTP $status); refusing to publish." >&2
    exit 1
    ;;
esac
