"""Generated entity models for the `OreStream` stack. Do not edit.

Wire payloads are snake_case and pass through untransformed; u64/u128 decimal
strings convert to `int`. `*_from_wire` builds full dataclasses (IDL struct
converters reject payloads missing a required key; entity converters leave
missing keys None); `*_patch_from_wire` converts only the keys present in a
patch. Fields fed by `#[capture]` mappings or event handlers arrive inside a
`CaptureWrapper` / `EventWrapper` envelope and are typed as such.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Dict, Generic, List, Mapping, Optional, TypeVar

_T = TypeVar("_T")

__all__ = [
    "EventWrapper",
    "event_wrapper_from_wire",
    "CaptureWrapper",
    "capture_wrapper_from_wire",
    "OreRoundId",
    "ore_round_id_from_wire",
    "ore_round_id_patch_from_wire",
    "OreRoundState",
    "ore_round_state_from_wire",
    "ore_round_state_patch_from_wire",
    "OreRoundResults",
    "ore_round_results_from_wire",
    "ore_round_results_patch_from_wire",
    "OreRoundMetrics",
    "ore_round_metrics_from_wire",
    "ore_round_metrics_patch_from_wire",
    "OreRoundTreasury",
    "ore_round_treasury_from_wire",
    "ore_round_treasury_patch_from_wire",
    "OreRoundEntropy",
    "ore_round_entropy_from_wire",
    "ore_round_entropy_patch_from_wire",
    "OreRound",
    "ore_round_from_wire",
    "ore_round_patch_from_wire",
    "Board",
    "board_from_wire",
    "board_patch_from_wire",
    "OreBoardId",
    "ore_board_id_from_wire",
    "ore_board_id_patch_from_wire",
    "OreBoardState",
    "ore_board_state_from_wire",
    "ore_board_state_patch_from_wire",
    "OreBoard",
    "ore_board_from_wire",
    "ore_board_patch_from_wire",
    "Treasury",
    "treasury_from_wire",
    "treasury_patch_from_wire",
    "OreTreasuryId",
    "ore_treasury_id_from_wire",
    "ore_treasury_id_patch_from_wire",
    "OreTreasuryState",
    "ore_treasury_state_from_wire",
    "ore_treasury_state_patch_from_wire",
    "OreTreasury",
    "ore_treasury_from_wire",
    "ore_treasury_patch_from_wire",
    "Miner",
    "miner_from_wire",
    "miner_patch_from_wire",
    "Automation",
    "automation_from_wire",
    "automation_patch_from_wire",
    "OreMinerId",
    "ore_miner_id_from_wire",
    "ore_miner_id_patch_from_wire",
    "OreMinerRewards",
    "ore_miner_rewards_from_wire",
    "ore_miner_rewards_patch_from_wire",
    "OreMinerState",
    "ore_miner_state_from_wire",
    "ore_miner_state_patch_from_wire",
    "OreMinerAutomation",
    "ore_miner_automation_from_wire",
    "ore_miner_automation_patch_from_wire",
    "OreMiner",
    "ore_miner_from_wire",
    "ore_miner_patch_from_wire",
    "AutomationConditions",
    "automation_conditions_from_wire",
    "automation_conditions_patch_from_wire",
    "OreAutomation",
    "ore_automation_from_wire",
    "ore_automation_patch_from_wire",
    "AdminConfig",
    "admin_config_from_wire",
    "admin_config_patch_from_wire",
    "ProtocolConfig",
    "protocol_config_from_wire",
    "protocol_config_patch_from_wire",
    "Config",
    "config_from_wire",
    "config_patch_from_wire",
    "Numeric",
    "numeric_from_wire",
    "numeric_patch_from_wire",
    "OreMinerAccount",
    "ore_miner_account_from_wire",
    "ore_miner_account_patch_from_wire",
    "Round",
    "round_from_wire",
    "round_patch_from_wire",
    "OreTreasuryAccount",
    "ore_treasury_account_from_wire",
    "ore_treasury_account_patch_from_wire",
    "Var",
    "var_from_wire",
    "var_patch_from_wire",
]

def _snake_key(key: str) -> str:
    out = []
    for index, ch in enumerate(key):
        if ch.isascii() and ch.isupper():
            if index != 0:
                out.append("_")
            out.append(ch.lower())
        else:
            out.append(ch)
    return "".join(out)


def _mapping(value: Any, context: str) -> Dict[str, Any]:
    if not isinstance(value, Mapping):
        raise TypeError(
            f"{context} payload must be a mapping, got {type(value).__name__}"
        )
    return {_snake_key(key): item for key, item in value.items()}


def _to_int(value: Any) -> Optional[int]:
    if value is None:
        return None
    if isinstance(value, bool):
        return int(value)
    if isinstance(value, int):
        return value
    if isinstance(value, (str, float)):
        return int(value)
    raise TypeError(f"Cannot convert {type(value).__name__} to int")


def _to_int_list(value: Any) -> Optional[List[Optional[int]]]:
    if value is None:
        return None
    return [_to_int(item) for item in value]


def _convert(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return converter(value)


def _convert_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [converter(item) for item in value]


def _convert_capture(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return capture_wrapper_from_wire(value, converter)


def _convert_capture_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [capture_wrapper_from_wire(item, converter) for item in value]


def _convert_event(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return event_wrapper_from_wire(value, converter)


def _convert_event_list(value: Any, converter: Any) -> Any:
    if value is None:
        return None
    return [event_wrapper_from_wire(item, converter) for item in value]


def _require(data: Mapping[str, Any], key: str, context: str) -> Any:
    if key not in data:
        raise ValueError(f"{context} payload is missing required field '{key}'")
    return data[key]


@dataclass
class EventWrapper(Generic[_T]):
    """Wrapper for captured events (timestamp + data + provenance)."""

    timestamp: int = 0
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None
    event_index: Optional[int] = None
    ix_path: Optional[str] = None


def event_wrapper_from_wire(value: Any, converter: Any = None) -> EventWrapper:
    """Converts a wire event wrapper into :class:`EventWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "EventWrapper")
    inner = data.get("data")
    return EventWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
        event_index=_to_int(data.get("event_index")),
        ix_path=data.get("ix_path"),
    )


@dataclass
class CaptureWrapper(Generic[_T]):
    """Wrapper for captured accounts (timestamp + address + data + provenance)."""

    timestamp: int = 0
    account_address: Optional[str] = None
    data: Optional[_T] = None
    slot: Optional[int] = None
    signature: Optional[str] = None


def capture_wrapper_from_wire(value: Any, converter: Any = None) -> CaptureWrapper:
    """Converts a wire account-capture wrapper into :class:`CaptureWrapper`.

    ``converter`` parses the inner ``data`` payload; omit it to pass the raw
    payload through.
    """
    data = _mapping(value, "CaptureWrapper")
    inner = data.get("data")
    return CaptureWrapper(
        timestamp=_to_int(data.get("timestamp")) or 0,
        account_address=data.get("account_address"),
        data=_convert(inner, converter) if converter is not None else inner,
        slot=_to_int(data.get("slot")),
        signature=data.get("signature"),
    )


@dataclass
class OreRoundId:
    """`id` section of `OreRound`."""

    round_id: Optional[int] = None
    round_address: Optional[str] = None


def ore_round_id_from_wire(value: Any) -> OreRoundId:
    """Converts a wire payload into :class:`OreRoundId`."""
    data = _mapping(value, "OreRoundId")
    return OreRoundId(
        round_id=_to_int(data.get("round_id")),
        round_address=data.get("round_address"),
    )


def ore_round_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundId` patch; only present keys appear."""
    data = _mapping(value, "OreRoundId patch")
    out: Dict[str, Any] = {}
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "round_address" in data:
        out["round_address"] = data["round_address"]
    return out


@dataclass
class OreRoundState:
    """`state` section of `OreRound`."""

    closes_at: Optional[int] = None
    expires_at: Optional[int] = None
    estimated_expires_at_unix: Optional[int] = None
    motherlode: Optional[float] = None
    total_deployed: Optional[float] = None
    total_vaulted: Optional[float] = None
    total_winnings: Optional[float] = None
    total_miners: Optional[int] = None
    deployed_per_square: Optional[List[int]] = None
    deployed_per_square_ui: Optional[List[float]] = None
    count_per_square: Optional[List[int]] = None


def ore_round_state_from_wire(value: Any) -> OreRoundState:
    """Converts a wire payload into :class:`OreRoundState`."""
    data = _mapping(value, "OreRoundState")
    return OreRoundState(
        closes_at=_to_int(data.get("closes_at")),
        expires_at=_to_int(data.get("expires_at")),
        estimated_expires_at_unix=_to_int(data.get("estimated_expires_at_unix")),
        motherlode=data.get("motherlode"),
        total_deployed=data.get("total_deployed"),
        total_vaulted=data.get("total_vaulted"),
        total_winnings=data.get("total_winnings"),
        total_miners=_to_int(data.get("total_miners")),
        deployed_per_square=_to_int_list(data.get("deployed_per_square")),
        deployed_per_square_ui=data.get("deployed_per_square_ui"),
        count_per_square=_to_int_list(data.get("count_per_square")),
    )


def ore_round_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundState` patch; only present keys appear."""
    data = _mapping(value, "OreRoundState patch")
    out: Dict[str, Any] = {}
    if "closes_at" in data:
        out["closes_at"] = _to_int(data["closes_at"])
    if "expires_at" in data:
        out["expires_at"] = _to_int(data["expires_at"])
    if "estimated_expires_at_unix" in data:
        out["estimated_expires_at_unix"] = _to_int(data["estimated_expires_at_unix"])
    if "motherlode" in data:
        out["motherlode"] = data["motherlode"]
    if "total_deployed" in data:
        out["total_deployed"] = data["total_deployed"]
    if "total_vaulted" in data:
        out["total_vaulted"] = data["total_vaulted"]
    if "total_winnings" in data:
        out["total_winnings"] = data["total_winnings"]
    if "total_miners" in data:
        out["total_miners"] = _to_int(data["total_miners"])
    if "deployed_per_square" in data:
        out["deployed_per_square"] = _to_int_list(data["deployed_per_square"])
    if "deployed_per_square_ui" in data:
        out["deployed_per_square_ui"] = data["deployed_per_square_ui"]
    if "count_per_square" in data:
        out["count_per_square"] = _to_int_list(data["count_per_square"])
    return out


