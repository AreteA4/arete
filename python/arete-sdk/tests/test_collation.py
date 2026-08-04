"""Regression tests for the shared ``localeCompare`` equivalent.

The expected values in :data:`NODE_LOCALE_COMPARE` and :data:`NODE_SORTS` are
captured verbatim from Node's default ``en-US``/ICU collator
(``node -e "...a.localeCompare(b)..."``, Node v23), which is the reference TS
uses at ``typescript/core/src/subscription.ts`` lines 57/123 and
``typescript/core/src/query-store.ts`` lines 64/387.

:func:`test_matches_live_node_localecompare` re-runs the whole table against
whatever ``node`` is on PATH, so the captured values cannot silently rot; it
skips when Node is unavailable.
"""

from __future__ import annotations

import json
import shutil
import subprocess

import pytest

from arete.subscription import collation_key, locale_compare

# The full printable-ASCII block and the order Node's collator puts it in.
_PRINTABLE_ASCII = [chr(code) for code in range(0x20, 0x7F)]
_PRINTABLE_ASCII_IN_ICU_ORDER = list(
    " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$"
    "0123456789"
    "aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ"
)

# (left, right, expected sign of left.localeCompare(right))
NODE_LOCALE_COMPARE = [
    ("a", "A", -1),
    ("A", "a", 1),
    ("a", "b", -1),
    ("A", "b", -1),
    ("b", "A", 1),
    ("aBc1", "apple", -1),
    ("apple", "Bqq", -1),
    ("Bqq", "Zap1", -1),
    ("Zap1", "aBc1", 1),
    ("Test", "test", 1),
    ("test", "Test", -1),
    ("aa", "aB", -1),
    ("a1", "A1", -1),
    ("abc", "abcd", -1),
    ("abcd", "abc", 1),
    ("ZapfvUeQ", "aBc1xYz", 1),
    (
        "So11111111111111111111111111111111111111112",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        1,
    ),
    ("état", "zone", -1),
    ("zone", "état", 1),
    ("etat", "état", -1),
    ("état", "Etat", 1),
    ("Etat", "ETAT", -1),
    ("ETAT", "état", -1),
    ("é", "e", 1),
    ("e", "é", -1),
    ("é", "f", -1),
    ("z", "é", 1),
    ("résumé", "resume", 1),
    ("resume", "résumé", -1),
    ("a", "á", -1),
    ("á", "à", -1),
    ("à", "ă", -1),
    ("ă", "â", -1),
    ("â", "ǎ", -1),
    ("ǎ", "å", -1),
    ("å", "ä", -1),
    ("ä", "ã", -1),
    ("ã", "ȧ", -1),
    ("ñ", "n", 1),
    ("ñ", "o", -1),
    ("ç", "c", 1),
    ("ç", "d", -1),
    ("ö", "o", 1),
    ("ö", "p", -1),
    ("ü", "u", 1),
    ("ü", "v", -1),
    ("ß", "ss", 1),
    ("ss", "ß", -1),
    ("ß", "st", -1),
    ("ß", "z", -1),
    ("æ", "ae", 1),
    ("æ", "ad", 1),
    ("æ", "af", -1),
    ("œ", "oe", 1),
    ("œ", "of", -1),
    ("ø", "o", 1),
    ("ø", "ö", 1),
    ("ø", "p", -1),
    ("ł", "l", 1),
    ("ł", "m", -1),
    ("đ", "d", 1),
    ("đ", "e", -1),
    ("Å", "å", 1),
    ("É", "é", 1),
    ("é", "e\u0301", 0),
    ("e\u0301", "é", 0),
    ("cafe\u0301", "café", 0),
    ("1", "a", -1),
    ("1", "A", -1),
    ("9", "a", -1),
    ("0", "Z", -1),
    ("10", "9", -1),
    ("9", "10", 1),
    ("2", "10", 1),
    ("a1", "a2", -1),
    ("1a", "a1", -1),
    ("0", "1", -1),
    ("_", "a", -1),
    ("_", "1", -1),
    ("-", "_", 1),
    (".", "-", 1),
    (".", "_", 1),
    ("a.b", "a_b", 1),
    ("a_b", "a.b", -1),
    ("a-b", "a.b", -1),
    (" ", "_", -1),
    (" ", "a", -1),
    ("$", "a", -1),
    ("$", "0", -1),
    ("~", "0", -1),
    ("(", ")", -1),
    ("[", "{", -1),
    ("@", "*", -1),
    ("/", "\\", -1),
    ("&", "#", -1),
    ("%", "`", -1),
    ("^", "+", -1),
    ("<", "=", -1),
    ("=", ">", -1),
    ("|", "~", -1),
    ("!", "?", -1),
    (",", ";", -1),
    (":", "!", -1),
    ("'", "\"", -1),
    ("a.b.c", "a.b", 1),
    ("view.field", "view_field", 1),
    ("owner.address", "Owner.Address", -1),
    ("", "a", -1),
    ("a", "", 1),
    ("", "", 0),
    ("a", "a", 0),
    ("état", "état", 0),
    ("", "_", -1),
    ("", "0", -1),
    ("a\u200bb", "ab", 0),
    ("ab", "a\u200bb", 0),
    ("a\u00adb", "ab", 0),
    ("a\u0000b", "ab", 0),
    ("a\u200b", "a", 0),
    ("\u200b", "", 0),
    ("α", "Ω", -1),
    ("Ω", "α", 1),
    ("α", "β", -1),
    ("а", "Я", -1),
]

