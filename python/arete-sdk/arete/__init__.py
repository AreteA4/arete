"""Arete Python SDK — idiomatic Python projection of the Arete core SDK API.

See docs/internal/sdk-core-api.md (canonical surface) and
docs/internal/sdk-python-alignment.md (Python projection).

    import arete
    a4 = await arete.Arete.connect(STACK, wallet=wallet)
    async for round in a4.views.ore_round.latest.use(take=10):
        ...
"""

from __future__ import annotations

__version__ = "0.4.0"


class _Unset:
    """Sentinel distinguishing "no active subscription" from an empty result."""

    _instance = None

    def __new__(cls):
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def __repr__(self) -> str:
        return "arete.UNSET"

    def __bool__(self) -> bool:
        return False


UNSET = _Unset()

# NOTE: UNSET must exist before the submodule imports below (arete.views
# imports it at module load).

from arete.errors import (  # noqa: E402
    AreteConnectionError,
    AreteError,
    AuthError,
    HttpRequestError,
    ProcessedSlotTimeoutError,
    SubscriptionError,
)
from arete.auth import AuthConfig  # noqa: E402
from arete.wire import RichUpdate, Update  # noqa: E402
from arete.views import (  # noqa: E402
    DEFAULT_INITIAL_DATA_TIMEOUT,
    InitialDataTimeoutError,
    ListViewHandle,
    StateViewHandle,
    ViewDef,
    ViewsNamespace,
)
from arete.chain import (  # noqa: E402
    ChainClient,
    ChainClock,
    ChainError,
    HttpChainClient,
    MintAccountInfo,
    NativeBalanceInfo,
    RawAccountInfo,
    TokenAccountInfo,
    TokenBalanceInfo,
)
from arete.transactions import (  # noqa: E402
    HttpTransactionTransport,
    TransactionTransport,
    TransactionTransportError,
)
from arete.gateway import (  # noqa: E402
    HostedSolanaGatewayBindings,
    create_hosted_solana_gateway_transports,
)
from arete.amounts import (  # noqa: E402
    format_raw_to_ui,
    get_mint_decimals,
    parse_ui_amount_to_raw,
    resolve_amount,
    resolve_amount_to_raw,
    resolve_amounts_to_raw,
    to_raw_amount,
)
from arete.spl import (  # noqa: E402
    ASSOCIATED_TOKEN_PROGRAM_ADDRESS,
    SPL_TOKEN_PROGRAM_ADDRESS,
    SYSTEM_PROGRAM_ADDRESS,
    TOKEN_2022_PROGRAM_ADDRESS,
    derive_associated_token_account,
    resolve_token_program_address,
)
from arete.instructions import (  # noqa: E402
    BuiltAccountMeta,
    BuiltInstruction,
    ErrorMetadata,
    InstructionError,
    InstructionHandler,
    PdaConfig,
    derive_pda,
    find_program_address,
    format_program_error,
    parse_program_error,
)
from arete.read import (  # noqa: E402
    AccountBatchItem,
    AccountBatchResult,
    AccountReader,
    ProgramAccountReadDef,
    ProgramQueryDef,
    ReadRequestError,
    StackQueryDef,
)
from arete.program_read_transport import (  # noqa: E402
    ProgramReadDescriptor,
    ProgramReleaseReference,
    validate_program_read_descriptor,
)
from arete.wallet import (  # noqa: E402
    MAX_TRANSACTION_BYTES,
    TRANSACTION_VERSIONS,
    V1_MAX_ACCOUNTS,
    V1_MAX_INSTRUCTIONS,
    V1_MAX_SIGNATURES,
    V1_MAX_TRANSACTION_BYTES,
    ConfirmedTransactionOutcome,
    SendOptions,
    SendResult,
    TransactionFailureOutcome,
    TransactionInspectionResult,
    TransactionResourceOptions,
    UnsupportedTransactionVersionError,
    WalletAdapter,
    WalletError,
    WalletExecutionContext,
    ensure_transaction_version_supported,
    wallet_supported_transaction_versions,
)
from arete.operations import (  # noqa: E402
    OperationCallbackError,
    OperationExecutionError,
    OperationReceipt,
    OperationTransactionReceipt,
    PreparedFlow,
    PreparedInstruction,
    PreparedOperation,
    PreparedTransaction,
    PreparedTransactionBody,
    SignerRegistry,
    TransactionExecutionError,
    append_flow_transactions,
    append_transaction_instructions,
    create_prepared_flow,
    create_prepared_instruction,
    create_prepared_transaction,
    create_prepared_transaction_body,
    create_signer_registry,
    describe_prepared_operation,
    execute_prepared_operation,
    format_prepared_operation,
    get_transaction_failure_outcome,
    inspect_prepared_operation,
    prepend_flow_transaction_instructions,
    prepend_transaction_instructions,
    unwrap_operation_execution_error,
)
from arete.stack import (  # noqa: E402
    ConnectedProgram,
    Operation,
    ProgramDef,
    ProgramOperationContext,
    ProgramOperations,
    StackDef,
    StackEndpoints,
    flow_operation,
    instruction_operation,
    transaction_operation,
    with_programs,
)
from arete.extensions import (  # noqa: E402
    apply_connected_stack_extensions,
    extend_program,
    extend_programs,
    extend_stack,
)
from arete.client import Arete  # noqa: E402
from arete.session import Session, SessionError, create_session  # noqa: E402