@dataclass
class OreRoundResults:
    """`results` section of `OreRound`."""

    top_miner: Optional[str] = None
    top_miner_reward: Optional[float] = None
    rent_payer: Optional[str] = None
    slot_hash: Optional[str] = None
    expires_at_slot_hash: Any = None
    rng: Optional[int] = None
    winning_square: Optional[int] = None
    did_hit_motherlode: Optional[bool] = None
    pre_reveal_rng_candidate: Optional[int] = None
    pre_reveal_rng: Optional[int] = None
    pre_reveal_winning_square: Optional[int] = None


def ore_round_results_from_wire(value: Any) -> OreRoundResults:
    """Converts a wire payload into :class:`OreRoundResults`."""
    data = _mapping(value, "OreRoundResults")
    return OreRoundResults(
        top_miner=data.get("top_miner"),
        top_miner_reward=data.get("top_miner_reward"),
        rent_payer=data.get("rent_payer"),
        slot_hash=data.get("slot_hash"),
        expires_at_slot_hash=data.get("expires_at_slot_hash"),
        rng=_to_int(data.get("rng")),
        winning_square=_to_int(data.get("winning_square")),
        did_hit_motherlode=data.get("did_hit_motherlode"),
        pre_reveal_rng_candidate=_to_int(data.get("pre_reveal_rng_candidate")),
        pre_reveal_rng=_to_int(data.get("pre_reveal_rng")),
        pre_reveal_winning_square=_to_int(data.get("pre_reveal_winning_square")),
    )


def ore_round_results_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundResults` patch; only present keys appear."""
    data = _mapping(value, "OreRoundResults patch")
    out: Dict[str, Any] = {}
    if "top_miner" in data:
        out["top_miner"] = data["top_miner"]
    if "top_miner_reward" in data:
        out["top_miner_reward"] = data["top_miner_reward"]
    if "rent_payer" in data:
        out["rent_payer"] = data["rent_payer"]
    if "slot_hash" in data:
        out["slot_hash"] = data["slot_hash"]
    if "expires_at_slot_hash" in data:
        out["expires_at_slot_hash"] = data["expires_at_slot_hash"]
    if "rng" in data:
        out["rng"] = _to_int(data["rng"])
    if "winning_square" in data:
        out["winning_square"] = _to_int(data["winning_square"])
    if "did_hit_motherlode" in data:
        out["did_hit_motherlode"] = data["did_hit_motherlode"]
    if "pre_reveal_rng_candidate" in data:
        out["pre_reveal_rng_candidate"] = _to_int(data["pre_reveal_rng_candidate"])
    if "pre_reveal_rng" in data:
        out["pre_reveal_rng"] = _to_int(data["pre_reveal_rng"])
    if "pre_reveal_winning_square" in data:
        out["pre_reveal_winning_square"] = _to_int(data["pre_reveal_winning_square"])
    return out


@dataclass
class OreRoundMetrics:
    """`metrics` section of `OreRound`."""

    deploy_count: Optional[int] = None
    checkpoint_count: Optional[int] = None


def ore_round_metrics_from_wire(value: Any) -> OreRoundMetrics:
    """Converts a wire payload into :class:`OreRoundMetrics`."""
    data = _mapping(value, "OreRoundMetrics")
    return OreRoundMetrics(
        deploy_count=_to_int(data.get("deploy_count")),
        checkpoint_count=_to_int(data.get("checkpoint_count")),
    )


def ore_round_metrics_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundMetrics` patch; only present keys appear."""
    data = _mapping(value, "OreRoundMetrics patch")
    out: Dict[str, Any] = {}
    if "deploy_count" in data:
        out["deploy_count"] = _to_int(data["deploy_count"])
    if "checkpoint_count" in data:
        out["checkpoint_count"] = _to_int(data["checkpoint_count"])
    return out


@dataclass
class OreRoundTreasury:
    """`treasury` section of `OreRound`."""

    motherlode: Optional[float] = None


def ore_round_treasury_from_wire(value: Any) -> OreRoundTreasury:
    """Converts a wire payload into :class:`OreRoundTreasury`."""
    data = _mapping(value, "OreRoundTreasury")
    return OreRoundTreasury(
        motherlode=data.get("motherlode"),
    )


def ore_round_treasury_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundTreasury` patch; only present keys appear."""
    data = _mapping(value, "OreRoundTreasury patch")
    out: Dict[str, Any] = {}
    if "motherlode" in data:
        out["motherlode"] = data["motherlode"]
    return out


@dataclass
class OreRoundEntropy:
    """`entropy` section of `OreRound`."""

    entropy_value: Optional[str] = None
    entropy_seed: Optional[str] = None
    entropy_slot_hash: Optional[str] = None
    entropy_start_at: Optional[int] = None
    entropy_end_at: Optional[int] = None
    entropy_samples: Optional[int] = None
    entropy_var_address: Optional[str] = None
    resolved_seed: Optional[List[int]] = None


