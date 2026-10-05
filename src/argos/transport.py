"""Zenoh session adapter.

The supervisor talks to this small surface. Topic strings stay in
``argos.contract``; a future transport can replace Zenoh without rewriting
operator actions.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Callable, Protocol


@dataclass(frozen=True)
class Sample:
    key: str
    payload: bytes


class Transport(Protocol):
    def put(self, key: str, payload: bytes) -> None: ...

    def subscribe(self, key: str, callback: Callable[[Sample], None]) -> object: ...

    def close(self) -> None: ...


class ZenohTransport:
    def __init__(self, session: object) -> None:
        self._session = session
        self._subscribers: list[object] = []

    @classmethod
    def connect(cls, endpoint: str) -> "ZenohTransport":
        return cls(open_session(mode="client", endpoint=endpoint))

    def put(self, key: str, payload: bytes) -> None:
        self._session.put(key, payload)  # type: ignore[attr-defined]

    def subscribe(self, key: str, callback: Callable[[Sample], None]) -> object:
        def handler(sample: object) -> None:
            callback(
                Sample(
                    key=str(getattr(sample, "key_expr")),
                    payload=payload_to_bytes(getattr(sample, "payload")),
                )
            )

        subscriber = self._session.declare_subscriber(key, handler)  # type: ignore[attr-defined]
        self._subscribers.append(subscriber)
        return subscriber

    def close(self) -> None:
        for subscriber in self._subscribers:
            subscriber.undeclare()  # type: ignore[attr-defined]
        self._subscribers.clear()
        self._session.close()  # type: ignore[attr-defined]


def open_session(*, mode: str, endpoint: str) -> object:
    if mode not in {"client", "peer"}:
        raise ValueError("Zenoh mode must be client or peer")
    if not endpoint:
        raise ValueError("Zenoh endpoint is required")
    import zenoh

    config = zenoh.Config()
    config.insert_json5("mode", json.dumps(mode))
    if mode == "client":
        config.insert_json5("connect/endpoints", json.dumps([endpoint]))
    else:
        config.insert_json5("listen/endpoints", json.dumps([endpoint]))
    config.insert_json5("scouting/multicast/enabled", "false")
    return zenoh.open(config)


def payload_to_bytes(payload: object) -> bytes:
    if isinstance(payload, (bytes, bytearray)):
        return bytes(payload)
    to_bytes = getattr(payload, "to_bytes", None)
    if callable(to_bytes):
        converted = to_bytes()
        if isinstance(converted, (bytes, bytearray)):
            return bytes(converted)
    return bytes(payload)  # type: ignore[arg-type]