__all__ = [
    "UNSET",
    "__version__",
    # client & session
    "Arete",
    "Session",
    "SessionError",
    "create_session",
    # errors
    "AreteError",
    "AreteConnectionError",
    "AuthError",
    "ChainError",
    "HttpRequestError",
    "InitialDataTimeoutError",
    "InstructionError",
    "OperationCallbackError",
    "OperationExecutionError",
    "ProcessedSlotTimeoutError",
    "ReadRequestError",
    "SubscriptionError",
    "TransactionExecutionError",
    "TransactionTransportError",
    "WalletError",
    # auth
    "AuthConfig",
    # views & updates
    "Update",
    "RichUpdate",
    "ViewDef",
    "ViewsNamespace",
    "DEFAULT_INITIAL_DATA_TIMEOUT",
    "ListViewHandle",
    "StateViewHandle",
    # stack binding model
    "StackDef",
    "StackEndpoints",
    "ProgramDef",
    "ProgramOperations",
    "ProgramOperationContext",
    "ConnectedProgram",
    "with_programs",
    # extensions
    "extend_stack",
    "extend_program",
    "extend_programs",
    "apply_connected_stack_extensions",
    "Operation",
    "instruction_operation",
    "transaction_operation",
    "flow_operation",
    # wallet & outcomes
    "WalletAdapter",
    "WalletExecutionContext",
    "SendOptions",
    "SendResult",
    "TransactionInspectionResult",
    "TransactionResourceOptions",
    "UnsupportedTransactionVersionError",
    "TRANSACTION_VERSIONS",
    "MAX_TRANSACTION_BYTES",
    "V1_MAX_TRANSACTION_BYTES",
    "V1_MAX_SIGNATURES",
    "V1_MAX_ACCOUNTS",
    "V1_MAX_INSTRUCTIONS",
    "ensure_transaction_version_supported",
    "wallet_supported_transaction_versions",
    "ConfirmedTransactionOutcome",
    "TransactionFailureOutcome",
    "get_transaction_failure_outcome",
    # prepared operations
    "PreparedInstruction",
    "PreparedTransaction",
    "PreparedFlow",
    "PreparedOperation",
    "PreparedTransactionBody",
    "create_prepared_instruction",
    "create_prepared_transaction",
    "create_prepared_flow",
    "create_prepared_transaction_body",
    "prepend_transaction_instructions",
    "append_transaction_instructions",
    "append_flow_transactions",
    "prepend_flow_transaction_instructions",
    "execute_prepared_operation",
    "inspect_prepared_operation",
    "describe_prepared_operation",
    "format_prepared_operation",
    "unwrap_operation_execution_error",
    "OperationReceipt",
    "OperationTransactionReceipt",
    "SignerRegistry",
    "create_signer_registry",
    # chain & transactions
    "ChainClient",
    "HttpChainClient",
    "ChainClock",
    "RawAccountInfo",
    "MintAccountInfo",
    "TokenAccountInfo",
    "TokenBalanceInfo",
    "NativeBalanceInfo",
    "TransactionTransport",
    "HttpTransactionTransport",
    "HostedSolanaGatewayBindings",
    "create_hosted_solana_gateway_transports",
    # amounts & SPL
    "parse_ui_amount_to_raw",
    "format_raw_to_ui",
    "to_raw_amount",
    "get_mint_decimals",
    "resolve_amount",
    "resolve_amount_to_raw",
    "resolve_amounts_to_raw",
    "SPL_TOKEN_PROGRAM_ADDRESS",
    "TOKEN_2022_PROGRAM_ADDRESS",
    "ASSOCIATED_TOKEN_PROGRAM_ADDRESS",
    "SYSTEM_PROGRAM_ADDRESS",
    "derive_associated_token_account",
    "resolve_token_program_address",
    # instruction runtime
    "InstructionHandler",
    "BuiltInstruction",
    "BuiltAccountMeta",
    "ErrorMetadata",
    "PdaConfig",
    "derive_pda",
    "find_program_address",
    "parse_program_error",
    "format_program_error",
    # program reads
    "AccountReader",
    "AccountBatchItem",
    "AccountBatchResult",
    "ProgramAccountReadDef",
    "ProgramQueryDef",
    "StackQueryDef",
    "ProgramReadDescriptor",
    "ProgramReleaseReference",
    "validate_program_read_descriptor",
]
