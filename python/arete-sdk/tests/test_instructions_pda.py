"""PDA derivation and seed serialization vectors.

Ported from typescript/core/src/instructions/instructions.test.ts
(createSeed / findProgramAddress suites),
typescript/core/src/instructions/seed-serializer.test.ts, and
rust/arete-a4-sdk/src/instruction/seed.rs tests. Golden addresses were
generated with the TS reference stack (@noble/ed25519 + bs58 + sha256), so the
three implementations are proven byte-identical.
"""

from __future__ import annotations

import pytest

from arete.instructions import (
    AccountRefSeed,
    ArgRefSeed,
    BytesSeed,
    InstructionError,
    LiteralSeed,
    PdaConfig,
    decode_base58,
    derive_pda,
    find_program_address,
    normalize_seed_type,
    serialize_seed_value,
)

SYSTEM_PROGRAM = "11111111111111111111111111111111"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
WSOL_MINT = "So11111111111111111111111111111111111111112"
ATA_PROGRAM = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL"


class TestNormalizeSeedType:
    def test_canonicalizes_pubkey_and_string_spellings(self):
        for spelling in ["pubkey", "Pubkey", "publicKey", "PublicKey", "solana_pubkey::Pubkey"]:
            assert normalize_seed_type(spelling) == "pubkey"
        for spelling in ["string", "String", "str"]:
            assert normalize_seed_type(spelling) == "string"

    def test_passes_integer_widths_through_and_rejects_everything_else(self):
        assert normalize_seed_type("u32") == "u32"
        assert normalize_seed_type("i64") == "i64"
        assert normalize_seed_type("u24") is None
        assert normalize_seed_type("Vec<u8>") is None
        assert normalize_seed_type(None) is None


class TestSerializeSeedValueTyped:
    def test_encodes_integers_little_endian_at_the_declared_width(self):
        assert list(serialize_seed_value(1, "u8")) == [1]
        assert list(serialize_seed_value(0x0102, "u16")) == [2, 1]
        assert list(serialize_seed_value(7, "u32")) == [7, 0, 0, 0]
        assert list(serialize_seed_value(42, "u64")) == [42, 0, 0, 0, 0, 0, 0, 0]
        # Decimal strings mirror the TS bigint / Rust string path.
        assert list(serialize_seed_value("42", "u64")) == [42, 0, 0, 0, 0, 0, 0, 0]

    def test_encodes_negative_signed_integers_in_twos_complement(self):
        assert serialize_seed_value(-1, "i64") == b"\xff" * 8

    def test_rejects_values_that_overflow_the_declared_width(self):
        with pytest.raises(InstructionError, match="does not fit"):
            serialize_seed_value(256, "u8")
        with pytest.raises(InstructionError, match="does not fit"):
            serialize_seed_value(-1, "u32")

    def test_decodes_pubkey_seeds_from_base58_to_32_bytes(self):
        decoded = serialize_seed_value(TOKEN_PROGRAM, "pubkey")
        assert len(decoded) == 32
        assert decoded == decode_base58(TOKEN_PROGRAM)
        # Path-qualified Rust spelling works too.
        assert serialize_seed_value(TOKEN_PROGRAM, "solana_pubkey::Pubkey") == decoded

    def test_rejects_non_pubkey_strings_for_pubkey_seeds(self):
        with pytest.raises(InstructionError, match="expected 32"):
            serialize_seed_value("abc", "pubkey")
        with pytest.raises(InstructionError, match="base58 string"):
            serialize_seed_value(42, "pubkey")

    def test_utf8_encodes_typed_string_seeds_without_base58_guessing(self):
        # 44 chars: the heuristic path would base58-decode this; typed must not.
        forty_four = "a" * 44
        assert serialize_seed_value(forty_four, "string") == forty_four.encode("utf-8")