def ore_round_entropy_from_wire(value: Any) -> OreRoundEntropy:
    """Converts a wire payload into :class:`OreRoundEntropy`."""
    data = _mapping(value, "OreRoundEntropy")
    return OreRoundEntropy(
        entropy_value=data.get("entropy_value"),
        entropy_seed=data.get("entropy_seed"),
        entropy_slot_hash=data.get("entropy_slot_hash"),
        entropy_start_at=_to_int(data.get("entropy_start_at")),
        entropy_end_at=_to_int(data.get("entropy_end_at")),
        entropy_samples=_to_int(data.get("entropy_samples")),
        entropy_var_address=data.get("entropy_var_address"),
        resolved_seed=_to_int_list(data.get("resolved_seed")),
    )


def ore_round_entropy_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRoundEntropy` patch; only present keys appear."""
    data = _mapping(value, "OreRoundEntropy patch")
    out: Dict[str, Any] = {}
    if "entropy_value" in data:
        out["entropy_value"] = data["entropy_value"]
    if "entropy_seed" in data:
        out["entropy_seed"] = data["entropy_seed"]
    if "entropy_slot_hash" in data:
        out["entropy_slot_hash"] = data["entropy_slot_hash"]
    if "entropy_start_at" in data:
        out["entropy_start_at"] = _to_int(data["entropy_start_at"])
    if "entropy_end_at" in data:
        out["entropy_end_at"] = _to_int(data["entropy_end_at"])
    if "entropy_samples" in data:
        out["entropy_samples"] = _to_int(data["entropy_samples"])
    if "entropy_var_address" in data:
        out["entropy_var_address"] = data["entropy_var_address"]
    if "resolved_seed" in data:
        out["resolved_seed"] = _to_int_list(data["resolved_seed"])
    return out


@dataclass
class OreRound:
    """Entity `OreRound`."""

    id: OreRoundId = field(default_factory=OreRoundId)
    state: OreRoundState = field(default_factory=OreRoundState)
    results: OreRoundResults = field(default_factory=OreRoundResults)
    metrics: OreRoundMetrics = field(default_factory=OreRoundMetrics)
    treasury: OreRoundTreasury = field(default_factory=OreRoundTreasury)
    entropy: OreRoundEntropy = field(default_factory=OreRoundEntropy)
    ore_metadata: Any = None


def ore_round_from_wire(value: Any) -> OreRound:
    """Converts a merged wire entity into :class:`OreRound`."""
    data = _mapping(value, "OreRound")
    return OreRound(
        id=ore_round_id_from_wire(data.get("id") or {}),
        state=ore_round_state_from_wire(data.get("state") or {}),
        results=ore_round_results_from_wire(data.get("results") or {}),
        metrics=ore_round_metrics_from_wire(data.get("metrics") or {}),
        treasury=ore_round_treasury_from_wire(data.get("treasury") or {}),
        entropy=ore_round_entropy_from_wire(data.get("entropy") or {}),
        ore_metadata=data.get("ore_metadata"),
    )


def ore_round_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreRound` patch; only present keys appear."""
    data = _mapping(value, "OreRound patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = ore_round_id_patch_from_wire(data["id"] or {})
    if "state" in data:
        out["state"] = ore_round_state_patch_from_wire(data["state"] or {})
    if "results" in data:
        out["results"] = ore_round_results_patch_from_wire(data["results"] or {})
    if "metrics" in data:
        out["metrics"] = ore_round_metrics_patch_from_wire(data["metrics"] or {})
    if "treasury" in data:
        out["treasury"] = ore_round_treasury_patch_from_wire(data["treasury"] or {})
    if "entropy" in data:
        out["entropy"] = ore_round_entropy_patch_from_wire(data["entropy"] or {})
    if "ore_metadata" in data:
        out["ore_metadata"] = data["ore_metadata"]
    return out


@dataclass
class Board:
    """Resolved type `Board`."""

    round_id: Optional[int] = None
    start_slot: Optional[int] = None
    end_slot: Optional[int] = None
    production_cost_ema: Optional[int] = None


def board_from_wire(value: Any) -> Board:
    """Converts a wire payload into :class:`Board`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Board")
    return Board(
        round_id=_to_int(_require(data, "round_id", "Board")),
        start_slot=_to_int(_require(data, "start_slot", "Board")),
        end_slot=_to_int(_require(data, "end_slot", "Board")),
        production_cost_ema=_to_int(_require(data, "production_cost_ema", "Board")),
    )


def board_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Board` patch; only present keys appear."""
    data = _mapping(value, "Board patch")
    out: Dict[str, Any] = {}
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "start_slot" in data:
        out["start_slot"] = _to_int(data["start_slot"])
    if "end_slot" in data:
        out["end_slot"] = _to_int(data["end_slot"])
    if "production_cost_ema" in data:
        out["production_cost_ema"] = _to_int(data["production_cost_ema"])
    return out


@dataclass
class OreBoardId:
    """`id` section of `OreBoard`."""

    address: Optional[str] = None


def ore_board_id_from_wire(value: Any) -> OreBoardId:
    """Converts a wire payload into :class:`OreBoardId`."""
    data = _mapping(value, "OreBoardId")
    return OreBoardId(
        address=data.get("address"),
    )


def ore_board_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreBoardId` patch; only present keys appear."""
    data = _mapping(value, "OreBoardId patch")
    out: Dict[str, Any] = {}
    if "address" in data:
        out["address"] = data["address"]
    return out


@dataclass
class OreBoardState:
    """`state` section of `OreBoard`."""

    round_id: Optional[int] = None
    start_slot: Optional[int] = None
    end_slot: Optional[int] = None
    production_cost_ema: Optional[int] = None


def ore_board_state_from_wire(value: Any) -> OreBoardState:
    """Converts a wire payload into :class:`OreBoardState`."""
    data = _mapping(value, "OreBoardState")
    return OreBoardState(
        round_id=_to_int(data.get("round_id")),
        start_slot=_to_int(data.get("start_slot")),
        end_slot=_to_int(data.get("end_slot")),
        production_cost_ema=_to_int(data.get("production_cost_ema")),
    )


def ore_board_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreBoardState` patch; only present keys appear."""
    data = _mapping(value, "OreBoardState patch")
    out: Dict[str, Any] = {}
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "start_slot" in data:
        out["start_slot"] = _to_int(data["start_slot"])
    if "end_slot" in data:
        out["end_slot"] = _to_int(data["end_slot"])
    if "production_cost_ema" in data:
        out["production_cost_ema"] = _to_int(data["production_cost_ema"])
    return out


@dataclass
class OreBoard:
    """Entity `OreBoard`."""

    id: OreBoardId = field(default_factory=OreBoardId)
    state: OreBoardState = field(default_factory=OreBoardState)
    board_snapshot: Optional[CaptureWrapper[Board]] = None


def ore_board_from_wire(value: Any) -> OreBoard:
    """Converts a merged wire entity into :class:`OreBoard`."""
    data = _mapping(value, "OreBoard")
    return OreBoard(
        id=ore_board_id_from_wire(data.get("id") or {}),
        state=ore_board_state_from_wire(data.get("state") or {}),
        board_snapshot=_convert_capture(data.get("board_snapshot"), board_from_wire),
    )


