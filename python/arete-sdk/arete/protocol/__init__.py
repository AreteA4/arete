"""Arete Protocols: call a hosted protocol deployment from Python.

``a4 install protocol <ref> --python`` generates a typed module for a
protocol: its definition (``<NAME>_PROTOCOL``), a publishable client
(``public`` steps only) and a secret client (every client-callable step).
This package is the runtime those modules build on::

    from arete import AuthConfig
    from token_raise_protocol import TokenRaiseClient

    async with TokenRaiseClient(auth=AuthConfig(publishable_key=key)) as client:
        result = await client.steps.contribute(
            instance=instance_id, wallet=wallet, args={"amount": "1000000"}
        )
        if result.ok:
            signed = sign_all(result.transactions)  # the wallet signs
            state = await client.submit(result.intent, signed)
        else:
            print(result.rejected)  # a guard code or an access code

``verify_receipt`` and ``verify_receipt_on_chain`` check execution receipts
(``client.get_receipt(signature)``) offline and against the transaction.

Mirror of ``typescript/core/src/protocol``.
"""

from __future__ import annotations

from typing import Any, Dict, Mapping, Optional, Sequence

from arete.protocol.client import (
    ARETE_API_URL_ENV,
    MAX_RECONNECT_DELAY_SECONDS,
    PROTOCOL_PREPARE_ANY_SCOPE,
    PROTOCOL_READ_SCOPE,
    PROTOCOL_SUBMIT_SCOPE,
    ProtocolClient,
    ProtocolEventStream,
    ProtocolSquads,
    SignMessage,
    is_protocol_access_rejection_code,
    is_protocol_state_rejection_code,
    resolve_protocol_endpoints,
)
from arete.protocol.handle import ProtocolInstanceHandle, ProtocolInstanceSnapshot
from arete.protocol.metadata_proof import (
    DEFAULT_PROOF_TTL_SECONDS,
    PROTOCOL_METADATA_PROOF_DOMAIN,
    PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS,
    ProtocolMetadataProofMessage,
    canonical_json_sha256,
    canonicalize_json,
    protocol_metadata_proof,
    protocol_metadata_proof_message,
)
from arete.protocol.receipt import (
    MEMO_PROGRAM_ID,
    MEMO_V2_PREFIX,
    PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1,
    PROTOCOL_RECEIPT_KEYS_SCHEMA_V1,
    RECEIPT_ERROR_CODES,
    ReceiptVerification,
    receipt_canonical_body,
    receipt_memo_id,
    receipt_signing_message,
    verify_receipt,
    verify_receipt_on_chain,
)
from arete.protocol.squads import (
    SQUADS_DASHBOARD_URL,
    SQUADS_SETUP_REQUIRED_CODE,
    SquadsAccountProposer,
    SquadsAccountStatus,
    SquadsAttachment,
    SquadsAttachmentDeployment,
    SquadsNotification,
    SquadsNotificationDeployment,
    SquadsAttachmentDetail,
    SquadsAttachmentProblem,
    SquadsAttachmentState,
    SquadsClient,
    SquadsConfigProposal,
    SquadsCreateMember,
    SquadsMember,
    SquadsPermission,
    SquadsPreparedTransaction,
    SquadsSetup,
    SquadsSetupRequired,
    SquadsSetupRequiredDetails,
    SquadsStepProposal,
    SquadsSubmitResult,
    squads_dashboard_url,
    squads_setup_required,
)
from arete.protocol.sse import ServerSentEvent, ServerSentEventParser
from arete.protocol.types import (
    EXECUTION_INTENT_SCHEMA_V1,
    PROTOCOL_ACCESS_REJECTION_CODES,
    PROTOCOL_STATE_REJECTION_CODES,
    ExecutionIntentV1,
    ProtocolAccessRejectionCode,
    ProtocolDefinition,
    ProtocolDeployment,
    ProtocolEventPayloadV1,
    ProtocolInstanceDetail,
    ProtocolInstancePage,
    ProtocolInstanceSummary,
    ProtocolInteger,
    ProtocolIntentState,
    ProtocolIntentStatus,
    ProtocolKeyClass,
    ProtocolLiveStack,
    ProtocolMachineKind,
    ProtocolMetadataFieldDefinition,
    ProtocolMetadataState,
    ProtocolMetadataUpdateResult,
    ProtocolParticipantDetail,
    ProtocolPrepared,
    ProtocolPreparedTransaction,
    ProtocolPrepareResult,
    ProtocolRejection,
    ProtocolStateRejectionCode,
    ProtocolStepAccess,
    ProtocolStepArgDescriptor,
    ProtocolStepAvailability,
    ProtocolStepDefinition,
    ProtocolStepScope,
    ProtocolStreamEvent,
    ProtocolTransactionFormat,
    ProtocolWalletActions,
    ProtocolWalletInstanceActions,
    SQUADS_PROPOSER_MISSING_EVENT,
    ProtocolSquadsAddProposerResult,
    ProtocolSquadsProposer,
    ProtocolSquadsProposerMissingData,
    ProtocolSquadsProposerProblem,
    ProtocolSquadsStatus,
)
from arete.protocol.webhooks import (
    WEBHOOK_ERROR_CODES,
    WEBHOOK_ID_HEADER,
    WEBHOOK_SIGNATURE_HEADER,
    WEBHOOK_TIMESTAMP_HEADER,
    WEBHOOK_TOLERANCE_SECONDS,
    VerifiedWebhook,
    WebhookVerificationError,
    sign_webhook,
    verify_webhook,
)


