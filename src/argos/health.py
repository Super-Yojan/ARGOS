"""Operator health derived from Terra fleet state and local commands.

Terra does not publish a separate health topic. Link health is how fresh
``fleet/state`` is. Motion health is how fresh this operator's last twist is,
using Terra's command watchdog (500 ms).
"""

from __future__ import annotations

from dataclasses import dataclass

from argos.contract import FleetState


@dataclass(frozen=True)
class RoverView:
    rover_id: int
    health: str
    motion: str
    linear: float | None
    angular: float | None
    command_age_s: float | None


@dataclass(frozen=True)
class FleetSnapshot:
    link: str
    state_age_s: float | None
    count: int | None
    max_count: int | None
    ids: tuple[int, ...]
    rovers: tuple[RoverView, ...]
    notice: str | None

    def to_dict(self) -> dict:
        return {
            "link": self.link,
            "state_age_s": self.state_age_s,
            "count": self.count,
            "max_count": self.max_count,
            "ids": list(self.ids),
            "rovers": [
                {
                    "id": rover.rover_id,
                    "health": rover.health,
                    "motion": rover.motion,
                    "linear": rover.linear,
                    "angular": rover.angular,
                    "command_age_s": rover.command_age_s,
                }
                for rover in self.rovers
            ],
            "notice": self.notice,
        }


def build_snapshot(
    *,
    now: float,
    state: FleetState | None,
    state_at: float | None,
    seen_ids: tuple[int, ...] | set[int],
    commands: dict[int, tuple[float, float, float]],
    notice: str | None,
    state_stale_after_s: float,
    command_timeout_s: float,
) -> FleetSnapshot:
    seen = set(seen_ids)
    if state is None or state_at is None:
        listed: list[tuple[int, str]] = []
        age = None
        link = "unknown"
        ids: tuple[int, ...] = ()
        count = None
        max_count = None
    else:
        age = max(0.0, now - state_at)
        ids = state.ids
        count = state.count
        max_count = state.max_count
        current = set(state.ids)
        missing = [rover_id for rover_id in seen if rover_id not in current]
        if age >= state_stale_after_s:
            link = "stale"
            listed = [(rover_id, "stale") for rover_id in state.ids]
            listed.extend((rover_id, "absent") for rover_id in missing)
        else:
            link = "healthy"
            listed = [(rover_id, "online") for rover_id in state.ids]
            listed.extend((rover_id, "absent") for rover_id in missing)
    known = {rover_id for rover_id, _health in listed}
    listed.extend(
        (rover_id, "unlisted") for rover_id in commands if rover_id not in known
    )
    rovers = tuple(
        _rover_view(rover_id, health, commands.get(rover_id), now, command_timeout_s)
        for rover_id, health in sorted(listed, key=lambda item: item[0])
    )
    return FleetSnapshot(
        link=link,
        state_age_s=age,
        count=count,
        max_count=max_count,
        ids=ids,
        rovers=rovers,
        notice=notice,
    )


def _rover_view(
    rover_id: int,
    health: str,
    command: tuple[float, float, float] | None,
    now: float,
    command_timeout_s: float,
) -> RoverView:
    if command is None:
        return RoverView(rover_id, health, "idle", None, None, None)
    linear, angular, sent_at = command
    age = max(0.0, now - sent_at)
    driving = (linear != 0.0 or angular != 0.0) and age < command_timeout_s
    return RoverView(
        rover_id,
        health,
        "driving" if driving else "idle",
        linear,
        angular,
        age,
    )
