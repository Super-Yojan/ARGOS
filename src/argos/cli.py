"""Minimal ARGOS operator CLI."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from contextlib import contextmanager

from argos.contract import DEFAULT_ENDPOINT, DEFAULT_PREFIX, TerraTopics
from argos.health import FleetSnapshot
from argos.supervisor import FleetSupervisor
from argos.transport import ZenohTransport


def main(argv: list[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        return args.func(args)
    except KeyboardInterrupt:
        return 130
    except ValueError as exc:
        print(exc, file=sys.stderr)
        return 2
    except TimeoutError as exc:
        print(exc, file=sys.stderr)
        return 1
    except ConnectionError as exc:
        print(exc, file=sys.stderr)
        return 1


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="argos",
        description="Supervise a Terra rover fleet over Zenoh.",
    )
    sub = parser.add_subparsers(dest="command", required=True)

    status = sub.add_parser("status", help="Print one fleet and health snapshot")
    _add_session_args(status)
    status.add_argument("--json", action="store_true", help="Print the snapshot as JSON")
    status.set_defaults(func=cmd_status)

    watch = sub.add_parser("watch", help="Print fleet health until interrupted")
    _add_session_args(watch)
    watch.add_argument("--interval", type=float, default=0.5, help="Seconds between prints")
    watch.set_defaults(func=cmd_watch)

    fleet = sub.add_parser("fleet", help="Discover rover ids, or request a fleet size")
    _add_session_args(fleet)
    fleet.add_argument("--count", type=int, help="Requested rover count, 0 through 32")
    fleet.add_argument("--json", action="store_true", help="Print the snapshot as JSON")
    fleet.set_defaults(func=cmd_fleet)

    drive = sub.add_parser(
        "drive",
        help="Temporary debug twist on cmd_vel, then zero. See docs/DESIGN.md",
    )
    _add_session_args(drive)
    drive.add_argument("--rover", type=int, required=True, help="Rover id from fleet/state")
    drive.add_argument("--linear", type=float, default=0.0, help="Forward speed in m/s")
    drive.add_argument("--angular", type=float, default=0.0, help="Left turn in rad/s")
    drive.add_argument("--seconds", type=float, default=5.0, help="How long to repeat the twist")
    drive.set_defaults(func=cmd_drive)
    return parser


def cmd_status(args: argparse.Namespace) -> int:
    with session(args) as supervisor:
        snapshot = supervisor.wait_for_state(args.timeout)
    _emit(snapshot, as_json=args.json)
    return 0


def cmd_watch(args: argparse.Namespace) -> int:
    if not _positive(args.interval):
        raise ValueError("interval must be positive and finite")
    with session(args) as supervisor:
        while True:
            if sys.stdout.isatty():
                sys.stdout.write("\033[H\033[2J")
            _emit(supervisor.snapshot(), as_json=False)
            sys.stdout.flush()
            time.sleep(args.interval)


def cmd_fleet(args: argparse.Namespace) -> int:
    with session(args) as supervisor:
        if args.count is None:
            snapshot = supervisor.wait_for_state(args.timeout)
        else:
            snapshot = supervisor.request_count(args.count, args.timeout)
    _emit(snapshot, as_json=args.json)
    return 0


def cmd_drive(args: argparse.Namespace) -> int:
    with session(args) as supervisor:
        try:
            snapshot = supervisor.wait_for_state(args.timeout)
        except TimeoutError:
            print(
                "warning: fleet state was not received; sending the twist anyway",
                file=sys.stderr,
            )
        else:
            if args.rover not in snapshot.ids:
                print(
                    f"warning: rover {args.rover} is outside the current fleet ids "
                    f"{list(snapshot.ids)}; Terra ignores commands for inactive ids",
                    file=sys.stderr,
                )
        supervisor.drive(args.rover, args.linear, args.angular, args.seconds)
    return 0


def format_snapshot(snapshot: FleetSnapshot) -> str:
    lines = [
        f"link: {snapshot.link}",
        f"state_age_s: {_num(snapshot.state_age_s)}",
        f"count: {_int(snapshot.count)}",
        f"max_count: {_int(snapshot.max_count)}",
        "ids: " + (" ".join(str(rover_id) for rover_id in snapshot.ids) or "-"),
    ]
    if snapshot.notice:
        lines.append(f"notice: {snapshot.notice}")
    lines.append("rovers:")
    if not snapshot.rovers:
        lines.append("  (none)")
    for rover in snapshot.rovers:
        lines.append(
            "  "
            f"id={rover.rover_id} health={rover.health} motion={rover.motion} "
            f"linear={_num(rover.linear)} angular={_num(rover.angular)} "
            f"cmd_age_s={_num(rover.command_age_s)}"
        )
    return "\n".join(lines)


@contextmanager
def session(args: argparse.Namespace):
    topics = TerraTopics(args.prefix)
    try:
        transport = ZenohTransport.connect(args.endpoint)
    except ValueError:
        raise
    except Exception as exc:
        raise ConnectionError(
            f"could not open Zenoh session to {args.endpoint}: {exc}"
        ) from exc
    supervisor = FleetSupervisor(transport, topics)
    supervisor.start()
    try:
        yield supervisor
    finally:
        transport.close()


def _add_session_args(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--endpoint",
        default=os.environ.get("ARGOS_ZENOH_ENDPOINT", DEFAULT_ENDPOINT),
        help=f"Zenoh connect endpoint (default: {DEFAULT_ENDPOINT})",
    )
    parser.add_argument(
        "--prefix",
        default=os.environ.get("ARGOS_ZENOH_PREFIX", DEFAULT_PREFIX),
        help=f"Terra key prefix (default: {DEFAULT_PREFIX})",
    )
    parser.add_argument(
        "--timeout",
        type=float,
        default=5.0,
        help="Seconds to wait for fleet state or a size acknowledgement",
    )


def _emit(snapshot: FleetSnapshot, *, as_json: bool) -> None:
    if as_json:
        print(json.dumps(snapshot.to_dict(), indent=2))
    else:
        print(format_snapshot(snapshot))


def _positive(value: float) -> bool:
    return value > 0 and value == value and value not in {float("inf"), float("-inf")}


def _num(value: float | None) -> str:
    if value is None:
        return "-"
    return f"{value:.3f}"


def _int(value: int | None) -> str:
    if value is None:
        return "-"
    return str(value)
