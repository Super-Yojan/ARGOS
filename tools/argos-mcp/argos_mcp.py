#!/usr/bin/env python3
"""MCP server that lets Claude or any agent advise, or operate, the ARGOS station.

Start the ARGOS dashboard first (sim or real fleet), then point an MCP client
at this script over stdio:

    python3 tools/argos-mcp/argos_mcp.py                      # advise (default)
    python3 tools/argos-mcp/argos_mcp.py --profile operator   # also command rovers

The server talks to the dashboard window, not to Zenoh, so the agent sees the
same fleet the operator sees and every command goes through the same checks as
the window's buttons. The operator picks Off, Advise or Operator in the
window's Agent panel; an operator-profile server still gets only advice
through while the window is in Advise. Raw drive waits for the operator to
press Approve drive.
"""
import argparse
import json

from argos_bridge import Bridge, BridgeError, rules_text, system_prompt, tick_prompt, topics_text

try:  # mcp 1.x
    from mcp.server.fastmcp import FastMCP
    from mcp.server.fastmcp.exceptions import ToolError
except ModuleNotFoundError:  # mcp 2.x renamed it
    from mcp.server.mcpserver import MCPServer as FastMCP
    from mcp.server.mcpserver.exceptions import ToolError

PROFILES = ("advise", "operator")

INSTRUCTIONS = """\
You are connected to ARGOS, the operator station for a fleet of Terra rovers
playing the NEXT Competition 2026. Read prompt `recommend_tick` or resources
argos://rules and argos://topics first. get_field_state shows every rover.
In advise mode you suggest moves with propose(); the operator accepts or
rejects each card, and you can see the decisions in list_recommendations().
"""

OPERATOR_NOTE = """
Operator profile: you may also command rovers (set_level, go_to, start_task,
stop, ...). ARGOS runs the same checks as for the human operator and tells you
why it refuses. Raw drive needs request_drive() and the operator's approval;
never ask for it just to save time. Do not dump balls into buckets: drive close
and hand the rover to the operator.
"""

PROPOSE_HELP = """Put recommendation cards in front of the operator. Each item:
  rover: rover id
  kind: set_level | go_to | task | handoff_bucket | attention
  action: per kind, {"level": "explore", "budget_min": 5} | {"x": .., "y": ..}
          or {"latitude": .., "longitude": ..} | {"instruction": "..", "budget_min": 5}
          | {"object_id": 7, "approach_m": 1.5} | {}
  urgency: 1 FYI, 2 soon, 3 now
  reason: under 200 characters, with the numbers that matter
  evidence: optional list of state fields you used
  confidence: optional 0-1; expires_s: optional, default 90
At most 3 cards are open. Returns per-item results: rec_id, or why it was refused."""


def dump(value):
    return json.dumps(value, indent=1)


def call(bridge, op, **fields):
    try:
        return dump(bridge.call(op, **fields))
    except BridgeError as error:
        # ToolError reaches the agent as the tool's error text, ARGOS's reason intact.
        raise ToolError(str(error)) from None