def ore_board_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreBoard` patch; only present keys appear."""
    data = _mapping(value, "OreBoard patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = ore_board_id_patch_from_wire(data["id"] or {})
    if "state" in data:
        out["state"] = ore_board_state_patch_from_wire(data["state"] or {})
    if "board_snapshot" in data:
        out["board_snapshot"] = _convert_capture(data["board_snapshot"], board_from_wire)
    return out


@dataclass
class Treasury:
    """Resolved type `Treasury`."""

    motherlode: Optional[int] = None
    miner_rewards_factor: Any = None
    total_refined: Optional[int] = None
    total_unclaimed: Optional[int] = None


def treasury_from_wire(value: Any) -> Treasury:
    """Converts a wire payload into :class:`Treasury`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Treasury")
    return Treasury(
        motherlode=_to_int(_require(data, "motherlode", "Treasury")),
        miner_rewards_factor=_require(data, "miner_rewards_factor", "Treasury"),
        total_refined=_to_int(_require(data, "total_refined", "Treasury")),
        total_unclaimed=_to_int(_require(data, "total_unclaimed", "Treasury")),
    )


def treasury_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Treasury` patch; only present keys appear."""
    data = _mapping(value, "Treasury patch")
    out: Dict[str, Any] = {}
    if "motherlode" in data:
        out["motherlode"] = _to_int(data["motherlode"])
    if "miner_rewards_factor" in data:
        out["miner_rewards_factor"] = data["miner_rewards_factor"]
    if "total_refined" in data:
        out["total_refined"] = _to_int(data["total_refined"])
    if "total_unclaimed" in data:
        out["total_unclaimed"] = _to_int(data["total_unclaimed"])
    return out


@dataclass
class OreTreasuryId:
    """`id` section of `OreTreasury`."""

    address: Optional[str] = None


def ore_treasury_id_from_wire(value: Any) -> OreTreasuryId:
    """Converts a wire payload into :class:`OreTreasuryId`."""
    data = _mapping(value, "OreTreasuryId")
    return OreTreasuryId(
        address=data.get("address"),
    )


def ore_treasury_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreTreasuryId` patch; only present keys appear."""
    data = _mapping(value, "OreTreasuryId patch")
    out: Dict[str, Any] = {}
    if "address" in data:
        out["address"] = data["address"]
    return out


@dataclass
class OreTreasuryState:
    """`state` section of `OreTreasury`."""

    motherlode: Optional[float] = None
    total_refined: Optional[float] = None
    total_unclaimed: Optional[float] = None


def ore_treasury_state_from_wire(value: Any) -> OreTreasuryState:
    """Converts a wire payload into :class:`OreTreasuryState`."""
    data = _mapping(value, "OreTreasuryState")
    return OreTreasuryState(
        motherlode=data.get("motherlode"),
        total_refined=data.get("total_refined"),
        total_unclaimed=data.get("total_unclaimed"),
    )


def ore_treasury_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreTreasuryState` patch; only present keys appear."""
    data = _mapping(value, "OreTreasuryState patch")
    out: Dict[str, Any] = {}
    if "motherlode" in data:
        out["motherlode"] = data["motherlode"]
    if "total_refined" in data:
        out["total_refined"] = data["total_refined"]
    if "total_unclaimed" in data:
        out["total_unclaimed"] = data["total_unclaimed"]
    return out


@dataclass
class OreTreasury:
    """Entity `OreTreasury`."""

    id: OreTreasuryId = field(default_factory=OreTreasuryId)
    state: OreTreasuryState = field(default_factory=OreTreasuryState)
    treasury_snapshot: Optional[CaptureWrapper[Treasury]] = None


def ore_treasury_from_wire(value: Any) -> OreTreasury:
    """Converts a merged wire entity into :class:`OreTreasury`."""
    data = _mapping(value, "OreTreasury")
    return OreTreasury(
        id=ore_treasury_id_from_wire(data.get("id") or {}),
        state=ore_treasury_state_from_wire(data.get("state") or {}),
        treasury_snapshot=_convert_capture(data.get("treasury_snapshot"), treasury_from_wire),
    )


def ore_treasury_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreTreasury` patch; only present keys appear."""
    data = _mapping(value, "OreTreasury patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = ore_treasury_id_patch_from_wire(data["id"] or {})
    if "state" in data:
        out["state"] = ore_treasury_state_patch_from_wire(data["state"] or {})
    if "treasury_snapshot" in data:
        out["treasury_snapshot"] = _convert_capture(data["treasury_snapshot"], treasury_from_wire)
    return out


@dataclass
class Miner:
    """Resolved type `Miner`."""

    authority: Optional[str] = None
    auto_return: Optional[int] = None
    checkpoint_id: Optional[int] = None
    checkpoint_fee: Optional[int] = None
    deployed: Optional[List[int]] = None
    mass: Optional[List[int]] = None
    cumulative: Optional[List[int]] = None
    round_id: Optional[int] = None
    rewards_factor: Any = None
    rewards_sol: Optional[int] = None
    refined_ore: Optional[int] = None
    rewards_ore: Optional[int] = None
    last_claim_ore_at: Optional[int] = None
    last_claim_sol_at: Optional[int] = None
    lifetime_rewards_ore: Optional[int] = None
    lifetime_deployed: Optional[int] = None
    lifetime_rewards_sol: Optional[int] = None


def miner_from_wire(value: Any) -> Miner:
    """Converts a wire payload into :class:`Miner`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Miner")
    return Miner(
        authority=_require(data, "authority", "Miner"),
        auto_return=_to_int(_require(data, "auto_return", "Miner")),
        checkpoint_id=_to_int(_require(data, "checkpoint_id", "Miner")),
        checkpoint_fee=_to_int(_require(data, "checkpoint_fee", "Miner")),
        deployed=_to_int_list(_require(data, "deployed", "Miner")),
        mass=_to_int_list(_require(data, "mass", "Miner")),
        cumulative=_to_int_list(_require(data, "cumulative", "Miner")),
        round_id=_to_int(_require(data, "round_id", "Miner")),
        rewards_factor=_require(data, "rewards_factor", "Miner"),
        rewards_sol=_to_int(_require(data, "rewards_sol", "Miner")),
        refined_ore=_to_int(_require(data, "refined_ore", "Miner")),
        rewards_ore=_to_int(_require(data, "rewards_ore", "Miner")),
        last_claim_ore_at=_to_int(_require(data, "last_claim_ore_at", "Miner")),
        last_claim_sol_at=_to_int(_require(data, "last_claim_sol_at", "Miner")),
        lifetime_rewards_ore=_to_int(_require(data, "lifetime_rewards_ore", "Miner")),
        lifetime_deployed=_to_int(_require(data, "lifetime_deployed", "Miner")),
        lifetime_rewards_sol=_to_int(_require(data, "lifetime_rewards_sol", "Miner")),
    )


