import json
import math

import pytest

from argos.contract import (
    MAX_ROVERS,
    TerraTopics,
    decode_fleet_state,
    encode_fleet_size,
    encode_twist,
)

TERRA_STATE = b'{"count":3,"max_count":32,"ids":[0,1,2]}'


def test_topics_follow_the_terra_prefix():
    topics = TerraTopics("terra/rover/")
    assert topics.prefix == "terra/rover"
    assert topics.cmd_vel(4) == "terra/rover/4/cmd_vel"
    assert topics.cmd_vel_selector() == "terra/rover/*/cmd_vel"
    assert topics.fleet_state() == "terra/rover/fleet/state"
    assert topics.fleet_size() == "terra/rover/fleet/size"
    custom = TerraTopics("lab/fleet")
    assert custom.cmd_vel(2) == "lab/fleet/2/cmd_vel"
    assert custom.fleet_state() == "lab/fleet/fleet/state"


def test_prefix_rejects_wildcards_and_empty_segments():
    for prefix in ("", "/", "*", "terra/*", "terra//rover", "terra/?"):
        with pytest.raises(ValueError):
            TerraTopics(prefix)


def test_twist_matches_terra_json_and_rejects_non_finite_values():
    assert encode_twist(1, 0.3) == b'{"linear": 1.0, "angular": 0.3}'
    assert json.loads(encode_twist(0.0, -0.25)) == {"linear": 0.0, "angular": -0.25}
    for linear, angular in ((math.nan, 0.0), (0.0, math.inf), (1e39, 0.0), (True, 0.0)):
        with pytest.raises(ValueError):
            encode_twist(linear, angular)


def test_fleet_size_matches_terra_limits():
    assert encode_fleet_size(0) == b'{"count": 0}'
    assert encode_fleet_size(MAX_ROVERS) == b'{"count": 32}'
    for count in (-1, 33, 1.5, True, False):
        with pytest.raises(ValueError):
            encode_fleet_size(count)


def test_fleet_state_decodes_terra_sample_and_keeps_id_order():
    state = decode_fleet_state(TERRA_STATE)
    assert state.count == 3
    assert state.max_count == 32
    assert state.ids == (0, 1, 2)
    gapped = decode_fleet_state(b'{"count":2,"max_count":32,"ids":[0,2],"extra":true}')
    assert gapped.ids == (0, 2)


@pytest.mark.parametrize(
    "payload",
    [
        b"not-json",
        b"[]",
        b'{"count":1,"max_count":32}',
        b'{"count":1,"max_count":32,"ids":[0,0]}',
        b'{"count":true,"max_count":32,"ids":[0]}',
        b'{"count":1,"max_count":32,"ids":[-1]}',
        b'{"count":1.0,"max_count":32,"ids":[0]}',
    ],
)
def test_fleet_state_rejects_bad_payloads(payload):
    with pytest.raises(ValueError):
        decode_fleet_state(payload)
