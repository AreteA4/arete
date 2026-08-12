"""InstructionHandler.build (splitParams semantics) and error metadata.

Ported from typescript/core/src/instructions/instructions.test.ts
(createInstructionHandler + buildInstruction suite),
rust/arete-a4-sdk/src/instruction/handler.rs tests, and the error-parser
metadata lookup vectors.
"""

from __future__ import annotations

import pytest

from arete.instructions import (
    AccountMeta,
    AccountRefSeed,
    ArgRefSeed,
    ArgSchema,
    BuiltAccountMeta,
    ErrorMetadata,
    InstructionError,
    InstructionHandler,
    LiteralSeed,
    Pda,
    PdaConfig,
    Signer,
    UserProvided,
    format_program_error,
    parse_program_error,
)

SYSTEM_PROGRAM = "11111111111111111111111111111111"
TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
WSOL_MINT = "So11111111111111111111111111111111111111112"

# Golden vectors generated with the TS reference stack (@noble/ed25519 + bs58).
STATE_WSOL_PDA = "HqK1X4NqLXxDwgMhTiHwMnfSMh19jdZj2VBmUdPAdBeS"  # ["state", WSOL]
STATE_TOKEN_PDA = "8LyFNDyuvNjb7SaM6WDF3D8to23F6csEFZFWcE1qHq39"  # ["state", TOKEN]
PROPOSAL_11_PDA = "6ipucxbyu3Gc2dbzT7Ban6qsRJPmWbU3ooAbcef3Lhnu"  # ["proposal", u64(11)]


def make_handler():
    return InstructionHandler(
        program_id=TOKEN_PROGRAM,
        discriminator=bytes([1]),
        accounts=[
            AccountMeta("authority", True, True, Signer()),
            AccountMeta("mint", False, False, UserProvided()),
            AccountMeta(
                "state",
                False,
                True,
                Pda(PdaConfig(seeds=[LiteralSeed("state"), AccountRefSeed("authority")])),
            ),
        ],
        args=[ArgSchema("amount", "u64")],
        errors=[ErrorMetadata(6000, "Boom", "boom")],
    )


