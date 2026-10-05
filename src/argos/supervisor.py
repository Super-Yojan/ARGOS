"""Fleet discovery, twist commands, and a health snapshot."""

from __future__ import annotations

import math
import threading
import time
from collections.abc import Callable

from argos.contract import (
    COMMAND_HZ,
    COMMAND_TIMEOUT_S,
    STATE_STALE_AFTER_S,
    FleetState,
    TerraTopics,
    decode_fleet_state,
    encode_fleet_size,
    encode_twist,
)
from argos.health import FleetSnapshot, build_snapshot
from argos.transport import Sample, Transport


class FleetSupervisor:
    def __init__(
        self,
        transport: Transport,
        topics: TerraTopics | None = None,
        *,
        clock: Callable[[], float] | None = None,
        sleep: Callable[[float], None] | None = None,
        state_stale_after_s: float = STATE_STALE_AFTER_S,
        command_timeout_s: float = COMMAND_TIMEOUT_S,
        command_hz: float = COMMAND_HZ,
    ) -> None:
        if state_stale_after_s <= 0 or not math.isfinite(state_stale_after_s):
            raise ValueError("state stale interval must be positive and finite")
        if command_timeout_s <= 0 or not math.isfinite(command_timeout_s):
            raise ValueError("command timeout must be positive and finite")
        if command_hz <= 0 or not math.isfinite(command_hz):
            raise ValueError("command rate must be positive and finite")
        self._transport = transport
        self._topics = topics if topics is not None else TerraTopics()
        self._clock = clock if clock is not None else time.monotonic
        self._sleep = sleep if sleep is not None else time.sleep
        self._state_stale_after_s = state_stale_after_s
        self._command_timeout_s = command_timeout_s
        self._command_hz = command_hz
        self._lock = threading.Lock()
        self._state: FleetState | None = None
        self._seen: set[int] = set()
        self._state_at: float | None = None
        self._commands: dict[int, tuple[float, float, float]] = {}
        self._notice: str | None = None
        self._started = False

    @property
    def topics(self) -> TerraTopics:
        return self._topics

    def start(self) -> None:
        if self._started:
            raise RuntimeError("supervisor already started")
        self._transport.subscribe(self._topics.fleet_state(), self._on_state)
        self._started = True

    def snapshot(self) -> FleetSnapshot:
        with self._lock:
            return build_snapshot(
                now=self._clock(),
                state=self._state,
                state_at=self._state_at,
                seen_ids=set(self._seen),
                commands=dict(self._commands),
                notice=self._notice,
                state_stale_after_s=self._state_stale_after_s,
                command_timeout_s=self._command_timeout_s,
            )

    def wait_for_state(self, timeout: float) -> FleetSnapshot:
        deadline = self._deadline(timeout)
        while True:
            snapshot = self.snapshot()
            if snapshot.link == "healthy":
                return snapshot
            if self._clock() >= deadline:
                raise TimeoutError(f"no fleet state within {timeout}s")
            self._sleep(min(0.05, deadline - self._clock()))

    def request_count(self, count: int, timeout: float = 5.0) -> FleetSnapshot:
        payload = encode_fleet_size(count)
        key = self._topics.fleet_size()
        deadline = self._deadline(timeout)
        # Learn who is already spawned so a shrink can be shown as absent.
        observe_until = min(deadline, self._clock() + min(1.0, timeout))
        while self._clock() < observe_until:
            if self.snapshot().link == "healthy":
                break
            self._sleep(min(0.05, observe_until - self._clock()))
        while True:
            self._transport.put(key, payload)
            snapshot = self.snapshot()
            if snapshot.link == "healthy" and snapshot.count == count:
                return snapshot
            if self._clock() >= deadline:
                raise TimeoutError(f"no fleet acknowledgement within {timeout}s")
            self._sleep(min(0.1, deadline - self._clock()))

    def drive(self, rover_id: int, linear: float, angular: float, seconds: float) -> None:
        if isinstance(rover_id, bool) or not isinstance(rover_id, int) or rover_id < 0:
            raise ValueError("rover id must be a nonnegative integer")
        if rover_id > 2**64 - 1:
            raise ValueError("rover id does not fit in u64")
        if not math.isfinite(seconds) or seconds <= 0:
            raise ValueError("drive duration must be positive and finite")
        payload = encode_twist(linear, angular)
        stop = encode_twist(0.0, 0.0)
        key = self._topics.cmd_vel(rover_id)
        period = 1.0 / self._command_hz
        deadline = self._clock() + seconds
        try:
            while self._clock() < deadline:
                self._transport.put(key, payload)
                self._note(rover_id, linear, angular)
                remaining = deadline - self._clock()
                if remaining <= 0:
                    break
                self._sleep(min(period, remaining))
        finally:
            self._transport.put(key, stop)
            self._note(rover_id, 0.0, 0.0)

    def _deadline(self, timeout: float) -> float:
        if not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be positive and finite")
        return self._clock() + timeout

    def _on_state(self, sample: Sample) -> None:
        try:
            state = decode_fleet_state(sample.payload)
        except ValueError as exc:
            with self._lock:
                self._notice = str(exc)
            return
        notice = None
        if state.count != len(state.ids):
            notice = "fleet state count does not match ids length"
        with self._lock:
            self._seen.update(state.ids)
            self._state = state
            self._state_at = self._clock()
            self._notice = notice

    def _note(self, rover_id: int, linear: float, angular: float) -> None:
        with self._lock:
            self._commands[rover_id] = (linear, angular, self._clock())
