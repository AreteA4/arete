#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Publish the Arete npm packages for a specific version from the current checkout.

Usage:
  ./scripts/publish-npm-version.sh <version> [--dry-run]
  ./scripts/publish-npm-version.sh <version> --stage-only <empty-dir>
  ./scripts/publish-npm-version.sh <version> --pack-only <empty-dir>
  ./scripts/publish-npm-version.sh <version> --publish-packed <dir>

Examples:
  ./scripts/publish-npm-version.sh 0.1.3
  NPM_TOKEN=xxxx ./scripts/publish-npm-version.sh 0.1.3
  ./scripts/publish-npm-version.sh 0.1.3 --dry-run

Notes:
  - Copies only the npm package directories into a temporary repo snapshot.
  - Rewrites the staged package metadata to the target version.
  - Publishes in dependency order: hash -> protocol -> sdk -> kit adapter -> react -> web3.js adapter -> automations -> a4 -> mcp.
  - Keeps the independently versioned hash and protocol packages at their
    checked-in versions.
  - Skips packages that are already published at that exact version.
  - Installs from the committed lockfiles with `npm ci --ignore-scripts` and
    publishes packed tarballs with `--ignore-scripts`.
  - --stage-only writes the rewritten snapshot to <empty-dir> and stops
    before contacting npm, so the staged manifests can be inspected.
  - --pack-only builds and packs the missing packages into <empty-dir>
    (with an ordered packages.txt) and refuses to run with npm or OIDC
    credentials in the environment. --publish-packed then publishes that
    directory without installing or building anything, so builds never run
    in a job that can publish.
EOF
}

VERSION="${1:-}"
MODE="${2:-}"

if [[ -z "$VERSION" ]]; then
  usage
  exit 1
fi

DRY_RUN=""
STAGE_ONLY_DIR=""
PACK_DIR=""
case "$MODE" in
  "")
    ;;
  --dry-run)
    DRY_RUN="--dry-run"
    ;;
  --stage-only|--pack-only)
    STAGE_ONLY_DIR="${3:-}"
    if [[ -z "$STAGE_ONLY_DIR" || ! -d "$STAGE_ONLY_DIR" || -n "$(ls -A "$STAGE_ONLY_DIR")" ]]; then
      echo "$MODE needs an existing empty directory"
      exit 1
    fi
    if [[ "$MODE" == "--pack-only" ]]; then
      PACK_DIR="$(cd "$STAGE_ONLY_DIR" && pwd)"
      STAGE_ONLY_DIR=""
    fi
    ;;
  --publish-packed)
    PACK_DIR="${3:-}"
    if [[ -z "$PACK_DIR" || ! -f "$PACK_DIR/packages.txt" ]]; then
      echo "--publish-packed needs a directory written by --pack-only"
      exit 1
    fi
    PACK_DIR="$(cd "$PACK_DIR" && pwd)"
    ;;
  *)
    usage
    exit 1
    ;;
esac

