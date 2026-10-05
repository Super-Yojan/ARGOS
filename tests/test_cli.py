import json

import pytest

from argos.cli import format_snapshot, main
from argos.contract import FleetState
from argos.health import build_snapshot
from argos.transport import Sample


class RecordingTransport:
    def __init__(self, state=b'{"count":1,"max_count":32,"ids":[0]}'):
        self.state = state
        self.sent = []
        self.closed = False
        self.callbacks = []

    def subscribe(self, key, callback):
        self.callbacks.append((key, callback))
        callback(Sample(key, self.state))

    def put(self, key, payload):
        self.sent.append((key, bytes(payload)))
        if key.endswith("/fleet/size"):
            count = json.loads(payload)["count"]
            body = json.dumps(
                {"count": count, "max_count": 32, "ids": list(range(count))}
            ).encode()
            for subscribed, callback in self.callbacks:
                callback(Sample(subscribed, body))

    def close(self):
        self.closed = True


def test_status_json_uses_the_session_flags(monkeypatch, capsys):
    transports = []

    def connect(endpoint):
        assert endpoint == "tcp/10.0.0.8:7447"
        transports.append(RecordingTransport())
        return transports[0]

    monkeypatch.setattr("argos.cli.ZenohTransport.connect", connect)
    assert main(["status", "--json", "--endpoint", "tcp/10.0.0.8:7447", "--timeout", "1"]) == 0
    payload = json.loads(capsys.readouterr().out)
    assert payload["link"] == "healthy"
    assert payload["ids"] == [0]
    assert payload["rovers"][0]["health"] == "online"
    assert transports[0].closed


def test_fleet_count_requests_an_acknowledgement(monkeypatch, capsys):
    transports = []

    def connect(_endpoint):
        transports.append(RecordingTransport())
        return transports[-1]

    monkeypatch.setattr("argos.cli.ZenohTransport.connect", connect)
    assert main(["fleet", "--count", "2", "--timeout", "1"]) == 0
    text = capsys.readouterr().out
    assert "count: 2" in text
    assert "ids: 0 1" in text
    assert transports[0].sent[0][0] == "terra/rover/fleet/size"


def test_drive_repeats_a_twist_and_stops(monkeypatch, capsys):
    transports = []

    def connect(_endpoint):
        transports.append(RecordingTransport())
        return transports[0]

    monkeypatch.setattr("argos.cli.ZenohTransport.connect", connect)
    code = main(
        [
            "drive",
            "--rover",
            "0",
            "--linear",
            "1",
            "--angular",
            "0.3",
            "--seconds",
            "0.05",
            "--timeout",
            "1",
        ]
    )
    assert code == 0
    assert capsys.readouterr().err == ""
    sent = transports[0].sent
    assert sent[0][0] == "terra/rover/0/cmd_vel"
    assert json.loads(sent[0][1]) == {"linear": 1.0, "angular": 0.3}
    assert json.loads(sent[-1][1]) == {"linear": 0.0, "angular": 0.0}


def test_status_fails_when_no_fleet_state_arrives(monkeypatch, capsys):
    class Silent(RecordingTransport):
        def subscribe(self, key, callback):
            self.callbacks.append((key, callback))

    monkeypatch.setattr("argos.cli.ZenohTransport.connect", lambda _endpoint: Silent())
    assert main(["status", "--timeout", "0.15"]) == 1
    assert "no fleet state" in capsys.readouterr().err


def test_bad_prefix_is_usage_and_does_not_connect(monkeypatch):
    def connect(_endpoint):
        raise AssertionError("connect should not run")

    monkeypatch.setattr("argos.cli.ZenohTransport.connect", connect)
    assert main(["status", "--prefix", "terra/*"]) == 2


def test_text_snapshot_lists_rover_health():
    snapshot = build_snapshot(
        now=1.0,
        state=FleetState(2, 32, (0, 2)),
        state_at=0.5,
        seen_ids=(0, 1),
        commands={},
        notice="fleet state count does not match ids length",
        state_stale_after_s=2.5,
        command_timeout_s=0.5,
    )
    text = format_snapshot(snapshot)
    assert "link: healthy" in text
    assert "notice: fleet state count does not match ids length" in text
    assert "id=1 health=absent motion=idle" in text
    assert "id=0 health=online motion=idle" in text


def test_help_exits_cleanly():
    with pytest.raises(SystemExit) as caught:
        main(["--help"])
    assert caught.value.code == 0
