import pytest

from argos.contract import COMMAND_TIMEOUT_S, STATE_STALE_AFTER_S, FleetState
from argos.health import build_snapshot


def _state(*ids, count=None):
    return FleetState(
        count=len(ids) if count is None else count,
        max_count=32,
        ids=ids,
    )


def test_unknown_until_a_fleet_sample_arrives():
    snapshot = build_snapshot(
        now=0.0,
        state=None,
        state_at=None,
        seen_ids=(),
        commands={},
        notice=None,
        state_stale_after_s=STATE_STALE_AFTER_S,
        command_timeout_s=COMMAND_TIMEOUT_S,
    )
    assert snapshot.link == "unknown"
    assert snapshot.ids == ()
    assert snapshot.rovers == ()


def test_fresh_state_is_healthy_and_a_dropped_id_is_absent():
    snapshot = build_snapshot(
        now=1.0,
        state=_state(0, 2),
        state_at=0.8,
        seen_ids=(0, 1, 2),
        commands={2: (1.0, 0.2, 0.9)},
        notice=None,
        state_stale_after_s=STATE_STALE_AFTER_S,
        command_timeout_s=COMMAND_TIMEOUT_S,
    )
    assert snapshot.link == "healthy"
    assert snapshot.state_age_s == pytest.approx(0.2)
    by_id = {rover.rover_id: rover for rover in snapshot.rovers}
    assert by_id[0].health == "online"
    assert by_id[0].motion == "idle"
    assert by_id[1].health == "absent"
    assert by_id[2].health == "online"
    assert by_id[2].motion == "driving"
    assert by_id[2].linear == 1.0


def test_command_at_the_terra_timeout_is_idle():
    snapshot = build_snapshot(
        now=COMMAND_TIMEOUT_S,
        state=_state(0),
        state_at=COMMAND_TIMEOUT_S,
        seen_ids=(),
        commands={0: (0.5, 0.0, 0.0)},
        notice=None,
        state_stale_after_s=STATE_STALE_AFTER_S,
        command_timeout_s=COMMAND_TIMEOUT_S,
    )
    assert snapshot.rovers[0].motion == "idle"
    assert snapshot.rovers[0].command_age_s == COMMAND_TIMEOUT_S


def test_stale_link_keeps_the_last_ids_and_local_commands():
    snapshot = build_snapshot(
        now=STATE_STALE_AFTER_S,
        state=_state(4),
        state_at=0.0,
        seen_ids=(1, 4),
        commands={9: (0.0, 1.0, STATE_STALE_AFTER_S)},
        notice=None,
        state_stale_after_s=STATE_STALE_AFTER_S,
        command_timeout_s=COMMAND_TIMEOUT_S,
    )
    assert snapshot.link == "stale"
    assert snapshot.ids == (4,)
    by_id = {rover.rover_id: rover for rover in snapshot.rovers}
    assert by_id[4].health == "stale"
    assert by_id[1].health == "absent"
    assert by_id[9].health == "unlisted"
    assert by_id[9].motion == "driving"