class TestSerializeSeedValueUntyped:
    def test_passes_raw_bytes_through(self):
        assert serialize_seed_value(bytes([1, 2, 3])) == bytes([1, 2, 3])
        assert serialize_seed_value([1, 2, 255]) == bytes([1, 2, 255])

    def test_tries_base58_for_43_44_char_strings_and_utf8_otherwise(self):
        assert len(serialize_seed_value(TOKEN_PROGRAM)) == 32
        assert serialize_seed_value("treasury") == b"treasury"
        assert serialize_seed_value("abc") == b"abc"

    def test_encodes_numbers_as_8_byte_little_endian(self):
        assert list(serialize_seed_value(256)) == [0, 1, 0, 0, 0, 0, 0, 0]
        assert list(serialize_seed_value(1)) == [1, 0, 0, 0, 0, 0, 0, 0]


class TestFindProgramAddress:
    def test_matches_the_ts_and_rust_golden_vectors(self):
        u64 = lambda n: n.to_bytes(8, "little")
        vectors = [
            ([b"treasury"], TOKEN_PROGRAM,
             "GLCzvhmavoYmT1Z4u8afD3G2yo5ro9xcPRnNEYxrtutP", 255),
            ([b"state", decode_base58(WSOL_MINT)], TOKEN_PROGRAM,
             "HqK1X4NqLXxDwgMhTiHwMnfSMh19jdZj2VBmUdPAdBeS", 254),
            ([b"state", decode_base58(TOKEN_PROGRAM)], TOKEN_PROGRAM,
             "8LyFNDyuvNjb7SaM6WDF3D8to23F6csEFZFWcE1qHq39", 254),
            ([bytes([1, 2, 255])], TOKEN_PROGRAM,
             "5AtDnwsRPbCgdHszHDXZ6qDKxhqnKgFmFAN963EfKNZV", 255),
            ([b"proposal", u64(7)], TOKEN_PROGRAM,
             "2EegVtjqSVQuHNtaC8aeMb6Yh5geKkLkQb4enGcLpeJ5", 254),
            ([b"proposal", u64(9)], TOKEN_PROGRAM,
             "3u4hmmZVg3SuMPsXzz7mqi26vaD8S8Q8tqxoddDRJRoG", 255),
            ([b"proposal", u64(11)], TOKEN_PROGRAM,
             "6ipucxbyu3Gc2dbzT7Ban6qsRJPmWbU3ooAbcef3Lhnu", 253),
            ([b"miner", decode_base58(WSOL_MINT)], TOKEN_PROGRAM,
             "AUqzPTU2HC744MrZxJArww9M1e6Jx7augTAoNdnMiGrC", 255),
            ([b"inner"], TOKEN_PROGRAM,
             "BuLr2Mg6aA42TiyThpYoW4V9fkVBNaSURDPR1SjxDXcG", 254),
            # Associated-token-account shape: [owner, token program, mint].
            ([decode_base58(WSOL_MINT), decode_base58(TOKEN_PROGRAM), decode_base58(WSOL_MINT)],
             ATA_PROGRAM, "5o9nTwSiofKC5DnLiv2gsjPYmGNgh2hAjieyAzyUuwi2", 251),
            ([], SYSTEM_PROGRAM,
             "Cu7NwqCXSmsR5vgGA3Vw9uYVViPi3kQvkbKByVQ8nPY9", 255),
        ]
        for seeds, program_id, expected_address, expected_bump in vectors:
            assert find_program_address(seeds, program_id) == (
                expected_address,
                expected_bump,
            )

    def test_is_deterministic(self):
        first = find_program_address([b"treasury"], TOKEN_PROGRAM)
        second = find_program_address([b"treasury"], TOKEN_PROGRAM)
        assert first == second
        assert len(decode_base58(first[0])) == 32

    def test_produces_different_addresses_for_different_seeds(self):
        a, _ = find_program_address([b"a"], TOKEN_PROGRAM)
        b, _ = find_program_address([b"b"], TOKEN_PROGRAM)
        assert a != b

    def test_rejects_more_than_16_seeds_and_oversized_seeds(self):
        with pytest.raises(InstructionError, match="16 seeds"):
            find_program_address([b"x"] * 17, TOKEN_PROGRAM)
        with pytest.raises(InstructionError, match="maximum length"):
            find_program_address([bytes(33)], TOKEN_PROGRAM)

    def test_rejects_invalid_program_ids(self):
        with pytest.raises(InstructionError, match="Invalid pubkey"):
            find_program_address([b"x"], "not-base58!")
        with pytest.raises(InstructionError, match="32 bytes"):
            find_program_address([b"x"], "abc")