if [[ "$MODE" == "--pack-only" ]]; then
  node_auth_token="${NODE_AUTH_TOKEN:-}"
  if [[ -n "${NPM_TOKEN:-}" || -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" \
    || ( -n "$node_auth_token" && "$node_auth_token" != "XXXXX-XXXXX-XXXXX-XXXXX" ) ]]; then
    echo "--pack-only runs package installs and builds; run it without npm or OIDC credentials"
    exit 1
  fi
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
if [[ -n "$STAGE_ONLY_DIR" ]]; then
  STAGING_DIR="$(cd "$STAGE_ONLY_DIR" && pwd)"
else
  STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arete-npm-publish-${VERSION}.XXXXXX")"

  cleanup() {
    rm -rf "$STAGING_DIR"
  }

  trap cleanup EXIT
fi
if [[ -z "$MODE" || "$MODE" == "--dry-run" ]]; then
  PACK_DIR="$STAGING_DIR/.packed"
  mkdir -p "$PACK_DIR"
fi

using_trusted_publishing() {
  local node_auth_token="${NODE_AUTH_TOKEN:-}"

  # setup-node exports this placeholder when registry-url is set without a token.
  [[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" \
    && -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" \
    && -z "${NPM_TOKEN:-}" \
    && ( -z "$node_auth_token" || "$node_auth_token" == "XXXXX-XXXXX-XXXXX-XXXXX" ) ]]
}

if [[ -n "${NPM_TOKEN:-}" && -z "$STAGE_ONLY_DIR" && "$MODE" != "--pack-only" ]]; then
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

# The npm packages released together at one version. A staged manifest or
# lockfile that depends on one of them gets the release range, so a package
# published here accepts the matching versions of its siblings (React's
# @usearete/sdk and @usearete/adapter-kit peers, for example). The
# independently versioned hash and protocol packages are not in this list.
LINKED_NPM_PACKAGES="@usearete/sdk,@usearete/adapter-kit,@usearete/adapter-web3js,@usearete/react,@usearete/automations,@usearete/a4,@usearete/mcp"

rewrite_package_json() {
  local file="$1"
  local version="$2"
  local linked_range="${3:-}"

  node - "$file" "$version" "$linked_range" "$LINKED_NPM_PACKAGES" <<'EOF'
const fs = require('node:fs');

const [file, version, linkedRange, linkedNames] = process.argv.slice(2);
const linked = new Set(linkedNames.split(','));
const pkg = JSON.parse(fs.readFileSync(file, 'utf8'));

pkg.version = version;

for (const section of ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies']) {
  for (const name of Object.keys(pkg[section] ?? {})) {
    if (linkedRange && linked.has(name)) pkg[section][name] = linkedRange;
  }
}

fs.writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
EOF
}

rewrite_package_lock() {
  local file="$1"
  local version="$2"
  local linked_range="${3:-}"

  if [[ ! -f "$file" ]]; then
    return
  fi

  node - "$file" "$version" "$linked_range" "$LINKED_NPM_PACKAGES" <<'EOF'
const fs = require('node:fs');

const [file, version, linkedRange, linkedNames] = process.argv.slice(2);
const linked = new Set(linkedNames.split(','));
const lock = JSON.parse(fs.readFileSync(file, 'utf8'));
const sections = ['dependencies', 'devDependencies', 'optionalDependencies', 'peerDependencies'];

function rewriteRanges(entry) {
  if (!linkedRange) return;
  for (const section of sections) {
    for (const name of Object.keys(entry[section] ?? {})) {
      if (linked.has(name)) entry[section][name] = linkedRange;
    }
  }
}

lock.version = version;

if (lock.packages && lock.packages['']) {
  lock.packages[''].version = version;
  rewriteRanges(lock.packages['']);
}

// Sibling packages linked from this checkout (for example React's
// "../adapters/kit") are staged at the release version too.
for (const [location, entry] of Object.entries(lock.packages ?? {})) {
  if (location === '' || location.split('/').includes('node_modules')) continue;
  if (!linkedRange || !linked.has(entry.name)) continue;
  entry.version = version;
  rewriteRanges(entry);
}

fs.writeFileSync(file, `${JSON.stringify(lock, null, 2)}\n`);
EOF
}

# Prints the registry's HTTP status for name@version (200 or 404), or exits.
registry_status() {
  local package_name="$1"
  local version="$2"
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
    "https://registry.npmjs.org/${encoded_name}/${version}")"

  case "$status" in
    200|404)
      printf '%s' "$status"
      ;;
    *)
      echo "Unable to check ${package_name}@${version} on npm (HTTP $status)" >&2
      cat "$response_file" >&2
      exit 1
      ;;
  esac
}

PACKED_COUNT=0

