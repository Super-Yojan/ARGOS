"""Terra Zenoh keys and payload codecs.

Terra owns the bus. Key strings and JSON shapes live here so a contract
correction does not spread through the supervisor.
"""

from __future__ import annotations

import json
import math
import struct
from dataclasses import dataclass

DEFAULT_ENDPOINT = "tcp/127.0.0.1:7447"
DEFAULT_PREFIX = "terra/rover"
MAX_ROVERS = 32
MAX_REQUEST_BYTES = 2048
MAX_STATE_BYTES = 65536
MAX_FLEET_IDS = 4096
COMMAND_HZ = 20.0
COMMAND_TIMEOUT_S = 0.5
FLEET_STATE_PERIOD_S = 1.0
STATE_STALE_AFTER_S = 2.5


@dataclass(frozen=True)
class FleetState:
    count: int
    max_count: int
    ids: tuple[int, ...]


@dataclass(frozen=True)
class TerraTopics:
    """Key expressions for one Terra prefix, default ``terra/rover``."""

    prefix: str = DEFAULT_PREFIX

    def __post_init__(self) -> None:
        prefix = self.prefix.strip().strip("/")
        if not prefix or any(part == "" for part in prefix.split("/")):
            raise ValueError("Zenoh prefix must be a non-empty concrete key")
        if "*" in prefix or "?" in prefix or "#" in prefix or "$" in prefix:
            raise ValueError("Zenoh prefix must be a concrete key with no wildcards")
        object.__setattr__(self, "prefix", prefix)

    def cmd_vel(self, rover_id: int) -> str:
        _rover_id(rover_id)
        return f"{self.prefix}/{rover_id}/cmd_vel"

    def cmd_vel_selector(self) -> str:
        return f"{self.prefix}/*/cmd_vel"

    def fleet_state(self) -> str:
        return f"{self.prefix}/fleet/state"

    def fleet_size(self) -> str:
        return f"{self.prefix}/fleet/size"


def encode_twist(linear: float, angular: float) -> bytes:
    """JSON twist Terra accepts on ``<prefix>/<id>/cmd_vel``."""
    payload = json.dumps(
        {"linear": _f32_number(linear), "angular": _f32_number(angular)}
    ).encode("utf-8")
    if len(payload) > MAX_REQUEST_BYTES:
        raise ValueError("twist payload exceeds 2048 bytes")
    return payload


def encode_fleet_size(count: int) -> bytes:
    """JSON size request Terra accepts on ``<prefix>/fleet/size``."""
    if isinstance(count, bool) or not isinstance(count, int) or not 0 <= count <= MAX_ROVERS:
        raise ValueError(f"fleet count must be an integer from 0 through {MAX_ROVERS}")
    payload = json.dumps({"count": count}).encode("utf-8")
    if len(payload) > MAX_REQUEST_BYTES:
        raise ValueError("fleet size payload exceeds 2048 bytes")
    return payload


def decode_fleet_state(payload: bytes) -> FleetState:
    """Parse ``<prefix>/fleet/state``.

    Unknown fields are kept out of the model and otherwise ignored, so Terra
    can add fields without breaking discovery. ``count`` is reported as sent;
    callers decide what to do when it disagrees with ``ids``.
    """
    if not isinstance(payload, (bytes, bytearray)):
        raise ValueError("fleet state payload must be bytes")
    if len(payload) > MAX_STATE_BYTES:
        raise ValueError("fleet state payload is too large")
    try:
        data = json.loads(payload)
    except json.JSONDecodeError as exc:
        raise ValueError("fleet state is not JSON") from exc
    if not isinstance(data, dict):
        raise ValueError("fleet state must be a JSON object")
    missing = [field for field in ("count", "max_count", "ids") if field not in data]
    if missing:
        raise ValueError(f"fleet state missing {', '.join(missing)}")
    count = _nonnegative_int(data["count"], "count")
    max_count = _nonnegative_int(data["max_count"], "max_count")
    ids_raw = data["ids"]
    if not isinstance(ids_raw, list):
        raise ValueError("fleet state ids must be a list")
    if len(ids_raw) > MAX_FLEET_IDS:
        raise ValueError("fleet state has too many ids")
    ids = tuple(_nonnegative_int(value, "id") for value in ids_raw)
    if len(set(ids)) != len(ids):
        raise ValueError("fleet state contains duplicate ids")
    return FleetState(count=count, max_count=max_count, ids=ids)


def _rover_id(rover_id: int) -> int:
    if isinstance(rover_id, bool) or not isinstance(rover_id, int) or rover_id < 0:
        raise ValueError("rover id must be a nonnegative integer")
    if rover_id > 2**64 - 1:
        raise ValueError("rover id does not fit in u64")
    return rover_id


def _nonnegative_int(value: object, name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ValueError(f"fleet state {name} must be a nonnegative integer")
    if value > 2**64 - 1:
        raise ValueError(f"fleet state {name} does not fit in u64")
    return value


def _f32_number(value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ValueError("twist components must be finite numbers")
    number = float(value)
    if not math.isfinite(number):
        raise ValueError("twist components must be finite numbers")
    packed = struct.unpack("<f", struct.pack("<f", number))[0]
    if not math.isfinite(packed):
        raise ValueError("twist components must fit in f32")
    return number
