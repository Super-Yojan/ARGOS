import json

import pytest

from argos.contract import TerraTopics
from argos.supervisor import FleetSupervisor
from support import Clock, FakeTransport

STATE_KEY = "terra/rover/fleet/state"


def supervisor(transport, clock, topics=None):
    fleet = FleetSupervisor(transport, topics, clock=clock, sleep=clock.sleep)
    fleet.start()
    return fleet


def state(ids, count=None):
    return json.dumps(
        {
            "count": len(ids) if count is None else count,
            "max_count": 32,
            "ids": list(ids),
        }
    ).encode()


def test_discovery_then_absent_rover_after_shrink():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    assert fleet.snapshot().link == "unknown"
    transport.inject(STATE_KEY, state([0, 1]))
    assert fleet.snapshot().link == "healthy"
    assert fleet.snapshot().ids == (0, 1)
    transport.inject(STATE_KEY, state([0]))
    transport.inject(STATE_KEY, state([0]))
    by_id = {rover.rover_id: rover.health for rover in fleet.snapshot().rovers}
    assert by_id == {0: "online", 1: "absent"}


def test_invalid_sample_keeps_the_previous_fleet():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    transport.inject(STATE_KEY, state([3]))
    transport.inject(STATE_KEY, b"nope")
    snapshot = fleet.snapshot()
    assert snapshot.ids == (3,)
    assert snapshot.notice == "fleet state is not JSON"
    transport.inject(STATE_KEY, state([3], count=9))
    assert fleet.snapshot().notice == "fleet state count does not match ids length"
    assert fleet.snapshot().count == 9
    assert fleet.snapshot().ids == (3,)


def test_stale_after_two_missed_republishes():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    transport.inject(STATE_KEY, state([0]))
    clock.now = 2.5
    assert fleet.snapshot().link == "stale"
    assert fleet.snapshot().rovers[0].health == "stale"


def test_drive_publishes_on_the_terra_key_then_stops():
    clock = Clock()
    transport = FakeTransport()
    topics = TerraTopics("lab/fleet")
    fleet = supervisor(transport, clock, topics)
    transport.inject("lab/fleet/fleet/state", state([1]))
    seen = []

    def sleep(delay):
        seen.append(fleet.snapshot())
        clock.sleep(delay)

    fleet._sleep = sleep
    fleet.drive(1, 0.4, -0.2, 0.2)
    assert len(transport.sent) >= 2
    assert {key for key, _payload in transport.sent} == {"lab/fleet/1/cmd_vel"}
    assert json.loads(transport.sent[0][1]) == {"linear": 0.4, "angular": -0.2}
    assert json.loads(transport.sent[-1][1]) == {"linear": 0.0, "angular": 0.0}
    assert all(json.loads(payload)["linear"] == 0.4 for _key, payload in transport.sent[:-1])
    assert seen[0].rovers[0].health == "online"
    assert seen[0].rovers[0].motion == "driving"
    stopped = fleet.snapshot().rovers[0]
    assert stopped.motion == "idle"
    assert stopped.linear == 0.0


def test_drive_still_sends_zero_when_a_publish_fails():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)

    def put(key, payload):
        transport.sent.append((key, bytes(payload)))
        if len(transport.sent) == 2:
            raise RuntimeError("link lost")

    transport.put = put
    with pytest.raises(RuntimeError, match="link lost"):
        fleet.drive(0, 1.0, 0.0, 1.0)
    assert json.loads(transport.sent[-1][1]) == {"linear": 0.0, "angular": 0.0}


def test_request_count_waits_for_the_state_count():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    transport.inject(STATE_KEY, state([0]))

    def put(key, payload):
        transport.sent.append((key, bytes(payload)))
        if key.endswith("/fleet/size"):
            count = json.loads(payload)["count"]
            transport.inject(STATE_KEY, state(range(count)))

    transport.put = put
    snapshot = fleet.request_count(2, timeout=5)
    assert snapshot.count == 2
    assert snapshot.ids == (0, 1)
    assert transport.sent[0][0] == "terra/rover/fleet/size"
    assert transport.sent[0][1] == b'{"count": 2}'


def test_start_subscribes_once():
    fleet = FleetSupervisor(FakeTransport(), clock=Clock(), sleep=lambda _delay: None)
    fleet.start()
    with pytest.raises(RuntimeError):
        fleet.start()


def test_resize_keeps_ids_that_disappeared():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    transport.inject(STATE_KEY, state([0, 1, 2]))

    def put(key, payload):
        transport.sent.append((key, bytes(payload)))
        if key.endswith("/fleet/size"):
            count = json.loads(payload)["count"]
            transport.inject(STATE_KEY, state(range(count)))

    transport.put = put
    snapshot = fleet.request_count(1, timeout=5)
    assert snapshot.ids == (0,)
    by_id = {rover.rover_id: rover.health for rover in snapshot.rovers}
    assert by_id == {0: "online", 1: "absent", 2: "absent"}


def test_wait_for_state_times_out_on_the_injected_clock():
    clock = Clock()
    transport = FakeTransport()
    fleet = supervisor(transport, clock)
    with pytest.raises(TimeoutError):
        fleet.wait_for_state(0.2)
    assert clock.now == pytest.approx(0.2)