# Installs from the staged lockfile without install scripts, builds, and packs
# a package that is not on npm yet. packages.txt lists the tarballs in publish
# order as name|version|path relative to PACK_DIR.
pack_package() {
  local package_dir="$1"
  local package_name="$2"
  local install_command="${3:-}"
  local expected_version="${4:-$VERSION}"

  local staged_version
  staged_version="$(node -p "require('$package_dir/package.json').version")"

  if [[ "$staged_version" != "$expected_version" ]]; then
    echo "Expected $package_name to be staged as $expected_version, found $staged_version"
    exit 1
  fi

  local status
  status="$(registry_status "$package_name" "$expected_version")"
  if [[ "$status" == 200 ]]; then
    echo "Skipping ${package_name}@${expected_version}; already published"
    return
  fi

  echo
  echo "Packing ${package_name}@${expected_version}"

  PACKED_COUNT=$((PACKED_COUNT + 1))
  local slot
  slot="$(printf '%02d' "$PACKED_COUNT")"
  mkdir -p "$PACK_DIR/$slot"
  (
    cd "$package_dir"
    if [[ -n "$install_command" ]]; then
      eval "$install_command"
    fi
    if [[ "$(node -p "Boolean(require('./package.json').scripts?.build)")" == "true" ]]; then
      npm run build
    fi
    npm pack --ignore-scripts --pack-destination "$PACK_DIR/$slot"
  )

  local tarballs=("$PACK_DIR/$slot"/*.tgz)
  if [[ ${#tarballs[@]} -ne 1 || ! -f "${tarballs[0]}" ]]; then
    echo "Expected one packed tarball for $package_name in $PACK_DIR/$slot"
    exit 1
  fi
  printf '%s|%s|%s\n' "$package_name" "$expected_version" "$slot/$(basename "${tarballs[0]}")" >>"$PACK_DIR/packages.txt"
}

tarball_id() {
  tar -xzOf "$1" package/package.json | node -e '
    let input = "";
    process.stdin.on("data", (chunk) => { input += chunk; });
    process.stdin.on("end", () => {
      const pkg = JSON.parse(input);
      process.stdout.write(`${pkg.name}@${pkg.version}`);
    });
  '
}

# Publishes the tarballs packages.txt lists, in order. Only the expected
# packages at the expected versions are accepted, whatever produced the
# directory.
publish_packed() {
  local hash_version
  local protocol_version=""
  hash_version="$(node -p "require('$ROOT_DIR/typescript/hash/package.json').version")"
  if [[ -f "$ROOT_DIR/typescript/protocol/package.json" ]]; then
    protocol_version="$(node -p "require('$ROOT_DIR/typescript/protocol/package.json').version")"
  fi

  local package_name
  local package_version
  local relative_path
  # fd 3, so npm cannot consume the list from stdin.
  while IFS='|' read -r -u 3 package_name package_version relative_path; do
    [[ -n "$package_name" ]] || continue

    local expected_version="$VERSION"
    case "$package_name" in
      @usearete/hash)
        expected_version="$hash_version"
        ;;
      @usearete/protocol)
        expected_version="$protocol_version"
        ;;
      @usearete/sdk|@usearete/adapter-kit|@usearete/react|@usearete/adapter-web3js|@usearete/automations|@usearete/a4|@usearete/mcp)
        ;;
      *)
        echo "Refusing to publish unexpected package $package_name"
        exit 1
        ;;
    esac
    if [[ -z "$expected_version" || "$package_version" != "$expected_version" ]]; then
      echo "Refusing to publish $package_name@$package_version; expected version ${expected_version:-<none>}"
      exit 1
    fi
    if [[ "$relative_path" == /* || "$relative_path" == *..* || "$relative_path" != *.tgz ]]; then
      echo "Refusing tarball path $relative_path for $package_name"
      exit 1
    fi
    local tarball="$PACK_DIR/$relative_path"
    if [[ ! -f "$tarball" ]]; then
      echo "Missing tarball $relative_path for $package_name"
      exit 1
    fi
    local packed_id
    packed_id="$(tarball_id "$tarball")"
    if [[ "$packed_id" != "$package_name@$package_version" ]]; then
      echo "Tarball $relative_path contains $packed_id, expected $package_name@$package_version"
      exit 1
    fi

    local status
    status="$(registry_status "$package_name" "$package_version")"
    if [[ "$status" == 200 ]]; then
      echo "Skipping ${package_name}@${package_version}; already published"
      continue
    fi

    echo
    echo "Publishing ${package_name}@${package_version}"
    if [[ "$DRY_RUN" == "--dry-run" ]]; then
      npm publish "$tarball" --access public --provenance --ignore-scripts --dry-run
    else
      npm publish "$tarball" --access public --provenance --ignore-scripts
      # Later packages resolve these from the registry.
      if [[ "$package_name" == "@usearete/hash" || "$package_name" == "@usearete/sdk" ]]; then
        wait_for_package "$package_name" "$package_version"
      fi
    fi
  done 3<"$PACK_DIR/packages.txt"
}

check_publish_auth() {
  if using_trusted_publishing; then
    echo "Using npm trusted publishing"
  elif npm_account="$(npm whoami 2>/dev/null)"; then
    echo "Using npm account: $npm_account"
  elif [[ "$DRY_RUN" == "--dry-run" ]]; then
    echo "npm auth not configured locally; continuing because this is a dry run"
  else
    echo "npm auth not configured. Run npm login or pass NPM_TOKEN=..."
    exit 1
  fi
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

if [[ "$MODE" == "--publish-packed" ]]; then
  check_publish_auth
  publish_packed
  echo
  echo "Publish flow completed"
  exit 0
fi

echo "Staging npm packages into $STAGING_DIR"

copy_package_dir "$ROOT_DIR/typescript/core" "$STAGING_DIR/typescript/core"
copy_package_dir "$ROOT_DIR/typescript/hash" "$STAGING_DIR/typescript/hash"
if [[ -f "$ROOT_DIR/typescript/protocol/package.json" ]]; then
  copy_package_dir "$ROOT_DIR/typescript/protocol" "$STAGING_DIR/typescript/protocol"
fi
copy_package_dir "$ROOT_DIR/typescript/react" "$STAGING_DIR/typescript/react"
copy_package_dir "$ROOT_DIR/typescript/adapters/kit" "$STAGING_DIR/typescript/adapters/kit"
copy_package_dir "$ROOT_DIR/typescript/adapters/web3js" "$STAGING_DIR/typescript/adapters/web3js"
# Older release sources have no published @usearete/automations.
AUTOMATIONS=""
if [[ -f "$ROOT_DIR/typescript/automations/package.json" ]] \
  && [[ "$(node -p "require('$ROOT_DIR/typescript/automations/package.json').private === true")" != "true" ]]; then
  AUTOMATIONS=1
  copy_package_dir "$ROOT_DIR/typescript/automations" "$STAGING_DIR/typescript/automations"
fi
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

if [[ -n "$AUTOMATIONS" ]]; then
  rewrite_package_json "$STAGING_DIR/typescript/automations/package.json" "$VERSION" "^$VERSION"
  rewrite_package_lock "$STAGING_DIR/typescript/automations/package-lock.json" "$VERSION" "^$VERSION"
fi

rewrite_package_json "$STAGING_DIR/packages/arete/package.json" "$VERSION"
rewrite_package_json "$STAGING_DIR/packages/mcp/package.json" "$VERSION"

if [[ -n "$STAGE_ONLY_DIR" ]]; then
  echo "Staged npm packages in $STAGING_DIR"
  exit 0
fi

if [[ "$MODE" != "--pack-only" ]]; then
  check_publish_auth
fi

: >"$PACK_DIR/packages.txt"

# Every install uses the staged lockfile and runs no install scripts. Staging
# drops dist, and a package already on npm is not rebuilt, so a dependent
# builds whichever linked sibling it compiles against is missing.
NPM_CI="npm ci --ignore-scripts --no-audit --no-fund"
BUILD_CORE="([ -d ../core/dist ] || (cd ../core && $NPM_CI && npm run build))"
BUILD_CORE_FROM_ADAPTER="([ -d ../../core/dist ] || (cd ../../core && $NPM_CI && npm run build))"

hash_version="$(node -p "require('$STAGING_DIR/typescript/hash/package.json').version")"
pack_package "$STAGING_DIR/typescript/hash" "@usearete/hash" "$NPM_CI" "$hash_version"

# @usearete/protocol builds against the staged hash package (its lockfile
# links ../hash), whether or not hash is packed above.
if [[ -f "$STAGING_DIR/typescript/protocol/package.json" ]]; then
  node "$ROOT_DIR/scripts/check-protocol-release-dependency.mjs" "$STAGING_DIR/typescript/protocol"
  protocol_version="$(node -p "require('$STAGING_DIR/typescript/protocol/package.json').version")"
  pack_package "$STAGING_DIR/typescript/protocol" "@usearete/protocol" \
    "([ -d ../hash/dist ] || (cd ../hash && $NPM_CI && npm run build)) && $NPM_CI" "$protocol_version"
fi

pack_package "$STAGING_DIR/typescript/core" "@usearete/sdk" "$NPM_CI"
pack_package "$STAGING_DIR/typescript/adapters/kit" "@usearete/adapter-kit" "$BUILD_CORE_FROM_ADAPTER && $NPM_CI"
# @usearete/react/wallet-standard compiles against the staged Kit adapter's
# types.
pack_package "$STAGING_DIR/typescript/react" "@usearete/react" \
  "$BUILD_CORE && ([ -d ../adapters/kit/dist ] || (cd ../adapters/kit && $NPM_CI && npm run build)) && $NPM_CI"
pack_package "$STAGING_DIR/typescript/adapters/web3js" "@usearete/adapter-web3js" "$BUILD_CORE_FROM_ADAPTER && $NPM_CI"
if [[ -n "$AUTOMATIONS" ]]; then
  pack_package "$STAGING_DIR/typescript/automations" "@usearete/automations" "$BUILD_CORE && $NPM_CI"
fi
pack_package "$STAGING_DIR/packages/arete" "@usearete/a4"
pack_package "$STAGING_DIR/packages/mcp" "@usearete/mcp"

if [[ "$MODE" == "--pack-only" ]]; then
  echo
  echo "Packed npm packages in $PACK_DIR:"
  cat "$PACK_DIR/packages.txt"
  exit 0
fi

publish_packed

echo
if [[ "$DRY_RUN" == "--dry-run" ]]; then
  echo "Dry run completed"
else
  echo "Publish flow completed"
fi
