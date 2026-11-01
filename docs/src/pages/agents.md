---
layout: ../layouts/DocPage.astro
title: Agents and recommendations
description: Let Claude Code, Codex, OpenCode, Pi or any MCP agent advise the operator once a minute, or operate the fleet through the same checks as the window's buttons.
---

ARGOS can take advice from an AI agent, and, when the operator allows it, take commands. The agent reaches ARGOS through an MCP server built into the ARGOS binary, `argos-gpui mcp`. That server talks to the open window, not to Zenoh, so the agent sees the same fleet you see and every command goes through the same checks as a button press. It works the same in **Sim** and **Real**, and needs no Python.

```
agent CLI (Claude Code, Codex, OpenCode, Pi) or any MCP client
   │  MCP over stdio                       (Pi: `argos-gpui call` from bash)
argos-gpui mcp
   │  one JSON line per request, 127.0.0.1 only, token from agent-bridge.json
ARGOS window  ── the operator's own checks ──▶  Zenoh  ──▶  Terra / Zorvane
```

## The Agent panel

The panel at the bottom left picks what an agent may do. You choose; the agent cannot change it.

| Mode | The agent can | Default |
| --- | --- | --- |
| **Off** | Nothing but say hello | |
| **Advise** | Read the field and put up to 3 recommendation cards in front of you | ✓ |
| **Operator** | Also set levels, send goals, start and answer L5 tasks, stop rovers, and ask for raw drive | |

Advise is the default so study sessions stay clean. Switch to Operator only when you mean to let the agent act.

## Recommendation cards

Each card names a rover, what to do and why, with a tag: **NOW**, **SOON** or **FYI**. **Accept** sends it at once, exactly as if you had pressed the button yourself. **Reject** asks for a reason (wrong rover, bad timing, unsafe, already doing it, other), which the agent sees on its next look. A card disappears by itself after 90 s, or sooner when it no longer applies (the rover left, or already runs that level).

| Card | What Accept does |
| --- | --- |
| Set level | Sends the level. L4 Explore carries its 3, 5 or 10 min budget |
| Go to | Sends a goal. If the rover is not in L2 or L3 it first switches to L3 Waypoint |
| Task | Starts an L5 task with the agent's instruction and budget |
| Hand off at bucket | Drives to 1.5 m short of a found bucket, then tells you it is your turn. It never dumps; you line up and dump by hand |
| Needs you | Nothing is sent. **Got it** closes the card |

At most 3 cards are open. A rejected idea stays away for 3 minutes. A card never uses a rover's study condition level: when ARGOS has locked a rover to a study level, a card that asks for another level is refused like the button would be.

## Operator mode

In Operator mode an agent's commands run through the same code as the window's controls, so they meet every check you do: the control lease, the study condition lock, L4 and L5 refusals, the GPS checks, a latched stop, and the goal rules. ARGOS answers with the same reason it would show you. An agent cannot command a rover you are driving. **Stop** and **Stop all** always work, and only you can reset a stop.

**Raw drive** needs your approval every time. The agent asks for up to 60 s on one rover, with a reason; the panel shows the request in amber, and nothing moves until you press **Approve drive**. The rover must be in L0 Teleop. During the grant the agent sends short bursts (2 s at most) that press the same W/A/S/D keys you would, under your drive caps. **Deny drive** or **End drive** ends it at once. So does Stop, taking the wheel yourself, switching Sim and Real, or changing the agent mode. An unanswered request lapses after 60 s.


## Run an agent

### Pick an engine in the panel

The panel's **Engine** row picks a coding-agent CLI that ARGOS runs as its recommendation engine: **Claude** (Claude Code), **Codex**, **OpenCode** or **Pi**. **None** runs nothing. Install the CLI and sign in to it once; it brings its own login and model, so ARGOS needs no API key. `ARGOS_AGENT_CLI=claude|codex|opencode|pi` picks one at launch, which suits study runs.

ARGOS runs the CLI once per tick, headless, every 60 s, or sooner (at most every 15 s) when something changes: a rover finds an object, holds, stops, finishes exploring, arrives, or asks an L5 question, or you decide a card. One run at a time; a run over 180 s is stopped. Each run gets ARGOS's MCP server and a short prompt: read `get_briefing` (the rules, the ladder, what each field means, and the field now), then `propose` up to 3 cards or say "no recommendation". In Operator mode the run gets the operator tools too. Nothing runs while the panel is Off.

