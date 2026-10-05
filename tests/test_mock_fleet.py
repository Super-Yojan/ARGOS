import pytest

from argos.contract import decode_fleet_state
from argos.mock_fleet import _initial_ids, _state_payload


def test_default_and_count_ids():
    assert _initial_ids(None, None) == [0]
    assert _initial_ids(3, None) == [0, 1, 2]
    assert _initial_ids(None, "0, 2") == [0, 2]
    assert decode_fleet_state(_state_payload([0, 2])).ids == (0, 2)


def test_rejects_both_selectors_and_duplicate_ids():
    with pytest.raises(SystemExit):
        _initial_ids(1, "0")
    with pytest.raises(SystemExit):
        _initial_ids(None, "1,1")
    with pytest.raises(SystemExit):
        _initial_ids(33, None)