def build_server(bridge, profile="advise"):
    if profile not in PROFILES:
        raise ValueError(f"profile is one of {PROFILES}")
    server = FastMCP("argos", instructions=INSTRUCTIONS + (OPERATOR_NOTE if profile == "operator" else ""))

    @server.prompt()
    def recommend_tick() -> str:
        """The game rules, what you can read, and the field right now: run once a minute."""
        state = bridge.call("state")
        return system_prompt() + "\n\n" + tick_prompt(state)

    @server.resource("argos://rules")
    def rules() -> str:
        """NEXT 2026 rules, the bucket rule, the L0-L5 ladder, and how to recommend."""
        return rules_text()

    @server.resource("argos://topics")
    def topics() -> str:
        """What get_field_state and read_rover contain, and the Zenoh topics behind them."""
        return topics_text()

    @server.tool()
    def get_field_state() -> str:
        """Every rover as ARGOS sees it (pose, level, safety, goal, L4/L5 status,
        found objects), plus open recommendations, recent decisions, and any drive grant."""
        return call(bridge, "state")

    @server.tool()
    def read_rover(rover: int) -> str:
        """Raw cached telemetry for one rover (autonomy/status, hardware, outdoor,
        pose quality, objects, search), each with its age in seconds."""
        return call(bridge, "rover", rover=rover)

    @server.tool()
    def list_recommendations() -> str:
        """Open cards and the last 10 decisions, with the operator's reject reasons."""
        return call(bridge, "recommendations")

    @server.tool(description=PROPOSE_HELP)
    def propose(recommendations: list[dict]) -> str:
        return call(bridge, "propose", recommendations=recommendations)

    @server.tool()
    def withdraw(rec_id: str) -> str:
        """Take down one of your open cards that no longer applies."""
        return call(bridge, "withdraw", rec_id=rec_id)

    if profile != "operator":
        return server

    @server.tool()
    def set_level(rover: int, level: str, budget_min: int | None = None) -> str:
        """Ask a rover for a level: teleop, assisted_teleop, waypoint_direct, waypoint,
        explore (needs budget_min 3, 5 or 10) or target_search. Use start_task for L5.
        The reply means sent; read the rover's level in get_field_state for the result."""
        return call(bridge, "set_level", rover=rover, level=level, budget_min=budget_min)

    @server.tool()
    def go_to(rover: int, x: float | None = None, y: float | None = None,
              latitude: float | None = None, longitude: float | None = None) -> str:
        """Send a goal from L2 or L3: x/y in the rover's local metres, or latitude/longitude
        when its navigation is outdoor_gps."""
        return call(bridge, "go_to", rover=rover, x=x, y=y, latitude=latitude, longitude=longitude)

    @server.tool()
    def cancel_goal(rover: int) -> str:
        """Cancel the rover's current goal."""
        return call(bridge, "cancel_goal", rover=rover)

    @server.tool()
    def start_task(rover: int, instruction: str, budget_min: int = 5, agent: str | None = None) -> str:
        """Start L5: the rover's on-phone model or MCP agent turns the instruction into
        checked waypoints. budget_min is 3, 5 or 10; agent is builtin or mcp (default:
        the phone's MCP agent when it has one)."""
        return call(bridge, "start_task", rover=rover, instruction=instruction,
                    budget_min=budget_min, agent=agent)

    @server.tool()
    def reply_task(rover: int, text: str) -> str:
        """Answer the rover's L5 question, or add guidance to its running task."""
        return call(bridge, "reply_task", rover=rover, text=text)

    @server.tool()
    def cancel_task(rover: int) -> str:
        """Cancel the rover's L5 task. It holds at L5."""
        return call(bridge, "cancel_task", rover=rover)

    @server.tool()
    def stop(rover: int) -> str:
        """Latch the safety stop on one rover. Only the operator can reset it."""
        return call(bridge, "stop", rover=rover)

    @server.tool()
    def stop_all() -> str:
        """Latch the safety stop on every rover."""
        return call(bridge, "stop_all")

    @server.tool()
    def request_drive(rover: int, seconds: float, reason: str) -> str:
        """Ask the operator for up to 60 s of raw drive on one rover (it must be in L0
        teleop). Nothing moves until they press Approve drive; poll drive_grant in
        get_field_state."""
        return call(bridge, "request_drive", rover=rover, seconds=seconds, reason=reason)

    @server.tool()
    def drive(rover: int, linear: float, angular: float, seconds: float = 1.0) -> str:
        """Drive during an approved grant for up to 2 s. linear/angular (-1..1) give the
        direction only (forward/back, left/right); the operator's drive limits set the speed."""
        return call(bridge, "drive", rover=rover, linear=linear, angular=angular, seconds=seconds)

    @server.tool()
    def end_drive(rover: int) -> str:
        """Release the keys and end your drive grant."""
        return call(bridge, "end_drive", rover=rover)

    return server


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--profile", choices=PROFILES, default="advise",
                        help="advise (default) or operator; the ARGOS window's mode still applies")
    parser.add_argument("--bridge", help="Path to agent-bridge.json (default: the ARGOS session folder)")
    parser.add_argument("--engine", default="mcp-agent", help="Name shown in ARGOS and the session log")
    args = parser.parse_args()
    bridge = Bridge(args.bridge, engine=args.engine)
    try:
        build_server(bridge, args.profile).run("stdio")
    finally:
        bridge.close()


if __name__ == "__main__":
    main()
