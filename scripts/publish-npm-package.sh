#!/usr/bin/env bash

set -euo pipefail

# With --tarball, the package directory supplies the expected name and
# version and the packed tarball (built in a job without publishing
# credentials) is what npm publishes. npm never runs lifecycle scripts here.
usage() {
  echo "Usage: $0 <package-directory> [--dry-run] [--tarball <file.tgz>]"
  exit 1
}

PACKAGE_DIR="${1:-}"
[[ -n "$PACKAGE_DIR" ]] || usage
shift
DRY_RUN=""
TARBALL=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN="--dry-run"; shift ;;
    --tarball) [[ -n "${2:-}" ]] || usage; TARBALL="$2"; shift 2 ;;
    # Callers pass an empty dry-run argument when publishing for real.
    "") shift ;;
    *) usage ;;
  esac
done

PACKAGE_DIR="$(cd "$PACKAGE_DIR" && pwd)"
PACKAGE_NAME="$(node -p "require('$PACKAGE_DIR/package.json').name")"
PACKAGE_VERSION="$(node -p "require('$PACKAGE_DIR/package.json').version")"
PUBLISH_SPEC="$PACKAGE_DIR"
if [[ -n "$TARBALL" ]]; then
  if [[ ! -f "$TARBALL" ]]; then
    echo "Tarball $TARBALL does not exist"
    exit 1
  fi
  TARBALL="$(cd "$(dirname "$TARBALL")" && pwd)/$(basename "$TARBALL")"
  TARBALL_ID="$(tar -xzOf "$TARBALL" package/package.json | node -e '
    let input = "";
    process.stdin.on("data", (chunk) => { input += chunk; });
    process.stdin.on("end", () => {
      const pkg = JSON.parse(input);
      process.stdout.write(`${pkg.name}@${pkg.version}`);
    });
  ')"
  if [[ "$TARBALL_ID" != "$PACKAGE_NAME@$PACKAGE_VERSION" ]]; then
    echo "Tarball $TARBALL contains $TARBALL_ID, expected $PACKAGE_NAME@$PACKAGE_VERSION"
    exit 1
  fi
  PUBLISH_SPEC="$TARBALL"
fi
ENCODED_NAME="${PACKAGE_NAME//@/%40}"
ENCODED_NAME="${ENCODED_NAME//\//%2F}"
RESPONSE_FILE="$(mktemp "${TMPDIR:-/tmp}/npm-registry-response.XXXXXX")"
trap 'rm -f "$RESPONSE_FILE"' EXIT

using_trusted_publishing() {
  local node_auth_token="${NODE_AUTH_TOKEN:-}"

  # setup-node exports this placeholder when registry-url is set without a token.
  [[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" \
    && -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" \
    && -z "${NPM_TOKEN:-}" \
    && ( -z "$node_auth_token" || "$node_auth_token" == "XXXXX-XXXXX-XXXXX-XXXXX" ) ]]
}

STATUS="$(curl --silent --show-error --retry 3 --retry-all-errors \
  --user-agent 'arete-release-workflow (https://github.com/AreteA4/arete)' \
  --output "$RESPONSE_FILE" \
  --write-out '%{http_code}' \
  "https://registry.npmjs.org/${ENCODED_NAME}/${PACKAGE_VERSION}")"

case "$STATUS" in
  200)
    echo "$PACKAGE_NAME@$PACKAGE_VERSION is already published; refusing to reuse a version for new source"
    exit 1
    ;;
  404)
    ;;
  *)
    echo "Unable to check $PACKAGE_NAME@$PACKAGE_VERSION on npm (HTTP $STATUS)"
    cat "$RESPONSE_FILE"
    exit 1
    ;;
esac

if using_trusted_publishing; then
  echo "Publishing $PACKAGE_NAME@$PACKAGE_VERSION with npm trusted publishing"
elif NPM_ACCOUNT="$(npm whoami 2>/dev/null)"; then
  echo "Publishing $PACKAGE_NAME@$PACKAGE_VERSION as $NPM_ACCOUNT"
elif [[ "$DRY_RUN" == "--dry-run" ]]; then
  echo "Publishing $PACKAGE_NAME@$PACKAGE_VERSION (unauthenticated dry run)"
else
  echo "npm authentication is not configured"
  exit 1
fi

(
  cd "$PACKAGE_DIR"
  if [[ "$DRY_RUN" == "--dry-run" ]]; then
    npm publish "$PUBLISH_SPEC" --access public --provenance --ignore-scripts --dry-run
  else
    npm publish "$PUBLISH_SPEC" --access public --provenance --ignore-scripts
  fi
)