def miner_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Miner` patch; only present keys appear."""
    data = _mapping(value, "Miner patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "auto_return" in data:
        out["auto_return"] = _to_int(data["auto_return"])
    if "checkpoint_id" in data:
        out["checkpoint_id"] = _to_int(data["checkpoint_id"])
    if "checkpoint_fee" in data:
        out["checkpoint_fee"] = _to_int(data["checkpoint_fee"])
    if "deployed" in data:
        out["deployed"] = _to_int_list(data["deployed"])
    if "mass" in data:
        out["mass"] = _to_int_list(data["mass"])
    if "cumulative" in data:
        out["cumulative"] = _to_int_list(data["cumulative"])
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "rewards_factor" in data:
        out["rewards_factor"] = data["rewards_factor"]
    if "rewards_sol" in data:
        out["rewards_sol"] = _to_int(data["rewards_sol"])
    if "refined_ore" in data:
        out["refined_ore"] = _to_int(data["refined_ore"])
    if "rewards_ore" in data:
        out["rewards_ore"] = _to_int(data["rewards_ore"])
    if "last_claim_ore_at" in data:
        out["last_claim_ore_at"] = _to_int(data["last_claim_ore_at"])
    if "last_claim_sol_at" in data:
        out["last_claim_sol_at"] = _to_int(data["last_claim_sol_at"])
    if "lifetime_rewards_ore" in data:
        out["lifetime_rewards_ore"] = _to_int(data["lifetime_rewards_ore"])
    if "lifetime_deployed" in data:
        out["lifetime_deployed"] = _to_int(data["lifetime_deployed"])
    if "lifetime_rewards_sol" in data:
        out["lifetime_rewards_sol"] = _to_int(data["lifetime_rewards_sol"])
    return out


@dataclass
class Automation:
    """Resolved type `Automation`."""

    amount: Optional[int] = None
    authority: Optional[str] = None
    balance: Optional[int] = None
    executor: Optional[str] = None
    fee: Optional[int] = None
    strategy: Optional[int] = None
    mask: Optional[int] = None
    reload: Optional[int] = None
    total_sol_spent: Optional[int] = None
    total_ore_earned: Optional[int] = None
    conditions: Any = None


def automation_from_wire(value: Any) -> Automation:
    """Converts a wire payload into :class:`Automation`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Automation")
    return Automation(
        amount=_to_int(_require(data, "amount", "Automation")),
        authority=_require(data, "authority", "Automation"),
        balance=_to_int(_require(data, "balance", "Automation")),
        executor=_require(data, "executor", "Automation"),
        fee=_to_int(_require(data, "fee", "Automation")),
        strategy=_to_int(_require(data, "strategy", "Automation")),
        mask=_to_int(_require(data, "mask", "Automation")),
        reload=_to_int(_require(data, "reload", "Automation")),
        total_sol_spent=_to_int(_require(data, "total_sol_spent", "Automation")),
        total_ore_earned=_to_int(_require(data, "total_ore_earned", "Automation")),
        conditions=_require(data, "conditions", "Automation"),
    )


def automation_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Automation` patch; only present keys appear."""
    data = _mapping(value, "Automation patch")
    out: Dict[str, Any] = {}
    if "amount" in data:
        out["amount"] = _to_int(data["amount"])
    if "authority" in data:
        out["authority"] = data["authority"]
    if "balance" in data:
        out["balance"] = _to_int(data["balance"])
    if "executor" in data:
        out["executor"] = data["executor"]
    if "fee" in data:
        out["fee"] = _to_int(data["fee"])
    if "strategy" in data:
        out["strategy"] = _to_int(data["strategy"])
    if "mask" in data:
        out["mask"] = _to_int(data["mask"])
    if "reload" in data:
        out["reload"] = _to_int(data["reload"])
    if "total_sol_spent" in data:
        out["total_sol_spent"] = _to_int(data["total_sol_spent"])
    if "total_ore_earned" in data:
        out["total_ore_earned"] = _to_int(data["total_ore_earned"])
    if "conditions" in data:
        out["conditions"] = data["conditions"]
    return out


@dataclass
class OreMinerId:
    """`id` section of `OreMiner`."""

    authority: Optional[str] = None
    miner_address: Optional[str] = None
    automation_address: Optional[str] = None


def ore_miner_id_from_wire(value: Any) -> OreMinerId:
    """Converts a wire payload into :class:`OreMinerId`."""
    data = _mapping(value, "OreMinerId")
    return OreMinerId(
        authority=data.get("authority"),
        miner_address=data.get("miner_address"),
        automation_address=data.get("automation_address"),
    )


def ore_miner_id_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMinerId` patch; only present keys appear."""
    data = _mapping(value, "OreMinerId patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "miner_address" in data:
        out["miner_address"] = data["miner_address"]
    if "automation_address" in data:
        out["automation_address"] = data["automation_address"]
    return out


@dataclass
class OreMinerRewards:
    """`rewards` section of `OreMiner`."""

    rewards_sol: Optional[int] = None
    rewards_ore: Optional[int] = None
    refined_ore: Optional[int] = None
    lifetime_rewards_sol: Optional[int] = None
    lifetime_rewards_ore: Optional[int] = None
    lifetime_deployed: Optional[int] = None


def ore_miner_rewards_from_wire(value: Any) -> OreMinerRewards:
    """Converts a wire payload into :class:`OreMinerRewards`."""
    data = _mapping(value, "OreMinerRewards")
    return OreMinerRewards(
        rewards_sol=_to_int(data.get("rewards_sol")),
        rewards_ore=_to_int(data.get("rewards_ore")),
        refined_ore=_to_int(data.get("refined_ore")),
        lifetime_rewards_sol=_to_int(data.get("lifetime_rewards_sol")),
        lifetime_rewards_ore=_to_int(data.get("lifetime_rewards_ore")),
        lifetime_deployed=_to_int(data.get("lifetime_deployed")),
    )


def ore_miner_rewards_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMinerRewards` patch; only present keys appear."""
    data = _mapping(value, "OreMinerRewards patch")
    out: Dict[str, Any] = {}
    if "rewards_sol" in data:
        out["rewards_sol"] = _to_int(data["rewards_sol"])
    if "rewards_ore" in data:
        out["rewards_ore"] = _to_int(data["rewards_ore"])
    if "refined_ore" in data:
        out["refined_ore"] = _to_int(data["refined_ore"])
    if "lifetime_rewards_sol" in data:
        out["lifetime_rewards_sol"] = _to_int(data["lifetime_rewards_sol"])
    if "lifetime_rewards_ore" in data:
        out["lifetime_rewards_ore"] = _to_int(data["lifetime_rewards_ore"])
    if "lifetime_deployed" in data:
        out["lifetime_deployed"] = _to_int(data["lifetime_deployed"])
    return out


@dataclass
class OreMinerState:
    """`state` section of `OreMiner`."""

    round_id: Optional[int] = None
    deployed_per_square: Optional[List[int]] = None
    deployed_per_square_ui: Optional[List[float]] = None
    total_deployed: Optional[float] = None
    checkpoint_id: Optional[int] = None
    checkpoint_fee: Optional[int] = None
    last_claim_ore_at: Optional[int] = None
    last_claim_sol_at: Optional[int] = None


def ore_miner_state_from_wire(value: Any) -> OreMinerState:
    """Converts a wire payload into :class:`OreMinerState`."""
    data = _mapping(value, "OreMinerState")
    return OreMinerState(
        round_id=_to_int(data.get("round_id")),
        deployed_per_square=_to_int_list(data.get("deployed_per_square")),
        deployed_per_square_ui=data.get("deployed_per_square_ui"),
        total_deployed=data.get("total_deployed"),
        checkpoint_id=_to_int(data.get("checkpoint_id")),
        checkpoint_fee=_to_int(data.get("checkpoint_fee")),
        last_claim_ore_at=_to_int(data.get("last_claim_ore_at")),
        last_claim_sol_at=_to_int(data.get("last_claim_sol_at")),
    )


def ore_miner_state_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMinerState` patch; only present keys appear."""
    data = _mapping(value, "OreMinerState patch")
    out: Dict[str, Any] = {}
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "deployed_per_square" in data:
        out["deployed_per_square"] = _to_int_list(data["deployed_per_square"])
    if "deployed_per_square_ui" in data:
        out["deployed_per_square_ui"] = data["deployed_per_square_ui"]
    if "total_deployed" in data:
        out["total_deployed"] = data["total_deployed"]
    if "checkpoint_id" in data:
        out["checkpoint_id"] = _to_int(data["checkpoint_id"])
    if "checkpoint_fee" in data:
        out["checkpoint_fee"] = _to_int(data["checkpoint_fee"])
    if "last_claim_ore_at" in data:
        out["last_claim_ore_at"] = _to_int(data["last_claim_ore_at"])
    if "last_claim_sol_at" in data:
        out["last_claim_sol_at"] = _to_int(data["last_claim_sol_at"])
    return out


@dataclass
class OreMinerAutomation:
    """`automation` section of `OreMiner`."""

    amount: Optional[int] = None
    balance: Optional[int] = None
    executor: Optional[str] = None
    fee: Optional[int] = None
    strategy: Optional[int] = None
    mask: Optional[int] = None
    reload: Optional[int] = None


def ore_miner_automation_from_wire(value: Any) -> OreMinerAutomation:
    """Converts a wire payload into :class:`OreMinerAutomation`."""
    data = _mapping(value, "OreMinerAutomation")
    return OreMinerAutomation(
        amount=_to_int(data.get("amount")),
        balance=_to_int(data.get("balance")),
        executor=data.get("executor"),
        fee=_to_int(data.get("fee")),
        strategy=_to_int(data.get("strategy")),
        mask=_to_int(data.get("mask")),
        reload=_to_int(data.get("reload")),
    )


def ore_miner_automation_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMinerAutomation` patch; only present keys appear."""
    data = _mapping(value, "OreMinerAutomation patch")
    out: Dict[str, Any] = {}
    if "amount" in data:
        out["amount"] = _to_int(data["amount"])
    if "balance" in data:
        out["balance"] = _to_int(data["balance"])
    if "executor" in data:
        out["executor"] = data["executor"]
    if "fee" in data:
        out["fee"] = _to_int(data["fee"])
    if "strategy" in data:
        out["strategy"] = _to_int(data["strategy"])
    if "mask" in data:
        out["mask"] = _to_int(data["mask"])
    if "reload" in data:
        out["reload"] = _to_int(data["reload"])
    return out


@dataclass
class OreMiner:
    """Entity `OreMiner`."""

    id: OreMinerId = field(default_factory=OreMinerId)
    rewards: OreMinerRewards = field(default_factory=OreMinerRewards)
    state: OreMinerState = field(default_factory=OreMinerState)
    automation: OreMinerAutomation = field(default_factory=OreMinerAutomation)
    miner_snapshot: Optional[CaptureWrapper[Miner]] = None
    automation_snapshot: Optional[CaptureWrapper[Automation]] = None


def ore_miner_from_wire(value: Any) -> OreMiner:
    """Converts a merged wire entity into :class:`OreMiner`."""
    data = _mapping(value, "OreMiner")
    return OreMiner(
        id=ore_miner_id_from_wire(data.get("id") or {}),
        rewards=ore_miner_rewards_from_wire(data.get("rewards") or {}),
        state=ore_miner_state_from_wire(data.get("state") or {}),
        automation=ore_miner_automation_from_wire(data.get("automation") or {}),
        miner_snapshot=_convert_capture(data.get("miner_snapshot"), miner_from_wire),
        automation_snapshot=_convert_capture(data.get("automation_snapshot"), automation_from_wire),
    )


def ore_miner_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMiner` patch; only present keys appear."""
    data = _mapping(value, "OreMiner patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = ore_miner_id_patch_from_wire(data["id"] or {})
    if "rewards" in data:
        out["rewards"] = ore_miner_rewards_patch_from_wire(data["rewards"] or {})
    if "state" in data:
        out["state"] = ore_miner_state_patch_from_wire(data["state"] or {})
    if "automation" in data:
        out["automation"] = ore_miner_automation_patch_from_wire(data["automation"] or {})
    if "miner_snapshot" in data:
        out["miner_snapshot"] = _convert_capture(data["miner_snapshot"], miner_from_wire)
    if "automation_snapshot" in data:
        out["automation_snapshot"] = _convert_capture(data["automation_snapshot"], automation_from_wire)
    return out


@dataclass
class AutomationConditions:
    """IDL type `AutomationConditions` as program reads decode it."""

    max_production_cost: Optional[int] = None
    min_motherlode: Optional[int] = None
    max_motherlode: Optional[int] = None


def automation_conditions_from_wire(value: Any) -> AutomationConditions:
    """Converts a wire payload into :class:`AutomationConditions`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "AutomationConditions")
    return AutomationConditions(
        max_production_cost=_to_int(_require(data, "max_production_cost", "AutomationConditions")),
        min_motherlode=_to_int(_require(data, "min_motherlode", "AutomationConditions")),
        max_motherlode=_to_int(_require(data, "max_motherlode", "AutomationConditions")),
    )


def automation_conditions_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `AutomationConditions` patch; only present keys appear."""
    data = _mapping(value, "AutomationConditions patch")
    out: Dict[str, Any] = {}
    if "max_production_cost" in data:
        out["max_production_cost"] = _to_int(data["max_production_cost"])
    if "min_motherlode" in data:
        out["min_motherlode"] = _to_int(data["min_motherlode"])
    if "max_motherlode" in data:
        out["max_motherlode"] = _to_int(data["max_motherlode"])
    return out


@dataclass
class OreAutomation:
    """Account `Automation` as program reads decode it."""

    amount: Optional[int] = None
    authority: Optional[str] = None
    balance: Optional[int] = None
    executor: Optional[str] = None
    fee: Optional[int] = None
    strategy: Optional[int] = None
    mask: Optional[int] = None
    reload: Optional[int] = None
    total_sol_spent: Optional[int] = None
    total_ore_earned: Optional[int] = None
    conditions: Optional[AutomationConditions] = None


def ore_automation_from_wire(value: Any) -> OreAutomation:
    """Converts a wire payload into :class:`OreAutomation`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "OreAutomation")
    return OreAutomation(
        amount=_to_int(_require(data, "amount", "OreAutomation")),
        authority=_require(data, "authority", "OreAutomation"),
        balance=_to_int(_require(data, "balance", "OreAutomation")),
        executor=_require(data, "executor", "OreAutomation"),
        fee=_to_int(_require(data, "fee", "OreAutomation")),
        strategy=_to_int(_require(data, "strategy", "OreAutomation")),
        mask=_to_int(_require(data, "mask", "OreAutomation")),
        reload=_to_int(_require(data, "reload", "OreAutomation")),
        total_sol_spent=_to_int(_require(data, "total_sol_spent", "OreAutomation")),
        total_ore_earned=_to_int(_require(data, "total_ore_earned", "OreAutomation")),
        conditions=_convert(_require(data, "conditions", "OreAutomation"), automation_conditions_from_wire),
    )


def ore_automation_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreAutomation` patch; only present keys appear."""
    data = _mapping(value, "OreAutomation patch")
    out: Dict[str, Any] = {}
    if "amount" in data:
        out["amount"] = _to_int(data["amount"])
    if "authority" in data:
        out["authority"] = data["authority"]
    if "balance" in data:
        out["balance"] = _to_int(data["balance"])
    if "executor" in data:
        out["executor"] = data["executor"]
    if "fee" in data:
        out["fee"] = _to_int(data["fee"])
    if "strategy" in data:
        out["strategy"] = _to_int(data["strategy"])
    if "mask" in data:
        out["mask"] = _to_int(data["mask"])
    if "reload" in data:
        out["reload"] = _to_int(data["reload"])
    if "total_sol_spent" in data:
        out["total_sol_spent"] = _to_int(data["total_sol_spent"])
    if "total_ore_earned" in data:
        out["total_ore_earned"] = _to_int(data["total_ore_earned"])
    if "conditions" in data:
        out["conditions"] = _convert(data["conditions"], automation_conditions_from_wire)
    return out


@dataclass
class AdminConfig:
    """IDL type `AdminConfig` as program reads decode it."""

    authority: Optional[str] = None
    fee_collector: Optional[str] = None
    fee_rate: Optional[int] = None


def admin_config_from_wire(value: Any) -> AdminConfig:
    """Converts a wire payload into :class:`AdminConfig`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "AdminConfig")
    return AdminConfig(
        authority=_require(data, "authority", "AdminConfig"),
        fee_collector=_require(data, "fee_collector", "AdminConfig"),
        fee_rate=_to_int(_require(data, "fee_rate", "AdminConfig")),
    )


