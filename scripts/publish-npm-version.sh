#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Publish the Arete npm packages for a specific version from the current checkout.

Usage:
  ./scripts/publish-npm-version.sh <version> [--dry-run]

Examples:
  ./scripts/publish-npm-version.sh 0.1.3
  NPM_TOKEN=xxxx ./scripts/publish-npm-version.sh 0.1.3
  ./scripts/publish-npm-version.sh 0.1.3 --dry-run

Notes:
  - Copies only the npm package directories into a temporary repo snapshot.
  - Rewrites the staged package metadata to the target version.
  - Publishes in dependency order: sdk -> react -> adapters -> a4 -> mcp.
  - Skips packages that are already published at that exact version.
EOF
}

VERSION="${1:-}"
DRY_RUN="${2:-}"

if [[ -z "$VERSION" ]]; then
  usage
  exit 1
fi

if [[ -n "$DRY_RUN" && "$DRY_RUN" != "--dry-run" ]]; then
  usage
  exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-npm-publish-${VERSION}.XXXXXX")"

cleanup() {
  rm -rf "$STAGING_DIR"
}

trap cleanup EXIT

if [[ -n "${NPM_TOKEN:-}" ]]; then
  cat >"$STAGING_DIR/.npmrc" <<EOF
//registry.npmjs.org/:_authToken=${NPM_TOKEN}
EOF
  export NPM_CONFIG_USERCONFIG="$STAGING_DIR/.npmrc"
fi

copy_package_dir() {
  local source_dir="$1"
  local destination_dir="$2"

  mkdir -p "$(dirname "$destination_dir")"
  rsync -a \
    --exclude node_modules \
    --exclude dist \
    --exclude '*.tsbuildinfo' \
    "$source_dir/" "$destination_dir/"
}

rewrite_package_json() {
  local file="$1"
  local version="$2"
  local sdk_dependency="${3:-}"

  node - "$file" "$version" "$sdk_dependency" <<'EOF'
const fs = require('node:fs');

const [file, version, sdkDependency] = process.argv.slice(2);
const pkg = JSON.parse(fs.readFileSync(file, 'utf8'));

pkg.version = version;

for (const section of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies']) {
  if (sdkDependency && pkg[section]?.['@usearete/sdk']) {
    pkg[section]['@usearete/sdk'] = sdkDependency;
  }
}

fs.writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
EOF
}

rewrite_package_lock() {
  local file="$1"
  local version="$2"
  local sdk_dependency="${3:-}"

  if [[ ! -f "$file" ]]; then
    return
  fi

  node - "$file" "$version" "$sdk_dependency" <<'EOF'
const fs = require('node:fs');

const [file, version, sdkDependency] = process.argv.slice(2);
const lock = JSON.parse(fs.readFileSync(file, 'utf8'));

lock.version = version;

if (lock.packages && lock.packages['']) {
  lock.packages[''].version = version;

  for (const section of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies']) {
    if (sdkDependency && lock.packages[''][section]?.['@usearete/sdk']) {
      lock.packages[''][section]['@usearete/sdk'] = sdkDependency;
    }
  }
}

fs.writeFileSync(file, `${JSON.stringify(lock, null, 2)}\n`);
EOF
}

publish_package() {
  local package_dir="$1"
  local package_name="$2"
  local install_command="${3:-}"

  local staged_version
  staged_version="$(node -p "require('$package_dir/package.json').version")"

  if [[ "$staged_version" != "$VERSION" ]]; then
    echo "Expected $package_name to be staged as $VERSION, found $staged_version"
    exit 1
  fi

  local encoded_name
  local response_file
  local status
  encoded_name="${package_name//@/%40}"
  encoded_name="${encoded_name//\//%2F}"
  response_file="$(mktemp "$STAGING_DIR/npm-registry-response.XXXXXX")"
  status="$(curl --silent --show-error --retry 3 --retry-all-errors \
    --user-agent 'arete-release-workflow (https://github.com/AreteA4/arete)' \
    --output "$response_file" \
    --write-out '%{http_code}' \
    "https://registry.npmjs.org/${encoded_name}/${VERSION}")"

  case "$status" in
    200)
      echo "Skipping ${package_name}@${VERSION}; already published"
      return
      ;;
    404)
      ;;
    *)
      echo "Unable to check ${package_name}@${VERSION} on npm (HTTP $status)"
      cat "$response_file"
      exit 1
      ;;
  esac

  echo
  echo "Publishing ${package_name}@${VERSION}"

  if [[ -n "$install_command" ]]; then
    (
      cd "$package_dir"
      eval "$install_command"
    )
  fi

  (
    cd "$package_dir"
    if [[ "$DRY_RUN" == "--dry-run" ]]; then
      npm publish --access public --dry-run
    else
      npm publish --access public
    fi
  )
}

