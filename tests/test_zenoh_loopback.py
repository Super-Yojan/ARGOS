"""Client/peer check that discovery and cmd_vel cross a real Zenoh session."""

import json
import socket
import threading
import time

from argos.contract import TerraTopics, encode_fleet_size
from argos.supervisor import FleetSupervisor
from argos.transport import ZenohTransport, open_session, payload_to_bytes


def _free_endpoint():
    probe = socket.socket()
    probe.bind(("127.0.0.1", 0))
    port = probe.getsockname()[1]
    probe.close()
    return f"tcp/127.0.0.1:{port}"


def test_supervisor_discovers_and_drives_over_zenoh():
    endpoint = _free_endpoint()
    topics = TerraTopics()
    peer = open_session(mode="peer", endpoint=endpoint)
    twists = []
    fleet = {"ids": [0, 1]}
    lock = threading.Lock()

    def publish():
        with lock:
            ids = list(fleet["ids"])
        peer.put(
            topics.fleet_state(),
            json.dumps({"count": len(ids), "max_count": 32, "ids": ids}).encode(),
        )

    def on_size(sample):
        count = json.loads(payload_to_bytes(sample.payload))["count"]
        encode_fleet_size(count)
        with lock:
            fleet["ids"] = list(range(count))
        publish()

    def on_cmd(sample):
        twists.append((str(sample.key_expr), payload_to_bytes(sample.payload)))

    subscribers = [
        peer.declare_subscriber(topics.fleet_size(), on_size),
        peer.declare_subscriber(topics.cmd_vel_selector(), on_cmd),
    ]
    stop = threading.Event()

    def republish():
        while not stop.is_set():
            publish()
            stop.wait(0.1)

    thread = threading.Thread(target=republish, daemon=True)
    thread.start()
    transport = None
    try:
        deadline = time.monotonic() + 5
        last_error = None
        while time.monotonic() < deadline:
            try:
                transport = ZenohTransport.connect(endpoint)
                break
            except Exception as exc:  # peer may still be binding
                last_error = exc
                time.sleep(0.05)
        assert transport is not None, last_error
        supervisor = FleetSupervisor(transport, topics)
        supervisor.start()
        snapshot = supervisor.wait_for_state(5)
        assert snapshot.link == "healthy"
        assert snapshot.ids == (0, 1)
        assert snapshot.count == 2

        supervisor.drive(1, 0.4, -0.2, 0.15)
        wait_until = time.monotonic() + 2
        while time.monotonic() < wait_until and not _stopped(twists):
            time.sleep(0.02)
        assert any(key == "terra/rover/1/cmd_vel" and _linear(payload) == 0.4 for key, payload in twists)
        assert _stopped(twists)

        acknowledged = supervisor.request_count(3, timeout=5)
        assert acknowledged.count == 3
        assert acknowledged.ids == (0, 1, 2)
    finally:
        stop.set()
        thread.join(timeout=2)
        if transport is not None:
            transport.close()
        for subscriber in subscribers:
            subscriber.undeclare()
        peer.close()


def _linear(payload):
    return json.loads(payload)["linear"]


def _stopped(twists):
    if not twists:
        return False
    key, payload = twists[-1]
    body = json.loads(payload)
    return key == "terra/rover/1/cmd_vel" and body == {"linear": 0.0, "angular": 0.0}