def admin_config_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `AdminConfig` patch; only present keys appear."""
    data = _mapping(value, "AdminConfig patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "fee_collector" in data:
        out["fee_collector"] = data["fee_collector"]
    if "fee_rate" in data:
        out["fee_rate"] = _to_int(data["fee_rate"])
    return out


@dataclass
class ProtocolConfig:
    """IDL type `ProtocolConfig` as program reads decode it."""

    authority: Optional[str] = None
    fee_collector: Optional[str] = None
    fee_rate: Optional[int] = None
    intermission_slots: Optional[int] = None
    round_slots: Optional[int] = None
    entropy_var_address: Optional[str] = None
    entropy_program_id: Optional[str] = None


def protocol_config_from_wire(value: Any) -> ProtocolConfig:
    """Converts a wire payload into :class:`ProtocolConfig`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "ProtocolConfig")
    return ProtocolConfig(
        authority=_require(data, "authority", "ProtocolConfig"),
        fee_collector=_require(data, "fee_collector", "ProtocolConfig"),
        fee_rate=_to_int(_require(data, "fee_rate", "ProtocolConfig")),
        intermission_slots=_to_int(_require(data, "intermission_slots", "ProtocolConfig")),
        round_slots=_to_int(_require(data, "round_slots", "ProtocolConfig")),
        entropy_var_address=_require(data, "entropy_var_address", "ProtocolConfig"),
        entropy_program_id=_require(data, "entropy_program_id", "ProtocolConfig"),
    )


