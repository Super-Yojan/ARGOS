"""Publish OptiTrack rigid bodies from Motive (NatNet 3.x or 4.x) to the ARGOS zenoh router.

Run on the Motive computer from a folder that has a working NatNetClient.py
(the SDK's PythonClient sample, or OptitrackNode's windows folder for Motive 3):

    pip install "eclipse-zenoh>=1.0,<2"
    python optitrack_to_argos.py --password <ARGOS_ZENOH_PASSWORD>

Each Motive frame becomes one message per rigid body on `optitrack/pose`:

    {"id": 1, "frame": 12345, "timestamp": 51.204166,
     "position": [x, y, z], "orientation": [qx, qy, qz, qw]}

`timestamp` is Motive's frame time in seconds; the analysis uses it, not the
network arrival time, so delivery jitter does not reach the mocap velocity.
In Motive: Streaming → Broadcast Frame Data on, Local Interface = Loopback
(or pass --local-ip with the interface Motive streams on). Either up axis is fine.
"""

import argparse
import inspect
import getpass
import json
import os
import threading
import time

import zenoh
from NatNetClient import NatNetClient

ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
ap.add_argument(
    "--endpoint",
    default=os.environ.get(
        "ARGOS_ZENOH_ENDPOINT", "tls/argos.34.135.204.210.sslip.io:7448"
    ),
)
ap.add_argument("--user", default=os.environ.get("ARGOS_ZENOH_USER", "terra"))
ap.add_argument("--password", default=os.environ.get("ARGOS_ZENOH_PASSWORD"))
ap.add_argument("--key", default="optitrack/pose")
ap.add_argument(
    "--server-ip",
    default="127.0.0.1",
    help="Motive computer address (default this one)",
)
ap.add_argument(
    "--local-ip",
    default="127.0.0.1",
    help="interface Motive streams on (default loopback)",
)
ap.add_argument(
    "--unicast",
    action="store_true",
    help="NatNet 4 clients: use unicast instead of multicast",
)
ap.add_argument(
    "--every",
    type=int,
    default=1,
    help="publish every Nth frame (2 halves 240 Hz to 120 Hz)",
)
ap.add_argument(
    "--out",
    help="also write every message to this JSONL file (same format as mocap_record)",
)
ap.add_argument(
    "--offline",
    action="store_true",
    help="don't connect to the router; only write --out",
)
args = ap.parse_args()
if args.offline and not args.out:
    ap.error("--offline needs --out FILE")
log = open(args.out, "a", buffering=1) if args.out else None
if args.offline:
    args.password = ""
elif args.password is None:
    args.password = getpass.getpass("ARGOS router password (empty for none): ")

try:
    zenoh.init_log_from_env_or(
        "warn"
    )  # set RUST_LOG=zenoh=debug for connection details
except AttributeError:
    pass
conf = zenoh.Config()
conf.insert_json5("mode", '"client"')
conf.insert_json5("connect/endpoints", json.dumps([args.endpoint]))
conf.insert_json5("scouting/multicast/enabled", "false")
if args.password:
    conf.insert_json5("transport/auth/usrpwd/user", json.dumps(args.user))
    conf.insert_json5("transport/auth/usrpwd/password", json.dumps(args.password))
if args.offline:
    session, publisher = None, None
    print(f"offline: writing to {args.out} only", flush=True)
else:
    print(f"connecting to {args.endpoint}", flush=True)
    try:
        session = zenoh.open(conf)
    except Exception as e:
        raise SystemExit(
            f"could not reach the router: {e}\n"
            "check the password, and that this network allows outbound TCP to port 7448; "
            "or run with --offline --out mocap_motive.jsonl and copy the file afterwards"
        )
    publisher = session.declare_publisher(args.key)

# NatNet calls the rigid-body listener for each body, then the frame listener
# with that frame's timestamp, all on its data thread.
bodies = []
stats = {"frames": 0, "sent": 0, "ids": set()}
lock = threading.Lock()


def on_rigid_body(body_id, position, rotation):
    bodies.append((body_id, position, rotation))


def on_frame(*fields):
    # NatNet 3 passes the frame fields positionally; NatNet 4 passes one dict.
    if len(fields) == 1 and isinstance(fields[0], dict):
        d = fields[0]
        frame = int(d.get("frame_number", d.get("frameNumber", 0)))
        timestamp = float(d.get("timestamp", float("nan")))
    else:
        frame, timestamp = int(fields[0]), float(fields[8])
    frame_bodies = list(bodies)
    bodies.clear()
    with lock:
        stats["frames"] += 1
    if frame % args.every:
        return
    if timestamp != timestamp:  # NaN is not valid JSON
        timestamp = None
    for body_id, p, q in frame_bodies:
        p, q = tuple(float(v) for v in p), tuple(float(v) for v in q)
        # Motive sends an untracked body as an all-zero pose; skip it.
        if p == (0.0, 0.0, 0.0) and q == (0.0, 0.0, 0.0, 0.0):
            continue
        payload = {
            "id": body_id,
            "frame": frame,
            "timestamp": timestamp,
            "position": list(p),
            "orientation": list(q),
        }
        if publisher is not None:
            publisher.put(json.dumps(payload))
        if log is not None:
            log.write(
                json.dumps(
                    {
                        "recv_utc_ns": time.time_ns(),
                        "recv_mono_ns": time.monotonic_ns(),
                        "key": args.key,
                        "payload": payload,
                    }
                )
                + "\n"
            )
        with lock:
            stats["sent"] += 1
            stats["ids"].add(body_id)


client = NatNetClient()
if hasattr(client, "set_server_address"):  # NatNet 4.x client
    client.set_server_address(args.server_ip)
    client.set_client_address(args.local_ip)
    client.set_use_multicast(not args.unicast)
    if hasattr(client, "set_print_level"):
        client.set_print_level(0)
    client.rigid_body_listener = on_rigid_body
    client.new_frame_listener = on_frame
else:  # NatNet 3.x client
    client.serverIPAddress = args.server_ip
    client.localIPAddress = args.local_ip
    client.rigidBodyListener = on_rigid_body
    client.newFrameListener = on_frame
started = (
    client.run("d") if len(inspect.signature(client.run).parameters) else client.run()
)
if started is False:
    raise SystemExit(
        "could not start the NatNet client; check --server-ip/--local-ip and --unicast"
    )
print(
    f"listening to Motive at {args.server_ip}, publishing on {args.key}; Ctrl-C to stop",
    flush=True,
)

last = time.time()
while True:
    try:
        time.sleep(5)
    except KeyboardInterrupt:
        if session is not None:
            session.close()
        if log is not None:
            log.close()
        os._exit(0)  # NatNetClient's receive threads never exit on their own
    now = time.time()
    with lock:
        frames, sent, ids = stats["frames"], stats["sent"], sorted(stats["ids"])
        stats["frames"], stats["sent"] = 0, 0
    print(
        f"Motive {frames / (now - last):6.1f} frames/s   published {sent / (now - last):6.1f} msg/s   bodies {ids}",
        flush=True,
    )
    if frames == 0:
        print(
            "   no frames from Motive: check Streaming → Broadcast Frame Data and --local-ip",
            flush=True,
        )
    last = now

