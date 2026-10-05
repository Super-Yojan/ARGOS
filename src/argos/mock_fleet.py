"""Local stand-in for Terra's fleet topics.

This peer publishes ``fleet/state`` and accepts ``fleet/size`` plus ``cmd_vel``.
It is only a smoke target. Terra keeps rover ids stable and may leave gaps
after shrinking the fleet; this mock renumbers surviving ids from zero.
"""

from __future__ import annotations

import argparse
import json
import threading

from argos.contract import (
    DEFAULT_ENDPOINT,
    DEFAULT_PREFIX,
    MAX_ROVERS,
    TerraTopics,
    decode_fleet_state,
    encode_fleet_size,
)
from argos.transport import open_session, payload_to_bytes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--listen", default=DEFAULT_ENDPOINT)
    parser.add_argument("--prefix", default=DEFAULT_PREFIX)
    parser.add_argument("--count", type=int, help=f"Initial rover count, 0 through {MAX_ROVERS}")
    parser.add_argument("--ids", help="Comma-separated initial ids, used when --count is omitted")
    parser.add_argument("--interval", type=float, default=1.0, help="fleet/state republish period")
    args = parser.parse_args(argv)
    if args.interval <= 0:
        parser.error("interval must be positive")
    topics = TerraTopics(args.prefix)
    ids = _initial_ids(args.count, args.ids)
    state = {"ids": ids}
    lock = threading.Lock()
    stop = threading.Event()
    last_cmd: dict[str, bytes] = {}

    def publish() -> None:
        with lock:
            payload = _state_payload(state["ids"])
        session.put(topics.fleet_state(), payload)
        print(payload.decode())

    def on_size(sample: object) -> None:
        try:
            count = json.loads(payload_to_bytes(getattr(sample, "payload")))["count"]
            encode_fleet_size(count)
        except (KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
            print(f"ignored fleet size: {exc}")
            return
        with lock:
            state["ids"] = list(range(count))
        print(f"fleet size request count={count}")
        publish()

    def on_cmd(sample: object) -> None:
        key = str(getattr(sample, "key_expr"))
        payload = payload_to_bytes(getattr(sample, "payload"))
        if last_cmd.get(key) == payload:
            return
        last_cmd[key] = payload
        print(f"cmd {key} {payload.decode(errors='replace')}")

    session = open_session(mode="peer", endpoint=args.listen)
    subscribers = [
        session.declare_subscriber(topics.fleet_size(), on_size),
        session.declare_subscriber(topics.cmd_vel_selector(), on_cmd),
    ]
    print(f"mock fleet listening on {args.listen} prefix {topics.prefix}")
    try:
        while not stop.is_set():
            publish()
            stop.wait(args.interval)
    except KeyboardInterrupt:
        return 130
    finally:
        for subscriber in subscribers:
            subscriber.undeclare()
        session.close()
    return 0


def _initial_ids(count: int | None, raw_ids: str | None) -> list[int]:
    if count is not None and raw_ids is not None:
        raise SystemExit("pass only one of --count and --ids")
    if count is not None:
        encode_fleet_size(count)
        return list(range(count))
    if raw_ids is None:
        return [0]
    if raw_ids.strip() == "":
        return []
    ids = []
    for part in raw_ids.split(","):
        try:
            rover_id = int(part.strip())
        except ValueError as exc:
            raise SystemExit(f"invalid rover id {part!r}") from exc
        if rover_id < 0:
            raise SystemExit(f"invalid rover id {rover_id}")
        ids.append(rover_id)
    try:
        decode_fleet_state(_state_payload(ids))
    except ValueError as exc:
        raise SystemExit(str(exc)) from exc
    return ids


def _state_payload(ids: list[int]) -> bytes:
    return json.dumps(
        {"count": len(ids), "max_count": MAX_ROVERS, "ids": ids}
    ).encode("utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
