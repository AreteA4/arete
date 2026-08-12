"""Account resolution ordering and semantics.

Ported from typescript/core/src/instructions/instructions.test.ts
(resolveAccounts suite) and rust/arete-a4-sdk/src/instruction/resolver.rs
tests, including topological PDA ordering, payer/override semantics, and
Anchor's optional-account placeholder convention.
"""

from __future__ import annotations

import pytest

from arete.instructions import (
    AccountMeta,
    AccountRefSeed,
    ArgRefSeed,
    BytesSeed,
    InstructionError,
    Known,
    LiteralSeed,
    Pda,
    PdaConfig,
    Signer,
    UserProvided,
    find_program_address,
    decode_base58,
    resolve_accounts,
    validate_account_resolution,
)

SYSTEM_PROGRAM = "11111111111111111111111111111111"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
WSOL_MINT = "So11111111111111111111111111111111111111112"

STATE_WSOL_PDA = "HqK1X4NqLXxDwgMhTiHwMnfSMh19jdZj2VBmUdPAdBeS"
BYTES_PDA = "5AtDnwsRPbCgdHszHDXZ6qDKxhqnKgFmFAN963EfKNZV"
PROPOSAL_7_PDA = "2EegVtjqSVQuHNtaC8aeMb6Yh5geKkLkQb4enGcLpeJ5"
PROPOSAL_9_PDA = "3u4hmmZVg3SuMPsXzz7mqi26vaD8S8Q8tqxoddDRJRoG"
INNER_PDA = "BuLr2Mg6aA42TiyThpYoW4V9fkVBNaSURDPR1SjxDXcG"
OUTER_PDA = "36BupGFJ7pYyDHEyeGBuyjgQnzjxwNm1uhi92qyh8Qxy"


def meta(name, resolution, *, signer=False, writable=False, optional=False):
    return AccountMeta(
        name=name,
        is_signer=signer,
        is_writable=writable,
        resolution=resolution,
        is_optional=optional,
    )


def signer_meta(name):
    return meta(name, Signer(), signer=True, writable=True)