class TestBuild:
    def test_splits_merged_params_into_args_and_account_overrides(self):
        built = make_handler().build(
            {"amount": 100, "mint": SYSTEM_PROGRAM}, payer=WSOL_MINT
        )
        assert built.program_id == TOKEN_PROGRAM
        assert [a.pubkey for a in built.accounts] == [
            WSOL_MINT,  # authority (signer)
            SYSTEM_PROGRAM,  # mint (user-provided)
            STATE_WSOL_PDA,  # state (PDA derived from authority)
        ]
        # discriminator [1] + u64 100 little-endian.
        assert list(built.data) == [1, 100, 0, 0, 0, 0, 0, 0, 0]
        assert built.accounts[0].is_signer is True
        assert built.accounts[0].is_writable is True
        assert built.accounts[1].is_signer is False

    def test_lets_merged_params_override_a_signer_slot_explicitly(self):
        built = make_handler().build(
            {"amount": 7, "authority": TOKEN_PROGRAM, "mint": SYSTEM_PROGRAM},
            payer=WSOL_MINT,
        )
        assert [a.pubkey for a in built.accounts] == [
            TOKEN_PROGRAM,
            SYSTEM_PROGRAM,
            STATE_TOKEN_PDA,
        ]

    def test_options_account_overrides_win_over_params(self):
        built = make_handler().build(
            {"amount": 1, "mint": SYSTEM_PROGRAM},
            payer=WSOL_MINT,
            accounts={"mint": TOKEN_PROGRAM},
        )
        assert built.accounts[1].pubkey == TOKEN_PROGRAM

    def test_rejects_non_string_account_params(self):
        with pytest.raises(InstructionError, match="not a known argument"):
            make_handler().build({"amount": 1, "mint": 42}, payer=WSOL_MINT)

    def test_accepts_helper_only_resolve_inputs_for_pda_derivation(self):
        handler = InstructionHandler(
            program_id=TOKEN_PROGRAM,
            discriminator=bytes([2]),
            accounts=[
                AccountMeta("authority", True, True, Signer()),
                AccountMeta(
                    "proposal",
                    False,
                    True,
                    Pda(
                        PdaConfig(
                            seeds=[
                                LiteralSeed("proposal"),
                                ArgRefSeed("transactionIndex", "u64"),
                            ]
                        )
                    ),
                ),
            ],
            args=[ArgSchema("amount", "u64")],
        )
        built = handler.build(
            {"amount": 5, "resolve": {"transactionIndex": 11}}, payer=WSOL_MINT
        )
        assert [a.pubkey for a in built.accounts] == [WSOL_MINT, PROPOSAL_11_PDA]
        assert list(built.data) == [2, 5, 0, 0, 0, 0, 0, 0, 0]

    def test_treats_none_resolve_as_absent(self):
        built = make_handler().build(
            {"amount": 1, "mint": SYSTEM_PROGRAM, "resolve": None}, payer=WSOL_MINT
        )
        assert len(built.accounts) == 3

    def test_rejects_non_mapping_resolve_inputs(self):
        for bad in [1, [1], "x"]:
            with pytest.raises(InstructionError, match="resolve"):
                make_handler().build(
                    {"amount": 1, "mint": SYSTEM_PROGRAM, "resolve": bad},
                    payer=WSOL_MINT,
                )

    def test_rejects_unknown_parameter_names(self):
        with pytest.raises(InstructionError) as excinfo:
            make_handler().build(
                {"amount": 1, "mint": SYSTEM_PROGRAM, "mnit": TOKEN_PROGRAM},
                payer=WSOL_MINT,
            )
        assert str(excinfo.value) == (
            'Unknown parameter "mnit". Expected one of args [amount] '
            "or accounts [authority, mint, state]"
        )

    def test_rejects_missing_required_args_instead_of_encoding_zeros(self):
        with pytest.raises(
            InstructionError, match='Missing required argument "amount"'
        ):
            make_handler().build({"mint": SYSTEM_PROGRAM}, payer=WSOL_MINT)

    def test_reports_missing_required_accounts(self):
        with pytest.raises(InstructionError) as excinfo:
            make_handler().build({"amount": 1}, payer=WSOL_MINT)
        assert str(excinfo.value) == "Missing required accounts: mint"

    def test_missing_payer_reports_the_signer_slot(self):
        with pytest.raises(InstructionError, match="authority"):
            make_handler().build({"amount": 1, "mint": SYSTEM_PROGRAM})

    def test_rejects_non_mapping_params(self):
        with pytest.raises(InstructionError, match="expected a mapping"):
            make_handler().build([1, 2], payer=WSOL_MINT)  # type: ignore[arg-type]

    def test_appends_remaining_accounts_after_the_declared_accounts(self):
        extra = BuiltAccountMeta(pubkey=TOKEN_PROGRAM, is_signer=False, is_writable=True)
        built = make_handler().build(
            {"amount": 1, "mint": SYSTEM_PROGRAM},
            payer=WSOL_MINT,
            remaining_accounts=[extra],
        )
        assert len(built.accounts) == 4  # 3 declared + 1 remaining
        assert built.accounts[3] == extra

    def test_option_args_may_be_omitted(self):
        handler = InstructionHandler(
            program_id=TOKEN_PROGRAM,
            discriminator=bytes([3]),
            accounts=[AccountMeta("authority", True, True, Signer())],
            args=[ArgSchema("maybe", {"option": "u8"})],
        )
        built = handler.build({}, payer=WSOL_MINT)
        assert list(built.data) == [3, 0]

    def test_validates_resolved_addresses_as_pubkeys(self):
        with pytest.raises(InstructionError, match="Invalid pubkey"):
            make_handler().build(
                {"amount": 1, "mint": "not-a-pubkey"}, payer=WSOL_MINT
            )


class TestErrorMetadata:
    def test_looks_up_idl_errors_by_code(self):
        handler = make_handler()
        found = handler.error_for_code(6000)
        assert found is not None and found.name == "Boom"
        assert handler.error_for_code(6001) is None

    def test_parse_program_error_maps_known_codes_to_metadata(self):
        errors = [ErrorMetadata(6000, "SlippageExceeded", "Slippage tolerance exceeded")]
        parsed = parse_program_error(6000, errors)
        assert parsed == ErrorMetadata(6000, "SlippageExceeded", "Slippage tolerance exceeded")
        assert format_program_error(parsed) == (
            "SlippageExceeded (6000): Slippage tolerance exceeded"
        )

    def test_parse_program_error_falls_back_to_a_synthetic_error(self):
        parsed = parse_program_error(12345, [])
        assert parsed == ErrorMetadata(12345, "CustomError12345", "Unknown error with code 12345")