def protocol_config_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `ProtocolConfig` patch; only present keys appear."""
    data = _mapping(value, "ProtocolConfig patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "fee_collector" in data:
        out["fee_collector"] = data["fee_collector"]
    if "fee_rate" in data:
        out["fee_rate"] = _to_int(data["fee_rate"])
    if "intermission_slots" in data:
        out["intermission_slots"] = _to_int(data["intermission_slots"])
    if "round_slots" in data:
        out["round_slots"] = _to_int(data["round_slots"])
    if "entropy_var_address" in data:
        out["entropy_var_address"] = data["entropy_var_address"]
    if "entropy_program_id" in data:
        out["entropy_program_id"] = data["entropy_program_id"]
    return out


@dataclass
class Config:
    """Account `Config` as program reads decode it."""

    admin: Optional[AdminConfig] = None
    protocol: Optional[ProtocolConfig] = None


def config_from_wire(value: Any) -> Config:
    """Converts a wire payload into :class:`Config`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Config")
    return Config(
        admin=_convert(_require(data, "admin", "Config"), admin_config_from_wire),
        protocol=_convert(_require(data, "protocol", "Config"), protocol_config_from_wire),
    )


def config_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Config` patch; only present keys appear."""
    data = _mapping(value, "Config patch")
    out: Dict[str, Any] = {}
    if "admin" in data:
        out["admin"] = _convert(data["admin"], admin_config_from_wire)
    if "protocol" in data:
        out["protocol"] = _convert(data["protocol"], protocol_config_from_wire)
    return out


@dataclass
class Numeric:
    """IDL type `Numeric` as program reads decode it."""

    bits: Optional[List[int]] = None


def numeric_from_wire(value: Any) -> Numeric:
    """Converts a wire payload into :class:`Numeric`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Numeric")
    return Numeric(
        bits=_to_int_list(_require(data, "bits", "Numeric")),
    )


def numeric_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Numeric` patch; only present keys appear."""
    data = _mapping(value, "Numeric patch")
    out: Dict[str, Any] = {}
    if "bits" in data:
        out["bits"] = _to_int_list(data["bits"])
    return out


@dataclass
class OreMinerAccount:
    """Account `Miner` as program reads decode it."""

    authority: Optional[str] = None
    auto_return: Optional[int] = None
    checkpoint_id: Optional[int] = None
    checkpoint_fee: Optional[int] = None
    deployed: Optional[List[int]] = None
    mass: Optional[List[int]] = None
    cumulative: Optional[List[int]] = None
    round_id: Optional[int] = None
    rewards_factor: Optional[Numeric] = None
    rewards_sol: Optional[int] = None
    refined_ore: Optional[int] = None
    rewards_ore: Optional[int] = None
    last_claim_ore_at: Optional[int] = None
    last_claim_sol_at: Optional[int] = None
    lifetime_rewards_ore: Optional[int] = None
    lifetime_deployed: Optional[int] = None
    lifetime_rewards_sol: Optional[int] = None


def ore_miner_account_from_wire(value: Any) -> OreMinerAccount:
    """Converts a wire payload into :class:`OreMinerAccount`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "OreMinerAccount")
    return OreMinerAccount(
        authority=_require(data, "authority", "OreMinerAccount"),
        auto_return=_to_int(_require(data, "auto_return", "OreMinerAccount")),
        checkpoint_id=_to_int(_require(data, "checkpoint_id", "OreMinerAccount")),
        checkpoint_fee=_to_int(_require(data, "checkpoint_fee", "OreMinerAccount")),
        deployed=_to_int_list(_require(data, "deployed", "OreMinerAccount")),
        mass=_to_int_list(_require(data, "mass", "OreMinerAccount")),
        cumulative=_to_int_list(_require(data, "cumulative", "OreMinerAccount")),
        round_id=_to_int(_require(data, "round_id", "OreMinerAccount")),
        rewards_factor=_convert(_require(data, "rewards_factor", "OreMinerAccount"), numeric_from_wire),
        rewards_sol=_to_int(_require(data, "rewards_sol", "OreMinerAccount")),
        refined_ore=_to_int(_require(data, "refined_ore", "OreMinerAccount")),
        rewards_ore=_to_int(_require(data, "rewards_ore", "OreMinerAccount")),
        last_claim_ore_at=_to_int(_require(data, "last_claim_ore_at", "OreMinerAccount")),
        last_claim_sol_at=_to_int(_require(data, "last_claim_sol_at", "OreMinerAccount")),
        lifetime_rewards_ore=_to_int(_require(data, "lifetime_rewards_ore", "OreMinerAccount")),
        lifetime_deployed=_to_int(_require(data, "lifetime_deployed", "OreMinerAccount")),
        lifetime_rewards_sol=_to_int(_require(data, "lifetime_rewards_sol", "OreMinerAccount")),
    )