class TestResolveAccounts:
    def test_resolves_signer_known_and_user_provided_categories(self):
        metas = [
            signer_meta("authority"),
            meta("systemProgram", Known(SYSTEM_PROGRAM)),
            meta("mint", UserProvided()),
        ]
        result = resolve_accounts(
            metas, {}, overrides={"mint": TOKEN_PROGRAM}, payer=WSOL_MINT
        )
        validate_account_resolution(result)
        assert [a.name for a in result.accounts] == ["authority", "systemProgram", "mint"]
        assert result.accounts[0].address == WSOL_MINT
        assert result.accounts[0].is_signer is True
        assert result.accounts[1].address == SYSTEM_PROGRAM
        assert result.accounts[2].address == TOKEN_PROGRAM

    def test_prefers_explicit_signer_overrides_over_the_payer(self):
        metas = [signer_meta("authority"), meta("mint", UserProvided())]
        result = resolve_accounts(
            metas,
            {},
            overrides={"authority": TOKEN_PROGRAM, "mint": WSOL_MINT},
            payer=WSOL_MINT,
        )
        validate_account_resolution(result)
        assert [a.address for a in result.accounts] == [TOKEN_PROGRAM, WSOL_MINT]

    def test_derives_a_pda_referencing_a_signer_and_keeps_original_order(self):
        metas = [
            signer_meta("authority"),
            meta(
                "state",
                Pda(
                    PdaConfig(
                        seeds=[LiteralSeed("state"), AccountRefSeed("authority")],
                        program_id=TOKEN_PROGRAM,
                    )
                ),
                writable=True,
            ),
        ]
        result = resolve_accounts(metas, {}, payer=WSOL_MINT)
        validate_account_resolution(result)
        # Original (instruction) order is preserved even though PDAs resolve later.
        assert [a.name for a in result.accounts] == ["authority", "state"]
        assert result.accounts[1].address == STATE_WSOL_PDA

    def test_derives_a_pda_from_raw_byte_seeds(self):
        metas = [
            meta(
                "config",
                Pda(PdaConfig(seeds=[BytesSeed(bytes([1, 2, 255]))], program_id=TOKEN_PROGRAM)),
            )
        ]
        result = resolve_accounts(metas, {})
        validate_account_resolution(result)
        assert result.accounts[0].address == BYTES_PDA

    def test_derives_a_pda_from_a_nested_arg_path(self):
        metas = [
            meta(
                "proposal",
                Pda(
                    PdaConfig(
                        seeds=[
                            LiteralSeed("proposal"),
                            ArgRefSeed("args.transactionIndex", "u64"),
                        ],
                        program_id=TOKEN_PROGRAM,
                    )
                ),
                writable=True,
            )
        ]
        result = resolve_accounts(metas, {"args": {"transactionIndex": 7}})
        validate_account_resolution(result)
        assert result.accounts[0].address == PROPOSAL_7_PDA

    def test_derives_a_pda_from_helper_only_resolve_inputs(self):
        metas = [
            meta(
                "proposal",
                Pda(
                    PdaConfig(
                        seeds=[
                            LiteralSeed("proposal"),
                            ArgRefSeed("transactionIndex", "u64"),
                        ],
                        program_id=TOKEN_PROGRAM,
                    )
                ),
                writable=True,
            )
        ]
        result = resolve_accounts(metas, {}, resolve={"transactionIndex": 9})
        validate_account_resolution(result)
        assert result.accounts[0].address == PROPOSAL_9_PDA

    def test_errors_when_a_pda_seed_argument_is_missing(self):
        metas = [
            meta(
                "proposal",
                Pda(PdaConfig(seeds=[ArgRefSeed("transactionIndex")], program_id=TOKEN_PROGRAM)),
            )
        ]
        with pytest.raises(
            InstructionError, match="missing argument: transactionIndex"
        ):
            resolve_accounts(metas, {})

    def test_resolves_pda_dependencies_in_topological_order(self):
        # "outer" is declared before "inner" but depends on it via AccountRefSeed.
        metas = [
            meta(
                "outer",
                Pda(
                    PdaConfig(
                        seeds=[LiteralSeed("outer"), AccountRefSeed("inner")],
                        program_id=TOKEN_PROGRAM,
                    )
                ),
            ),
            meta(
                "inner",
                Pda(PdaConfig(seeds=[LiteralSeed("inner")], program_id=TOKEN_PROGRAM)),
            ),
        ]
        result = resolve_accounts(metas, {})
        validate_account_resolution(result)
        assert [a.name for a in result.accounts] == ["outer", "inner"]
        assert result.accounts[1].address == INNER_PDA
        assert result.accounts[0].address == OUTER_PDA
        # Cross-check the golden vector against a direct derivation.
        assert find_program_address(
            [b"outer", decode_base58(INNER_PDA)], TOKEN_PROGRAM
        )[0] == OUTER_PDA

    def test_rejects_circular_pda_dependencies(self):
        def pda_ref(name, dep):
            return meta(
                name,
                Pda(PdaConfig(seeds=[AccountRefSeed(dep)], program_id=TOKEN_PROGRAM)),
            )

        with pytest.raises(InstructionError, match="Circular dependency"):
            resolve_accounts([pda_ref("a", "b"), pda_ref("b", "a")], {})

    def test_errors_when_a_pda_has_no_program_id(self):
        metas = [meta("state", Pda(PdaConfig(seeds=[LiteralSeed("state")])))]
        with pytest.raises(InstructionError, match='Cannot derive PDA for "state"'):
            resolve_accounts(metas, {})

    def test_reports_missing_required_user_provided_accounts(self):
        metas = [meta("mint", UserProvided())]
        result = resolve_accounts(metas, {})
        assert result.missing == ["mint"]
        assert result.accounts == []
        with pytest.raises(InstructionError, match="Missing required accounts"):
            validate_account_resolution(result)

    def test_substitutes_the_program_id_for_omitted_non_trailing_optionals(self):
        metas = [
            signer_meta("authority"),
            meta("referrer", UserProvided(), optional=True),
            meta("mint", UserProvided()),
        ]
        result = resolve_accounts(
            metas,
            {},
            overrides={"mint": TOKEN_PROGRAM},
            payer=WSOL_MINT,
            program_id=SYSTEM_PROGRAM,
        )
        validate_account_resolution(result)
        assert [a.name for a in result.accounts] == ["authority", "referrer", "mint"]
        # Anchor convention: omitted optional in a non-trailing slot = program id.
        assert result.accounts[1].address == SYSTEM_PROGRAM
        assert result.accounts[1].is_signer is False
        assert result.accounts[1].is_writable is False

    def test_errors_for_omitted_non_trailing_optionals_without_a_program_id(self):
        metas = [
            signer_meta("authority"),
            meta("referrer", UserProvided(), optional=True),
            meta("mint", UserProvided()),
        ]
        with pytest.raises(InstructionError, match="placeholder"):
            resolve_accounts(
                metas, {}, overrides={"mint": TOKEN_PROGRAM}, payer=WSOL_MINT
            )

    def test_drops_omitted_trailing_optional_accounts(self):
        metas = [
            signer_meta("authority"),
            meta("referrer", UserProvided(), optional=True),
        ]
        result = resolve_accounts(
            metas, {}, payer=WSOL_MINT, program_id=SYSTEM_PROGRAM
        )
        validate_account_resolution(result)
        assert [a.name for a in result.accounts] == ["authority"]

    def test_resolves_provided_optional_accounts_normally(self):
        metas = [
            signer_meta("authority"),
            meta("referrer", UserProvided(), optional=True),
            meta("mint", UserProvided()),
        ]
        result = resolve_accounts(
            metas,
            {},
            overrides={"referrer": WSOL_MINT, "mint": TOKEN_PROGRAM},
            payer=WSOL_MINT,
            program_id=SYSTEM_PROGRAM,
        )
        assert [a.address for a in result.accounts] == [
            WSOL_MINT,
            WSOL_MINT,
            TOKEN_PROGRAM,
        ]

    def test_missing_signer_without_payer_is_reported(self):
        metas = [signer_meta("authority")]
        result = resolve_accounts(metas, {})
        assert result.missing == ["authority"]
