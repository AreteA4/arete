#!/usr/bin/env bash

set -euo pipefail

PACKAGE_DIR="${1:-}"
DRY_RUN="${2:-}"

if [[ -z "$PACKAGE_DIR" || ( -n "$DRY_RUN" && "$DRY_RUN" != "--dry-run" ) ]]; then
  echo "Usage: $0 <package-directory> [--dry-run]"
  exit 1
fi

PACKAGE_DIR="$(cd "$PACKAGE_DIR" && pwd)"
PACKAGE_NAME="$(node -p "require('$PACKAGE_DIR/package.json').name")"
PACKAGE_VERSION="$(node -p "require('$PACKAGE_DIR/package.json').version")"
ENCODED_NAME="${PACKAGE_NAME//@/%40}"
ENCODED_NAME="${ENCODED_NAME//\//%2F}"
RESPONSE_FILE="$(mktemp "${TMPDIR:-/tmp}/npm-registry-response.XXXXXX")"
trap 'rm -f "$RESPONSE_FILE"' EXIT

using_trusted_publishing() {
  [[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" \
    && -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" \
    && -z "${NODE_AUTH_TOKEN:-}" \
    && -z "${NPM_TOKEN:-}" ]]
}

STATUS="$(curl --silent --show-error --retry 3 --retry-all-errors \
  --user-agent 'arete-release-workflow (https://github.com/AreteA4/arete)' \
  --output "$RESPONSE_FILE" \
  --write-out '%{http_code}' \
  "https://registry.npmjs.org/${ENCODED_NAME}/${PACKAGE_VERSION}")"

case "$STATUS" in
  200)
    echo "Skipping $PACKAGE_NAME@$PACKAGE_VERSION; already published"
    exit 0
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
    npm publish --access public --provenance --dry-run
  else
    npm publish --access public --provenance
  fi
)