# (input, expected [...input].sort((a, b) => a.localeCompare(b)))
NODE_SORTS = [
    (
        ["Zap1", "aBc1", "Bqq", "apple"],
        ["aBc1", "apple", "Bqq", "Zap1"],
    ),
    (
        ["état", "zone"],
        ["état", "zone"],
    ),
    (
        ["zone", "état"],
        ["état", "zone"],
    ),
    (
        ["etat", "Etat", "ETAT", "état", "zone", "Zone"],
        ["etat", "Etat", "ETAT", "état", "zone", "Zone"],
    ),
    (
        ["b", "B", "a", "A", "c", "C"],
        ["a", "A", "b", "B", "c", "C"],
    ),
    (
        ["9", "10", "A", "_", "é", "Z", "0", "a"],
        ["_", "0", "10", "9", "a", "A", "é", "Z"],
    ),
    (
        ["owner.address", "owner_address", "owner-address", "Owner.Address"],
        ["owner_address", "owner-address", "owner.address", "Owner.Address"],
    ),
    (
        ["", "a", "A", "0", "_"],
        ["", "_", "0", "a", "A"],
    ),
    (
        _PRINTABLE_ASCII,
        _PRINTABLE_ASCII_IN_ICU_ORDER,
    ),
    (
        ["mint", "Mint", "MINT", "mInt", "mint1", "mint0"],
        ["mint", "mInt", "Mint", "MINT", "mint0", "mint1"],
    ),
]

_NODE_SCRIPT = """
const input = JSON.parse(require('fs').readFileSync(0, 'utf8'));
const cmp = (a, b) => Math.sign(a.localeCompare(b));
console.log(JSON.stringify({
  pairs: input.pairs.map(([a, b]) => cmp(a, b)),
  sorts: input.sorts.map((list) => [...list].sort((a, b) => a.localeCompare(b))),
}));
"""


@pytest.mark.parametrize(("left", "right", "expected"), NODE_LOCALE_COMPARE)
def test_locale_compare_matches_node(left, right, expected):
    assert locale_compare(left, right) == expected


@pytest.mark.parametrize(("source", "expected"), NODE_SORTS)
def test_collation_key_sorts_like_node(source, expected):
    assert sorted(source, key=collation_key) == expected


def test_locale_compare_is_antisymmetric_and_transitive():
    sample = [
        "", "a", "A", "b", "B", "z", "Z", "0", "1", "9", "_", "-", ".", " ",
        "\u00e9", "\u00c9", "e", "E", "\u00e4", "\u00f1", "aBc1", "apple",
        "Bqq", "Zap1", "\u00e9tat", "zone",
    ]
    for left in sample:
        assert locale_compare(left, left) == 0
        for right in sample:
            assert locale_compare(left, right) == -locale_compare(right, left)
    ordered = sorted(sample, key=collation_key)
    for index in range(len(ordered) - 1):
        assert locale_compare(ordered[index], ordered[index + 1]) <= 0


def test_canonical_equivalence_and_ignorables():
    # NFC and NFD spellings of the same text collate equal, as in ICU.
    assert locale_compare("caf\u00e9", "cafe\u0301") == 0
    assert collation_key("caf\u00e9") == collation_key("cafe\u0301")
    # Completely ignorable characters contribute nothing.
    assert locale_compare("a\u200bb", "ab") == 0
    assert locale_compare("a\u00adb", "ab") == 0


@pytest.mark.skipif(shutil.which("node") is None, reason="node is not installed")
def test_matches_live_node_localecompare():
    payload = {
        "pairs": [[left, right] for left, right, _ in NODE_LOCALE_COMPARE],
        "sorts": [source for source, _ in NODE_SORTS],
    }
    completed = subprocess.run(
        ["node", "-e", _NODE_SCRIPT],
        input=json.dumps(payload),
        capture_output=True,
        text=True,
        check=True,
    )
    live = json.loads(completed.stdout.strip().splitlines()[-1])

    assert live["pairs"] == [expected for _, _, expected in NODE_LOCALE_COMPARE]
    assert live["sorts"] == [expected for _, expected in NODE_SORTS]
    assert [
        locale_compare(left, right) for left, right, _ in NODE_LOCALE_COMPARE
    ] == live["pairs"]
    assert [
        sorted(source, key=collation_key) for source, _ in NODE_SORTS
    ] == live["sorts"]
