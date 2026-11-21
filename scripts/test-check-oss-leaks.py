#!/usr/bin/env python3
"""Self-test for check-oss-leaks.py against throwaway git trees.

The fixtures are written at runtime into temporary directories. The leaks they
plant are assembled from pieces, so this file never spells one out itself.
"""

import os
import subprocess
import sys
import tempfile

CHECKER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "check-oss-leaks.py")

ORG = "Arete" + "A4"
TRACKER_ID = "HYP" + "-123"
SECOND_TRACKER_ID = "A4" + "-456"
PRIVATE_REPO = f"{ORG}/some-private-thing"

# References the checker must accept.
CLEAN_LINES = [
    f"Clone https://github.com/{ORG}/arete.git or browse https://github.com/{ORG}/arete/tree/main/docs.",
    f"A clone URL can end a sentence: https://github.com/{ORG}/arete.git.",
    f"Install skills with `npx skills add {ORG}/skills`.",
    f"Organization page: https://github.com/{ORG} and github.com/{ORG}/.",
    f"Lookalikes are other organizations: not{ORG.lower()}/private-thing.",
    f"* Merge pull request #75 from {ORG}/feature-branch",
    f"* Merge pull request [#75](https://github.com/{ORG}/arete/issues/75) from {ORG}/feature-branch",
    "Paper sizes like A4 and ids without digits like HYP-X are fine.",
]


def git_tree(files, untracked=None):
    root = tempfile.mkdtemp(prefix="oss-leak-fixture-")
    subprocess.run(["git", "init", "-q", root], check=True)
    for path, content in files.items():
        full = os.path.join(root, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        mode = "wb" if isinstance(content, bytes) else "w"
        with open(full, mode) as handle:
            handle.write(content)
    subprocess.run(["git", "-C", root, "add", "-A"], check=True)
    for path, content in (untracked or {}).items():
        with open(os.path.join(root, path), "w") as handle:
            handle.write(content)
    return root


def run(root, extra_patterns=None):
    env = dict(os.environ)
    env.pop("OSS_LEAK_EXTRA_PATTERNS", None)
    if extra_patterns is not None:
        env["OSS_LEAK_EXTRA_PATTERNS"] = extra_patterns
    result = subprocess.run(
        [sys.executable, CHECKER, "--root", root], env=env, capture_output=True, text=True
    )
    return result.returncode, result.stdout + result.stderr


failures = []


def expect(label, condition, output):
    if condition:
        print(f"ok   {label}")
    else:
        print(f"FAIL {label}\n{output}")
        failures.append(label)


# A clean tree passes, including leaks in files the checker must skip.
clean = git_tree(
    {
        "README.md": "\n".join(CLEAN_LINES) + "\n",
        "crate/CHANGELOG.md": CLEAN_LINES[4] + "\n",
        "Cargo.lock": f"{TRACKER_ID}\n",
        "web/package-lock.json": f"{PRIVATE_REPO}\n",
        "vendor/lib/notes.md": f"{TRACKER_ID}\n",
        "scripts/check-oss-leaks.py": f"# {PRIVATE_REPO}\n",
        "assets/blob.bin": b"\x00\x01" + TRACKER_ID.encode() + b"\n",
    },
    untracked={"scratch.md": f"{TRACKER_ID} {PRIVATE_REPO}\n"},
)
code, output = run(clean)
expect("clean tree passes", code == 0 and "No leaks" in output, output)
expect(
    "unset extra patterns are skipped with a notice",
    "OSS_LEAK_EXTRA_PATTERNS is not set" in output,
    output,
)

# Tracker ids and non-public repositories fail, anywhere, with file:line.
leaky = git_tree(
    {
        "README.md": "intro\n" + f"See https://github.com/{PRIVATE_REPO}.git for details.\n",
        "packages/x/CHANGELOG.md": f"# Changelog\n\n* fix a bug ({TRACKER_ID})\n",
        "src/lib.rs": f"// tracked as {SECOND_TRACKER_ID}\n",
        "docs/merge.md": f"Merge pull request #9 from {ORG}/branch, see {PRIVATE_REPO}\n",
    }
)
code, output = run(leaky)
expect("leaky tree fails", code == 1, output)
expect("non-public repo reported with file:line", "README.md:2: non-public-repo" in output, output)
expect("tracker id reported in a nested CHANGELOG", "packages/x/CHANGELOG.md:3: tracker-id" in output, output)
expect("second tracker id form reported", "src/lib.rs:1: tracker-id" in output, output)
expect(
    "merge-line exemption covers only the branch",
    "docs/merge.md:1: non-public-repo 'some-private-thing'" in output,
    output,
)

# Extra patterns match by number and never print the pattern or the line.
secret_word = "zebra" + "-crossing"
extra = git_tree({"notes.md": f"one\nmentions {secret_word.upper()}-7 here\n"})
code, output = run(extra, extra_patterns=f"# comment line\n\nunused-token\n{secret_word}-[0-9]+\n")
expect("extra pattern fails the tree", code == 1, output)
expect("extra pattern reported by number", "notes.md:2: matched extra pattern #2" in output, output)
expect(
    "extra pattern output hides the pattern and the line",
    "zebra" not in output.lower() and "mentions" not in output,
    output,
)
# A line an extra pattern matches stays hidden even when a built-in rule
# matches it too, including the repository name it would print.
overlap = git_tree(
    {"notes.md": f"{TRACKER_ID} and {ORG}/{secret_word}-9 in one line\n"}
)
code, output = run(overlap, extra_patterns=f"{secret_word}-[0-9]+")
expect("overlapping rules fail the tree", code == 1, output)
expect(
    "overlapping rules report every rule without the line",
    "notes.md:1: tracker-id\n" in output
    and "notes.md:1: non-public-repo\n" in output
    and "notes.md:1: matched extra pattern #1" in output,
    output,
)
expect(
    "overlapping rules hide the matched text",
    "zebra" not in output.lower() and "one line" not in output and TRACKER_ID not in output,
    output,
)

code, output = run(extra, extra_patterns="[unclosed")
expect("invalid expression is matched literally", code == 0, output)

if failures:
    print(f"\n{len(failures)} check(s) failed")
    sys.exit(1)
print("\nAll leak checker self-tests passed")