| Engine | How ARGOS runs it | Tested |
| --- | --- | --- |
| Claude | `claude -p … --mcp-config argos-mcp-claude.json --strict-mcp-config --allowedTools mcp__argos` | Yes, sim with no rovers |
| Codex | `codex exec --skip-git-repo-check -c mcp_servers.argos.command=… -c mcp_servers.argos.args=[…] …` | No |
| OpenCode | `OPENCODE_CONFIG=argos-mcp-opencode.json opencode run …` | No |
| Pi | `pi --tools bash -p …`. Pi has no MCP client, so the prompt tells it to use `argos-gpui call briefing` and `argos-gpui call propose '{…}'` | No |

A local model (Qwen through llama.cpp, vLLM or Ollama) runs through OpenCode or Pi with that CLI's own provider settings. Every run's output, including its final answer, goes to `argos-agent-<engine>-<time>.log` in the session folder, and the panel shows the last answer.

### Any MCP client

Point the client at the ARGOS binary. For example, `.mcp.json` for Claude Code:

```json
{
  "mcpServers": {
    "argos": {
      "command": "/path/to/argos-gpui",
      "args": ["mcp", "--profile", "advise"]
    }
  }
}
```

Start ARGOS first: it writes `agent-bridge.json` (port and a fresh token, readable only by you) into its session folder, `Documents/ARGOS Sessions` or `ARGOS_SESSION_DIR`. Use `--profile operator` to give the agent the command tools as well; the panel's mode still decides what gets through. In Operator mode a command takes this window's control lease itself, so nobody has to press **Take control** first. The server also offers the `recommend_tick` prompt and the `argos://rules` and `argos://topics` resources. `--engine NAME` sets the name the panel shows; `--bridge FILE` points at another `agent-bridge.json`.

From a shell, `argos-gpui call <op> [json]` makes one call: `argos-gpui call briefing`, `argos-gpui call state`, `argos-gpui call propose '{"recommendations": [...]}'`.

### A command of your own

`ARGOS_AGENT_CMD` starts any long-running command once the bridge is up, with `ARGOS_AGENT_BRIDGE` and `ARGOS_AGENT_PARENT` set. Its output goes to `argos-agent-<time>.log`, and ARGOS stops it when the window closes. Use a single command (a script is fine) so closing ARGOS stops all of it.

### Is the agent connected?

The line under the mode buttons says:

| Panel says | Meaning | What to do |
| --- | --- | --- |
| `<engine> · last call N s ago` | Connected | Nothing |
| `… · waiting for an agent` | First 20 s after launch | Wait |
| **Agent not connected. Start one: …** (amber) | The bridge listens, but no agent has called in 20 s | Pick an engine, or start your MCP client |
| **Agent not connected after N s. ARGOS started …** (amber) | The engine runs but has not called in | Read its `argos-agent-…log` |
| **Agent command stopped: …** (amber) | The last run failed (for example "`codex` is not installed or not on PATH", or the CLI's own error) | Fix it; the next tick tries again |
| **An agent tried to connect with a stale token** (amber) | An agent still holds an earlier run's token | Restart the agent |
| **`<engine>` · silent for N s** (amber) | Connected once, no call for 3 min | Check that the agent is still running |

## What is logged

Everything lands in the session log next to the rest of the run, so a study can tell the operator's actions from the agent's:

| Event | When |
| --- | --- |
| `agent_hello`, `agent_mode` | An agent connects; you change the mode |
| `agent_engine`, `agent_run` | You pick an engine; each run starts (with why) and ends (ok, seconds, last line) |
| `agent_missing`, `agent_process` | No agent called in the first 20 s; the `ARGOS_AGENT_CMD` command exited |
| `rec_shown`, `rec_decided`, `rec_dispatched`, `rec_outcome` | A card appears; you accept, reject or it expires; what was sent; whether the rover got there |
| `agent_command` | Every Operator-mode command, with ARGOS's answer |
| `drive_grant`, `agent_drive` | Raw drive requests, approvals, bursts and endings |
| `intervention` | Now carries `source`: `operator`, `agent` or `recommendation` |

Commands carry the source in their token too (`gpui-agent-…`, `gpui-rec-…`). Each engine run's full output is in its `argos-agent-<engine>-<time>.log`.

## Settings

Read from the environment only, not `.env`.

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_AGENT` | on | `0` turns the agent bridge off; the panel then stays Off |
| `ARGOS_AGENT_PORT` | 7471 | Local port for the bridge. `0` picks a free one |
| `ARGOS_AGENT_BRIDGE` | the session folder's file | Where the MCP server looks for `agent-bridge.json` |
| `ARGOS_AGENT_CLI` | none | Engine at launch: `claude`, `codex`, `opencode` or `pi` |
| `ARGOS_AGENT_CMD` | unset | A long-running command ARGOS starts once the bridge is up |