wait_for_package() {
  local package_name="$1"
  local version="$2"

  for attempt in {1..30}; do
    local published_version
    published_version="$(npm view "${package_name}@${version}" version 2>/dev/null || true)"
    if [[ "$published_version" == "$version" ]]; then
      echo "${package_name}@${version} is available"
      return
    fi

    echo "Waiting for ${package_name}@${version} (attempt ${attempt}/30)"
    sleep 10
  done

  echo "${package_name}@${version} did not propagate within 5 minutes"
  exit 1
}

echo "Staging npm packages into $STAGING_DIR"

copy_package_dir "$ROOT_DIR/typescript/core" "$STAGING_DIR/typescript/core"
copy_package_dir "$ROOT_DIR/typescript/react" "$STAGING_DIR/typescript/react"
copy_package_dir "$ROOT_DIR/typescript/adapters/kit" "$STAGING_DIR/typescript/adapters/kit"
copy_package_dir "$ROOT_DIR/typescript/adapters/web3js" "$STAGING_DIR/typescript/adapters/web3js"
copy_package_dir "$ROOT_DIR/packages/arete" "$STAGING_DIR/packages/arete"
copy_package_dir "$ROOT_DIR/packages/mcp" "$STAGING_DIR/packages/mcp"

rewrite_package_json "$STAGING_DIR/typescript/core/package.json" "$VERSION"
rewrite_package_lock "$STAGING_DIR/typescript/core/package-lock.json" "$VERSION"

rewrite_package_json "$STAGING_DIR/typescript/react/package.json" "$VERSION" "^$VERSION"
rewrite_package_lock "$STAGING_DIR/typescript/react/package-lock.json" "$VERSION" "^$VERSION"

rewrite_package_json "$STAGING_DIR/typescript/adapters/kit/package.json" "$VERSION" "^$VERSION"
rewrite_package_lock "$STAGING_DIR/typescript/adapters/kit/package-lock.json" "$VERSION" "^$VERSION"

rewrite_package_json "$STAGING_DIR/typescript/adapters/web3js/package.json" "$VERSION" "^$VERSION"
rewrite_package_lock "$STAGING_DIR/typescript/adapters/web3js/package-lock.json" "$VERSION" "^$VERSION"

rewrite_package_json "$STAGING_DIR/packages/arete/package.json" "$VERSION"
rewrite_package_json "$STAGING_DIR/packages/mcp/package.json" "$VERSION"

if npm_account="$(npm whoami 2>/dev/null)"; then
  echo "Using npm account: $npm_account"
elif [[ "$DRY_RUN" == "--dry-run" ]]; then
  echo "npm auth not configured locally; continuing because this is a dry run"
else
  echo "npm auth not configured. Run npm login or pass NPM_TOKEN=..."
  exit 1
fi

publish_package "$STAGING_DIR/typescript/core" "@usearete/sdk" "npm install"

if [[ "$DRY_RUN" != "--dry-run" ]]; then
  wait_for_package "@usearete/sdk" "$VERSION"
fi

publish_package "$STAGING_DIR/typescript/react" "@usearete/react" "npm install ../core --no-save --package-lock=false"
publish_package "$STAGING_DIR/typescript/adapters/kit" "@usearete/adapter-kit" "npm install ../../core --no-save --package-lock=false"
publish_package "$STAGING_DIR/typescript/adapters/web3js" "@usearete/adapter-web3js" "npm install ../../core --no-save --package-lock=false"
publish_package "$STAGING_DIR/packages/arete" "@usearete/a4"
publish_package "$STAGING_DIR/packages/mcp" "@usearete/mcp"

echo
if [[ "$DRY_RUN" == "--dry-run" ]]; then
  echo "Dry run completed"
else
  echo "Publish flow completed"
fi
