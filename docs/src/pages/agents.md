---
layout: ../layouts/DocPage.astro
title: Agents and recommendations
description: Let Claude or any MCP agent advise the operator once a minute, or operate the fleet through the same checks as the window's buttons.
---

ARGOS can take advice from an AI agent, and, when the operator allows it, take commands. The agent reaches ARGOS through an MCP server in `tools/argos-mcp`. That server talks to the open window, not to Zenoh, so the agent sees the same fleet you see and every command goes through the same checks as a button press. It works the same in **Sim** and **Real**.

```
agent (Claude Code, Claude, a local model)
   │  MCP over stdio
tools/argos-mcp/argos_mcp.py
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

Start ARGOS first. It writes `agent-bridge.json` (port and a fresh token, readable only by you) into its session folder, `Documents/ARGOS Sessions` or `ARGOS_SESSION_DIR`. Then install the server's one dependency:

```sh
pip install -r tools/argos-mcp/requirements.txt
```

### Claude Code, or any MCP client

Add the server to the client's MCP config, for example `.mcp.json` in your project:

```json
{
  "mcpServers": {
    "argos": {
      "command": "python3",
      "args": ["tools/argos-mcp/argos_mcp.py", "--profile", "advise"]
    }
  }
}
```

Use `--profile operator` to give the agent the command tools as well; the panel's mode still decides what gets through, and commands other than Stop need this window to hold control (**Take control**). For a recommendation every minute in Claude Code, run `/loop 1m` with the server's `recommend_tick` prompt. That prompt carries the game rules, the autonomy ladder, what each field means, and the field right now.

### The built-in loop

`recommend_loop.py` runs a model as the recommendation engine without an MCP client. It reads the field every 5 s and asks the model every 60 s, or sooner (at most every 15 s) when something changes: a rover finds an object, holds, stops, finishes exploring, arrives, or asks an L5 question, or you decide a card.

```sh
python3 tools/argos-mcp/recommend_loop.py                    # Claude; needs the anthropic package and an API key
python3 tools/argos-mcp/recommend_loop.py --backend local \
    --base-url http://localhost:8080/v1 --model qwen3.5      # any OpenAI-style server: llama.cpp, vLLM, Ollama
python3 tools/argos-mcp/recommend_loop.py --backend dry --once   # print the prompt; calls no model
```

The loop pauses while the panel is Off.

## What is logged

Everything lands in the session log next to the rest of the run, so a study can tell the operator's actions from the agent's:

| Event | When |
| --- | --- |
| `agent_hello`, `agent_mode` | An agent connects; you change the mode |
| `rec_shown`, `rec_decided`, `rec_dispatched`, `rec_outcome` | A card appears; you accept, reject or it expires; what was sent; whether the rover got there |
| `agent_command` | Every Operator-mode command, with ARGOS's answer |
| `drive_grant`, `agent_drive` | Raw drive requests, approvals, bursts and endings |
| `intervention` | Now carries `source`: `operator`, `agent` or `recommendation` |

Commands carry the source in their token too (`gpui-agent-…`, `gpui-rec-…`). The built-in loop also writes `argos-recommend-<time>.jsonl` with each tick's state, prompt hash, tool calls, answers and timing.

## Settings

Read from the environment only, not `.env`.

| Variable | Default | Role |
| --- | --- | --- |
| `ARGOS_AGENT` | on | `0` turns the agent bridge off; the panel then stays Off |
| `ARGOS_AGENT_PORT` | 7471 | Local port for the bridge. `0` picks a free one |
| `ARGOS_AGENT_BRIDGE` | the session folder's file | Where the MCP server looks for `agent-bridge.json` |
