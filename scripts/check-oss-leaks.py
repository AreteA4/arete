#!/usr/bin/env python3
"""Fail when tracked files mention internal tracker ids or non-public repositories.

This repository is public. Its tree, including every generated CHANGELOG, must
not carry ids from the internal issue tracker or name repositories of the
GitHub organization that are not public. Rules:

  tracker-id         An internal tracker id: HYP-<n> or A4-<n> (case-sensitive,
                     word-bounded).
  non-public-repo    An ``AreteA4/<name>`` reference (any case, bare or in a
                     URL) whose <name> is not in PUBLIC_REPOS below. A ``.git``
                     suffix, sub-paths and a bare organization URL with no
                     repository are fine.
  extra pattern #N   Optional. Each non-blank line of the OSS_LEAK_EXTRA_PATTERNS
                     environment variable that does not start with ``#`` is a
                     regular expression (matched case-insensitively; a line that
                     is not a valid expression is matched literally). Hits are
                     reported by number only: neither the pattern nor the
                     matching line is ever printed. CI fills the variable from
                     the OSS_LEAK_EXTRA_PATTERNS repository secret, which GitHub
                     masks in logs; when it is unset the rule is skipped.

Only files ``git ls-files`` lists are scanned. Lock files, vendored
directories, binary files and this checker's own files are skipped.

Usage: check-oss-leaks.py [--root <git work tree>]
Exits 1 when anything matched, 0 when the tree is clean.
"""

import argparse
import os
import re
import subprocess
import sys

# Repositories of the organization that are public, compared in lower case.
# A reference to any other repository of the organization fails the check.
PUBLIC_REPOS = frozenset({"arete", "skills"})

# Organization references that name something other than a repository.
#   "Merge pull request #75 from AreteA4/auth" lines that release tooling
#   copies into CHANGELOGs name a branch of this repository, not a repository.
#   ALLOWED_PATH_REFERENCES lists lookalike names that a test uses on purpose
#   to prove they are rejected, by the file that uses them.
MERGE_LINE_BRANCH = re.compile(
    r"Merge pull request \[?#\d+\]?(?:\([^)]*\))? from AreteA4/\S+", re.IGNORECASE
)
ALLOWED_PATH_REFERENCES = {
    "cli/src/agents/skills.rs": frozenset({"skills-fork"}),
}

TRACKER_ID = re.compile(r"\b(?:HYP|A4)-[0-9]+\b")
# The organization name not preceded by another name character (so
# "notaretea4/x" is not a reference), then an optional repository name.
ORG_REFERENCE = re.compile(r"(?<![A-Za-z0-9_.-])AreteA4(?:/([A-Za-z0-9_.-]*))?", re.IGNORECASE)

# Paths of this checker and its self-test, which spell out what they look for.
SELF_PATHS = frozenset({"scripts/check-oss-leaks.py", "scripts/test-check-oss-leaks.py"})
LOCK_FILES = frozenset(
    {
        "Cargo.lock",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "pnpm-lock.yaml",
        "yarn.lock",
        "bun.lock",
        "bun.lockb",
        "poetry.lock",
        "uv.lock",
        "Gemfile.lock",
        "composer.lock",
        "flake.lock",
        "go.sum",
    }
)
VENDORED_DIRS = frozenset({"vendor", "vendored", "third_party", "third-party", "node_modules"})

MAX_PRINTED_LINE = 200


def load_extra_patterns():
    raw = os.environ.get("OSS_LEAK_EXTRA_PATTERNS", "")
    patterns = []
    for line in raw.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        try:
            patterns.append(re.compile(line, re.IGNORECASE))
        except re.error:
            patterns.append(re.compile(re.escape(line), re.IGNORECASE))
    return patterns


def skipped(path):
    parts = path.split("/")
    return (
        path in SELF_PATHS
        or parts[-1] in LOCK_FILES
        or any(part in VENDORED_DIRS for part in parts[:-1])
    )


def repo_name(raw):
    name = raw.lower()
    if name.endswith(".git"):
        name = name[: -len(".git")]
    return name.rstrip(".")


def non_public_references(path, line):
    allowed = PUBLIC_REPOS | ALLOWED_PATH_REFERENCES.get(path, frozenset())
    names = []
    for match in ORG_REFERENCE.finditer(MERGE_LINE_BRANCH.sub("", line)):
        name = repo_name(match.group(1) or "")
        if name and name not in allowed:
            names.append(name)
    return names


def scan(root, extra_patterns):
    listed = subprocess.run(
        ["git", "-C", root, "ls-files", "-z"], check=True, capture_output=True
    ).stdout
    paths = [p for p in listed.decode("utf-8", "surrogateescape").split("\0") if p]
    hits = []
    scanned = 0
    for path in paths:
        if skipped(path):
            continue
        full = os.path.join(root, path)
        if not os.path.isfile(full) or os.path.islink(full):
            continue
        with open(full, "rb") as handle:
            data = handle.read()
        if b"\0" in data[:8192]:
            continue
        scanned += 1
        text = data.decode("utf-8", "replace")
        for number, line in enumerate(text.splitlines(), start=1):
            shown = line.strip()
            if len(shown) > MAX_PRINTED_LINE:
                shown = shown[:MAX_PRINTED_LINE] + "..."
            if TRACKER_ID.search(line):
                hits.append(f"{path}:{number}: tracker-id: {shown}")
            for name in non_public_references(path, line):
                hits.append(f"{path}:{number}: non-public-repo '{name}': {shown}")
            for index, pattern in enumerate(extra_patterns, start=1):
                if pattern.search(line):
                    hits.append(f"{path}:{number}: matched extra pattern #{index}")
    return scanned, hits


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="git work tree to scan (default: .)")
    args = parser.parse_args()

    extra_patterns = load_extra_patterns()
    if not extra_patterns:
        print("OSS_LEAK_EXTRA_PATTERNS is not set: skipping extra patterns.")

    scanned, hits = scan(args.root, extra_patterns)
    for hit in hits:
        print(hit)
    rules = 2 + len(extra_patterns)
    if hits:
        print(
            f"\n{len(hits)} leak(s) in {scanned} tracked files. Remove internal tracker ids"
            " and references to non-public repositories from the public tree."
        )
        return 1
    print(f"No leaks: {scanned} tracked files checked against {rules} rules.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
