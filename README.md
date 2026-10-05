# ARGOS

ARGOS (Adaptive Robotic Group Operator System) supervises a Terra rover fleet over Zenoh.

This repository is the operator slice: discover rover ids, send `cmd_vel` twists, and show basic link and motion health. Terra (https://github.com/Super-Yojan/Terra) owns the simulator and the topic contract. The first slice is tracked on the [ARGOS project board](https://github.com/users/Super-Yojan/projects/6).

Design, topic table, and out-of-scope items: [docs/DESIGN.md](docs/DESIGN.md).

## Run against Terra

Start the simulator with its Zenoh bridge (default listen `tcp/127.0.0.1:7447`, prefix `terra/rover`):

```sh
cd /path/to/Terra/simulator
TERRA_ROVER_COUNT=3 cargo run
```

In another environment, install ARGOS and talk to that session:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -e '.[dev]'

argos fleet
argos status
argos drive --rover 0 --linear 1.0 --angular 0.3 --seconds 5
argos watch
```

`drive` repeats the twist at 20 Hz, then sends a zero twist. Terra stops a rover 500 ms after the last valid command.

From another machine, start Terra with `TERRA_ZENOH_LISTEN=tcp/0.0.0.0:7447` and point ARGOS at it:

```sh
argos status --endpoint tcp/SIMULATOR_IP:7447
```

If Terra was started with `TERRA_ZENOH_PREFIX`, pass the same value to `--prefix` (or set `ARGOS_ZENOH_PREFIX`). `ARGOS_ZENOH_ENDPOINT` overrides the default endpoint.

Request a fleet size only when you want Terra to spawn or remove rovers:

```sh
argos fleet --count 5
```

`status --json` and `fleet --json` print the same snapshot as a single JSON object.

## Run without the simulator

`mock_fleet` publishes `fleet/state` and logs twists. It is a smoke peer, not Terra: shrinking the fleet renumbers ids from zero instead of retiring them.

```sh
python -m argos.mock_fleet --listen tcp/127.0.0.1:7447 --count 3
```

Leave that process running, then use the `argos` commands above.

## Tests

```sh
python -m pytest
```

Unit tests use an in-memory transport. One test opens a real Zenoh peer and client on localhost, still without Terra.

## Layout

| Path | Role |
| --- | --- |
| `src/argos/contract.py` | Topic names and JSON codecs. Change these if Terra's keys move. |
| `src/argos/supervisor.py` | Discover, resize, and drive |
| `src/argos/health.py` | Link and motion health derived from fleet state |
| `src/argos/cli.py` | `status`, `watch`, `fleet`, `drive` |
| `src/argos/mock_fleet.py` | Local stand-in for Terra's three supervision keys |