def decimal_args(
    args: Optional[Mapping[str, Any]], names: Sequence[str]
) -> Dict[str, Any]:
    """A copy of ``args`` with the integer arguments ``names`` (``u64``,
    ``i64``, unix timestamps ...) as decimal strings, the wire form. Strings
    pass through; lists convert element-wise."""
    out: Dict[str, Any] = dict(args or {})
    for name in names:
        if name not in out:
            continue
        value = out[name]
        if isinstance(value, int) and not isinstance(value, bool):
            out[name] = str(value)
        elif isinstance(value, (list, tuple)):
            out[name] = [
                str(item) if isinstance(item, int) and not isinstance(item, bool) else item
                for item in value
            ]
    return out


__all__ = [
    # client
    "ARETE_API_URL_ENV",
    "ProtocolClient",
    "resolve_protocol_endpoints",
    "ProtocolEventStream",
    "SignMessage",
    "PROTOCOL_READ_SCOPE",
    "PROTOCOL_SUBMIT_SCOPE",
    "PROTOCOL_PREPARE_ANY_SCOPE",
    "MAX_RECONNECT_DELAY_SECONDS",
    "is_protocol_access_rejection_code",
    "is_protocol_state_rejection_code",
    "decimal_args",
    # definitions and results
    "ProtocolDefinition",
    "ProtocolDeployment",
    "ProtocolInstanceHandle",
    "ProtocolInstanceSnapshot",
    "ProtocolLiveStack",
    "ProtocolMetadataFieldDefinition",
    "ProtocolStepDefinition",
    "ProtocolKeyClass",
    "ProtocolStepScope",
    "ProtocolStepAccess",
    "ProtocolMachineKind",
    "ProtocolTransactionFormat",
    "ProtocolInteger",
    "PROTOCOL_ACCESS_REJECTION_CODES",
    "ProtocolAccessRejectionCode",
    "PROTOCOL_STATE_REJECTION_CODES",
    "ProtocolStateRejectionCode",
    "ProtocolPrepared",
    "ProtocolPreparedTransaction",
    "ProtocolRejection",
    "ProtocolPrepareResult",
    "EXECUTION_INTENT_SCHEMA_V1",
    "ExecutionIntentV1",
    "ProtocolIntentStatus",
    "ProtocolIntentState",
    "ProtocolInstanceSummary",
    "ProtocolInstanceDetail",
    "ProtocolInstancePage",
    "ProtocolParticipantDetail",
    "ProtocolStepAvailability",
    "ProtocolStepArgDescriptor",
    "ProtocolWalletActions",
    "ProtocolWalletInstanceActions",
    "ProtocolMetadataState",
    "ProtocolMetadataUpdateResult",
    "ProtocolEventPayloadV1",
    "ProtocolStreamEvent",
    "SQUADS_PROPOSER_MISSING_EVENT",
    "ProtocolSquads",
    # account squads
    "SQUADS_DASHBOARD_URL",
    "SQUADS_SETUP_REQUIRED_CODE",
    "SquadsAccountProposer",
    "SquadsAccountStatus",
    "SquadsAttachment",
    "SquadsAttachmentDeployment",
    "SquadsNotification",
    "SquadsNotificationDeployment",
    "SquadsAttachmentDetail",
    "SquadsAttachmentProblem",
    "SquadsAttachmentState",
    "SquadsClient",
    "SquadsConfigProposal",
    "SquadsCreateMember",
    "SquadsMember",
    "SquadsPermission",
    "SquadsPreparedTransaction",
    "SquadsSetup",
    "SquadsSetupRequired",
    "SquadsSetupRequiredDetails",
    "SquadsStepProposal",
    "SquadsSubmitResult",
    "squads_dashboard_url",
    "squads_setup_required",
    "ProtocolSquadsAddProposerResult",
    "ProtocolSquadsProposer",
    "ProtocolSquadsProposerMissingData",
    "ProtocolSquadsProposerProblem",
    "ProtocolSquadsStatus",
    # metadata proofs
    "canonicalize_json",
    "canonical_json_sha256",
    "protocol_metadata_proof_message",
    "protocol_metadata_proof",
    "ProtocolMetadataProofMessage",
    "PROTOCOL_METADATA_PROOF_DOMAIN",
    "PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS",
    "DEFAULT_PROOF_TTL_SECONDS",
    # execution receipts
    "verify_receipt",
    "verify_receipt_on_chain",
    "receipt_canonical_body",
    "receipt_signing_message",
    "receipt_memo_id",
    "ReceiptVerification",
    "RECEIPT_ERROR_CODES",
    "PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1",
    "PROTOCOL_RECEIPT_KEYS_SCHEMA_V1",
    "MEMO_V2_PREFIX",
    "MEMO_PROGRAM_ID",
    # event streams
    "ServerSentEvent",
    "ServerSentEventParser",
    # webhooks
    "verify_webhook",
    "sign_webhook",
    "VerifiedWebhook",
    "WebhookVerificationError",
    "WEBHOOK_ID_HEADER",
    "WEBHOOK_TIMESTAMP_HEADER",
    "WEBHOOK_SIGNATURE_HEADER",
    "WEBHOOK_TOLERANCE_SECONDS",
    "WEBHOOK_ERROR_CODES",
]