def ore_miner_account_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreMinerAccount` patch; only present keys appear."""
    data = _mapping(value, "OreMinerAccount patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "auto_return" in data:
        out["auto_return"] = _to_int(data["auto_return"])
    if "checkpoint_id" in data:
        out["checkpoint_id"] = _to_int(data["checkpoint_id"])
    if "checkpoint_fee" in data:
        out["checkpoint_fee"] = _to_int(data["checkpoint_fee"])
    if "deployed" in data:
        out["deployed"] = _to_int_list(data["deployed"])
    if "mass" in data:
        out["mass"] = _to_int_list(data["mass"])
    if "cumulative" in data:
        out["cumulative"] = _to_int_list(data["cumulative"])
    if "round_id" in data:
        out["round_id"] = _to_int(data["round_id"])
    if "rewards_factor" in data:
        out["rewards_factor"] = _convert(data["rewards_factor"], numeric_from_wire)
    if "rewards_sol" in data:
        out["rewards_sol"] = _to_int(data["rewards_sol"])
    if "refined_ore" in data:
        out["refined_ore"] = _to_int(data["refined_ore"])
    if "rewards_ore" in data:
        out["rewards_ore"] = _to_int(data["rewards_ore"])
    if "last_claim_ore_at" in data:
        out["last_claim_ore_at"] = _to_int(data["last_claim_ore_at"])
    if "last_claim_sol_at" in data:
        out["last_claim_sol_at"] = _to_int(data["last_claim_sol_at"])
    if "lifetime_rewards_ore" in data:
        out["lifetime_rewards_ore"] = _to_int(data["lifetime_rewards_ore"])
    if "lifetime_deployed" in data:
        out["lifetime_deployed"] = _to_int(data["lifetime_deployed"])
    if "lifetime_rewards_sol" in data:
        out["lifetime_rewards_sol"] = _to_int(data["lifetime_rewards_sol"])
    return out


@dataclass
class Round:
    """Account `Round` as program reads decode it."""

    id: Optional[int] = None
    deployed: Optional[List[int]] = None
    mass: Optional[List[int]] = None
    count: Optional[List[int]] = None
    slot_hash: Optional[List[int]] = None
    expires_at: Optional[int] = None
    motherlode: Optional[int] = None
    rent_payer: Optional[str] = None
    rewards: Optional[List[int]] = None
    total_vaulted: Optional[int] = None
    total_returned_sol: Optional[int] = None
    total_miners: Optional[int] = None
    top_miner: Optional[str] = None


def round_from_wire(value: Any) -> Round:
    """Converts a wire payload into :class:`Round`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Round")
    return Round(
        id=_to_int(_require(data, "id", "Round")),
        deployed=_to_int_list(_require(data, "deployed", "Round")),
        mass=_to_int_list(_require(data, "mass", "Round")),
        count=_to_int_list(_require(data, "count", "Round")),
        slot_hash=_to_int_list(_require(data, "slot_hash", "Round")),
        expires_at=_to_int(_require(data, "expires_at", "Round")),
        motherlode=_to_int(_require(data, "motherlode", "Round")),
        rent_payer=_require(data, "rent_payer", "Round"),
        rewards=_to_int_list(_require(data, "rewards", "Round")),
        total_vaulted=_to_int(_require(data, "total_vaulted", "Round")),
        total_returned_sol=_to_int(_require(data, "total_returned_sol", "Round")),
        total_miners=_to_int(_require(data, "total_miners", "Round")),
        top_miner=_require(data, "top_miner", "Round"),
    )


def round_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Round` patch; only present keys appear."""
    data = _mapping(value, "Round patch")
    out: Dict[str, Any] = {}
    if "id" in data:
        out["id"] = _to_int(data["id"])
    if "deployed" in data:
        out["deployed"] = _to_int_list(data["deployed"])
    if "mass" in data:
        out["mass"] = _to_int_list(data["mass"])
    if "count" in data:
        out["count"] = _to_int_list(data["count"])
    if "slot_hash" in data:
        out["slot_hash"] = _to_int_list(data["slot_hash"])
    if "expires_at" in data:
        out["expires_at"] = _to_int(data["expires_at"])
    if "motherlode" in data:
        out["motherlode"] = _to_int(data["motherlode"])
    if "rent_payer" in data:
        out["rent_payer"] = data["rent_payer"]
    if "rewards" in data:
        out["rewards"] = _to_int_list(data["rewards"])
    if "total_vaulted" in data:
        out["total_vaulted"] = _to_int(data["total_vaulted"])
    if "total_returned_sol" in data:
        out["total_returned_sol"] = _to_int(data["total_returned_sol"])
    if "total_miners" in data:
        out["total_miners"] = _to_int(data["total_miners"])
    if "top_miner" in data:
        out["top_miner"] = data["top_miner"]
    return out


@dataclass
class OreTreasuryAccount:
    """Account `Treasury` as program reads decode it."""

    motherlode: Optional[int] = None
    miner_rewards_factor: Optional[Numeric] = None
    total_refined: Optional[int] = None
    total_unclaimed: Optional[int] = None


def ore_treasury_account_from_wire(value: Any) -> OreTreasuryAccount:
    """Converts a wire payload into :class:`OreTreasuryAccount`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "OreTreasuryAccount")
    return OreTreasuryAccount(
        motherlode=_to_int(_require(data, "motherlode", "OreTreasuryAccount")),
        miner_rewards_factor=_convert(_require(data, "miner_rewards_factor", "OreTreasuryAccount"), numeric_from_wire),
        total_refined=_to_int(_require(data, "total_refined", "OreTreasuryAccount")),
        total_unclaimed=_to_int(_require(data, "total_unclaimed", "OreTreasuryAccount")),
    )


def ore_treasury_account_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `OreTreasuryAccount` patch; only present keys appear."""
    data = _mapping(value, "OreTreasuryAccount patch")
    out: Dict[str, Any] = {}
    if "motherlode" in data:
        out["motherlode"] = _to_int(data["motherlode"])
    if "miner_rewards_factor" in data:
        out["miner_rewards_factor"] = _convert(data["miner_rewards_factor"], numeric_from_wire)
    if "total_refined" in data:
        out["total_refined"] = _to_int(data["total_refined"])
    if "total_unclaimed" in data:
        out["total_unclaimed"] = _to_int(data["total_unclaimed"])
    return out


@dataclass
class Var:
    """Account `Var` as program reads decode it."""

    authority: Optional[str] = None
    id: Optional[int] = None
    provider: Optional[str] = None
    commit: Optional[List[int]] = None
    seed: Optional[List[int]] = None
    slot_hash: Optional[List[int]] = None
    value: Optional[List[int]] = None
    samples: Optional[int] = None
    is_auto: Optional[int] = None
    start_at: Optional[int] = None
    end_at: Optional[int] = None


def var_from_wire(value: Any) -> Var:
    """Converts a wire payload into :class:`Var`.

    Raises ``ValueError`` when a required field is absent."""
    data = _mapping(value, "Var")
    return Var(
        authority=_require(data, "authority", "Var"),
        id=_to_int(_require(data, "id", "Var")),
        provider=_require(data, "provider", "Var"),
        commit=_to_int_list(_require(data, "commit", "Var")),
        seed=_to_int_list(_require(data, "seed", "Var")),
        slot_hash=_to_int_list(_require(data, "slot_hash", "Var")),
        value=_to_int_list(_require(data, "value", "Var")),
        samples=_to_int(_require(data, "samples", "Var")),
        is_auto=_to_int(_require(data, "is_auto", "Var")),
        start_at=_to_int(_require(data, "start_at", "Var")),
        end_at=_to_int(_require(data, "end_at", "Var")),
    )


def var_patch_from_wire(value: Any) -> Dict[str, Any]:
    """Converts a partial `Var` patch; only present keys appear."""
    data = _mapping(value, "Var patch")
    out: Dict[str, Any] = {}
    if "authority" in data:
        out["authority"] = data["authority"]
    if "id" in data:
        out["id"] = _to_int(data["id"])
    if "provider" in data:
        out["provider"] = data["provider"]
    if "commit" in data:
        out["commit"] = _to_int_list(data["commit"])
    if "seed" in data:
        out["seed"] = _to_int_list(data["seed"])
    if "slot_hash" in data:
        out["slot_hash"] = _to_int_list(data["slot_hash"])
    if "value" in data:
        out["value"] = _to_int_list(data["value"])
    if "samples" in data:
        out["samples"] = _to_int(data["samples"])
    if "is_auto" in data:
        out["is_auto"] = _to_int(data["is_auto"])
    if "start_at" in data:
        out["start_at"] = _to_int(data["start_at"])
    if "end_at" in data:
        out["end_at"] = _to_int(data["end_at"])
    return out