class TestDerivePda:
    def test_derives_from_literal_and_arg_ref_seeds(self):
        config = PdaConfig(
            seeds=[LiteralSeed("proposal"), ArgRefSeed("transactionIndex", "u64")],
            program_id=TOKEN_PROGRAM,
        )
        assert derive_pda(config, args={"transactionIndex": 7}) == (
            "2EegVtjqSVQuHNtaC8aeMb6Yh5geKkLkQb4enGcLpeJ5",
            254,
        )

    def test_falls_back_to_resolve_inputs_for_arg_ref_seeds(self):
        config = PdaConfig(
            seeds=[LiteralSeed("proposal"), ArgRefSeed("transactionIndex", "u64")],
            program_id=TOKEN_PROGRAM,
        )
        assert derive_pda(config, resolve={"transactionIndex": 9})[0] == (
            "3u4hmmZVg3SuMPsXzz7mqi26vaD8S8Q8tqxoddDRJRoG"
        )

    def test_derives_from_nested_arg_paths(self):
        config = PdaConfig(
            seeds=[LiteralSeed("proposal"), ArgRefSeed("args.transactionIndex", "u64")],
            program_id=TOKEN_PROGRAM,
        )
        address, _ = derive_pda(config, args={"args": {"transactionIndex": 7}})
        assert address == "2EegVtjqSVQuHNtaC8aeMb6Yh5geKkLkQb4enGcLpeJ5"

    def test_derives_from_account_ref_and_bytes_seeds(self):
        config = PdaConfig(
            seeds=[LiteralSeed("state"), AccountRefSeed("authority")],
            program_id=TOKEN_PROGRAM,
        )
        address, bump = derive_pda(config, accounts={"authority": WSOL_MINT})
        assert (address, bump) == ("HqK1X4NqLXxDwgMhTiHwMnfSMh19jdZj2VBmUdPAdBeS", 254)

        raw = PdaConfig(seeds=[BytesSeed(bytes([1, 2, 255]))], program_id=TOKEN_PROGRAM)
        assert derive_pda(raw)[0] == "5AtDnwsRPbCgdHszHDXZ6qDKxhqnKgFmFAN963EfKNZV"

    def test_config_program_id_wins_over_the_fallback(self):
        config = PdaConfig(seeds=[LiteralSeed("treasury")], program_id=TOKEN_PROGRAM)
        address, _ = derive_pda(config, program_id=SYSTEM_PROGRAM)
        assert address == "GLCzvhmavoYmT1Z4u8afD3G2yo5ro9xcPRnNEYxrtutP"

        fallback_only = PdaConfig(seeds=[LiteralSeed("treasury")])
        assert derive_pda(fallback_only, program_id=TOKEN_PROGRAM)[0] == address

    def test_fails_closed_on_missing_inputs(self):
        no_program = PdaConfig(seeds=[LiteralSeed("treasury")])
        with pytest.raises(InstructionError, match="no program ID"):
            derive_pda(no_program)

        arg_ref = PdaConfig(
            seeds=[ArgRefSeed("transactionIndex")], program_id=TOKEN_PROGRAM
        )
        with pytest.raises(InstructionError, match="Missing arg for PDA seed"):
            derive_pda(arg_ref)

        account_ref = PdaConfig(
            seeds=[AccountRefSeed("authority")], program_id=TOKEN_PROGRAM
        )
        with pytest.raises(InstructionError, match="Missing account for PDA seed"):
            derive_pda(account_ref)
