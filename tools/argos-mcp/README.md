# ARGOS MCP

Lets Claude, or any MCP agent, advise the ARGOS operator or operate the fleet
through the open ARGOS window. The full guide, with the Agent panel, the cards,
Operator mode and the session log, is [docs/src/pages/agents.md](../../docs/src/pages/agents.md).

| File | What it is |
| --- | --- |
| `argos_mcp.py` | MCP server over stdio. `--profile advise` (default) or `operator` |
| `recommend_loop.py` | Runs a model as the recommendation engine every 60 s: Claude, a local OpenAI-style server (Qwen), or `dry` |
| `argos_bridge.py` | Client for the window's local bridge, shared by both |
| `rules.md`, `topics.md` | The system prompt: game rules, autonomy ladder, what each field means |
| `test_argos_mcp.py` | Tests against a fake window |

```sh
pip install -r requirements.txt
# start ARGOS, then:
python3 argos_mcp.py --profile advise          # from an MCP client's config
python3 recommend_loop.py --backend dry --once  # see the prompt a model gets
python3 -m unittest test_argos_mcp              # tests; no ARGOS needed
```

The window writes `agent-bridge.json` (port and token) to its session folder,
`~/Documents/ARGOS Sessions` or `ARGOS_SESSION_DIR`; set `ARGOS_AGENT_BRIDGE`
to point somewhere else. The panel's mode in the window always wins over the
server's profile.
