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


def test_goal_encoding_matches_native_dashboard_contract():
    from argos.contract import encode_goal, decode_goal_status
    import json
    assert json.loads(encode_goal(frame="local", x=12, y=-4, token="g-1")) == {
        "frame": "local", "x": 12, "y": -4, "token": "g-1"}
    assert json.loads(encode_goal(cancel=True)) == {"cancel": True}
    assert TerraTopics().goal(8) == "terra/rover/8/goal"
    assert TerraTopics().goal_status(8) == "terra/rover/8/goal/status"
    status = decode_goal_status(b'{"state":"active","goal_id":1,"distance":12,"x":12,"y":0,"token":"g-1"}')
    assert status.token == "g-1"
    assert status.state == "active"


def test_goal_encoding_rejects_invalid_shapes():
    from argos.contract import encode_goal, decode_goal_status
    import pytest
    for fields in [dict(frame="local", x=True, y=0), dict(frame="local", x=float("nan"), y=0),
                   dict(frame="wgs84", latitude=86, longitude=0),dict(cancel=True,token="extra"),
                   dict(frame="local",x=1,y=0,latitude=0),dict(frame="local",x=1,y=0,token="bad token")]:
        with pytest.raises(ValueError):
            encode_goal(**fields)
    with pytest.raises(ValueError):
        decode_goal_status(b'{"state":"active","goal_id":0,"distance":0,"x":0,"y":0}')
