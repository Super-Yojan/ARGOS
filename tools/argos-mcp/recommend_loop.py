#!/usr/bin/env python3
"""Run a model as ARGOS's recommendation engine once a minute.

Every tick reads the field from the ARGOS window, gives the model the fixed
system prompt (game rules, ladder, what it can read) plus the field state, and
lets it call two tools: read_rover for detail and propose for cards. Ticks also
come early, at most every 15 s, when something changes: a new found object, a
hold, an L4 run ending, an L5 question, or the operator deciding a card.

    python3 recommend_loop.py                                  # Claude (ANTHROPIC_API_KEY or `ant auth login`)
    python3 recommend_loop.py --backend local --base-url http://localhost:8080/v1 --model qwen3.5
    python3 recommend_loop.py --backend dry --once             # print the prompt, call no model

Each tick is one line in argos-recommend-<time>.jsonl in the ARGOS session
folder: the state and its hash, what the model said and called, what ARGOS
answered, latency and token use. Use any other agent through argos_mcp.py.
"""
import argparse
import json
import time
import urllib.request

from argos_bridge import Bridge, BridgeError, exit_with_argos, session_dir, state_digest, system_prompt, tick_prompt

CLAUDE_MODEL = "claude-opus-5-5"
MAX_TURNS = 4
EARLY_TICK_MIN_S = 15.0
POLL_S = 5.0

PROPOSE_SCHEMA = {
    "type": "object",
    "properties": {
        "recommendations": {
            "type": "array",
            "minItems": 1,
            "maxItems": 3,
            "items": {
                "type": "object",
                "properties": {
                    "rover": {"type": "integer"},
                    "kind": {"type": "string",
                             "enum": ["set_level", "go_to", "task", "handoff_bucket", "attention"]},
                    "action": {"type": "object", "description":
                               "set_level {level, budget_min?}; go_to {x, y} or {latitude, longitude}; "
                               "task {instruction, budget_min}; handoff_bucket {object_id, approach_m?}; attention {}"},
                    "urgency": {"type": "integer", "enum": [1, 2, 3]},
                    "reason": {"type": "string", "maxLength": 200},
                    "evidence": {"type": "array", "items": {"type": "string"}},
                    "confidence": {"type": "number"},
                },
                "required": ["rover", "kind", "action", "urgency", "reason"],
            },
        }
    },
    "required": ["recommendations"],
}
TOOLS = [
    {"name": "propose", "description": "Put up to 3 recommendation cards in front of the operator.",
     "input_schema": PROPOSE_SCHEMA},
    {"name": "read_rover", "description": "Raw cached telemetry for one rover, when the state is not enough.",
     "input_schema": {"type": "object", "properties": {"rover": {"type": "integer"}}, "required": ["rover"]}},
]


def run_tool(bridge, name, arguments):
    """Run one tool call against ARGOS. Returns (text, is_error)."""
    try:
        if name == "propose":
            return json.dumps(bridge.call("propose", recommendations=arguments.get("recommendations", []))), False
        if name == "read_rover":
            return json.dumps(bridge.call("rover", rover=arguments.get("rover"))), False
        return f"Unknown tool {name}", True
    except BridgeError as error:
        return str(error), True


class Claude:
    def __init__(self, model):
        import anthropic
        self.client = anthropic.Anthropic()
        self.model = model

    def tick(self, bridge, system, user):
        """One tick: a short manual tool loop. Returns a log record."""
        messages = [{"role": "user", "content": user}]
        record = {"calls": [], "usage": []}
        for _ in range(MAX_TURNS):
            response = self.client.beta.messages.create(
                model=self.model,
                max_tokens=16000,
                # The rules never change during a match, so they cache across ticks.
                system=[{"type": "text", "text": system, "cache_control": {"type": "ephemeral"}}],
                output_config={"effort": "medium"},
                betas=["server-side-fallback-2026-07-01"],
                fallbacks="default",
                tools=TOOLS,
                messages=messages,
            )
            record["usage"].append(response.usage.model_dump(exclude_none=True))
            record["stop_reason"] = response.stop_reason
            record["text"] = " ".join(b.text for b in response.content if b.type == "text").strip()
            if response.stop_reason == "refusal":
                record["refusal"] = response.stop_details.model_dump() if response.stop_details else None
                break
            uses = [b for b in response.content if b.type == "tool_use"]
            if response.stop_reason != "tool_use" or not uses:
                break
            messages.append({"role": "assistant", "content": response.content})
            results = []
            for use in uses:
                text, error = run_tool(bridge, use.name, use.input)
                record["calls"].append({"tool": use.name, "input": use.input, "result": text, "error": error})
                results.append({"type": "tool_result", "tool_use_id": use.id, "content": text, "is_error": error})
            messages.append({"role": "user", "content": results})
        return record


class Local:
    """A local model behind an OpenAI-style chat completions server (llama.cpp, vLLM, Ollama)."""

    def __init__(self, base_url, model):
        self.url = base_url.rstrip("/") + "/chat/completions"
        self.model = model
        self.tools = [{"type": "function", "function": {
            "name": tool["name"], "description": tool["description"], "parameters": tool["input_schema"]}}
            for tool in TOOLS]

    def tick(self, bridge, system, user):
        messages = [{"role": "system", "content": system}, {"role": "user", "content": user}]
        record = {"calls": [], "usage": []}
        for _ in range(MAX_TURNS):
            body = json.dumps({"model": self.model, "messages": messages, "tools": self.tools}).encode()
            request = urllib.request.Request(self.url, body, {"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=300) as reply:
                answer = json.load(reply)
            record["usage"].append(answer.get("usage"))
            message = answer["choices"][0]["message"]
            record["text"] = (message.get("content") or "").strip()
            calls = message.get("tool_calls") or []
            if not calls:
                break
            messages.append(message)
            for item in calls:
                try:
                    arguments = json.loads(item["function"].get("arguments") or "{}")
                except ValueError:
                    arguments, text, error = {}, "Arguments were not valid JSON", True
                else:
                    text, error = run_tool(bridge, item["function"]["name"], arguments)
                record["calls"].append({"tool": item["function"]["name"], "input": arguments,
                                        "result": text, "error": error})
                messages.append({"role": "tool", "tool_call_id": item.get("id"), "content": text})
        return record


class Dry:
    def tick(self, bridge, system, user):
        print(system, "\n\n", user)
        return {"calls": [], "text": "dry run"}


def changes(before, after):
    """Why the field changed enough to tick early, or an empty list."""
    if before is None:
        return []
    reasons = []

    def rovers(state):
        return {rover["id"]: rover for rover in state.get("rovers", [])}

    old, new = rovers(before), rovers(after)
    if set(old) != set(new):
        reasons.append("fleet changed")
    for rover_id, rover in new.items():
        was = old.get(rover_id)
        if not was:
            continue
        seen = {o["id"] for o in was.get("objects", [])}
        if any(o["id"] not in seen for o in rover.get("objects", [])):
            reasons.append(f"rover {rover_id} found something")
        if rover.get("hold") and rover.get("hold") != was.get("hold"):
            reasons.append(f"rover {rover_id} holds")
        if rover.get("stopped") and not was.get("stopped"):
            reasons.append(f"rover {rover_id} stopped")
        phase = (rover.get("exploration") or {}).get("phase")
        if phase == "ended" and (was.get("exploration") or {}).get("phase") != "ended":
            reasons.append(f"rover {rover_id} finished exploring")
        question = (rover.get("task") or {}).get("question")
        if question and question != (was.get("task") or {}).get("question"):
            reasons.append(f"rover {rover_id} asks a question")
        if (rover.get("goal") or {}).get("state") == "arrived" and (was.get("goal") or {}).get("state") != "arrived":
            reasons.append(f"rover {rover_id} arrived")

    def decided(state):
        return {(r["rec_id"], r["state"]) for r in state.get("recommendations", {}).get("recent", [])}

    if decided(after) - decided(before):
        reasons.append("operator decided a card")
    return reasons


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--backend", choices=("claude", "local", "dry"), default="claude")
    parser.add_argument("--model", help=f"Model id (default {CLAUDE_MODEL} for claude)")
    parser.add_argument("--base-url", default="http://localhost:8080/v1", help="Local server for --backend local")
    parser.add_argument("--every", type=float, default=60.0, help="Seconds between regular ticks")
    parser.add_argument("--once", action="store_true", help="Run one tick and exit")
    parser.add_argument("--bridge", help="Path to agent-bridge.json")
    args = parser.parse_args()
    if args.backend == "claude":
        engine = Claude(args.model or CLAUDE_MODEL)
    elif args.backend == "local":
        if not args.model:
            parser.error("--backend local needs --model")
        engine = Local(args.base_url, args.model)
    else:
        engine = Dry()
    name = f"loop:{args.backend}:{args.model or (CLAUDE_MODEL if args.backend == 'claude' else args.backend)}"
    exit_with_argos()
    bridge = Bridge(args.bridge, engine=name)
    system = system_prompt()
    log_path = session_dir() / f"argos-recommend-{time.strftime('%Y%m%d-%H%M%S')}.jsonl"
    last_tick, last_state, tick = 0.0, None, 0
    print(f"{name}: ticks every {args.every:.0f} s, log {log_path}")
    while True:
        try:
            state = bridge.call("state")
        except BridgeError as error:
            print(f"waiting for ARGOS: {error}")
            time.sleep(POLL_S)
            continue
        now = time.monotonic()
        early = changes(last_state, state)
        due = now - last_tick >= args.every or (early and now - last_tick >= EARLY_TICK_MIN_S)
        if state.get("mode") == "off" or not due:
            last_state = state if state.get("mode") != "off" else last_state
            time.sleep(POLL_S)
            continue
        tick += 1
        started = time.monotonic()
        try:
            record = engine.tick(bridge, system, tick_prompt(state))
        except Exception as error:  # a failed tick must not end the loop
            record = {"error": f"{type(error).__name__}: {error}"}
        record.update({"tick": tick, "utc": time.time(), "engine": name, "why": early or ["timer"],
                       "state_hash": state_digest(state), "latency_s": time.monotonic() - started,
                       "state": state})
        log_path.parent.mkdir(parents=True, exist_ok=True)
        with open(log_path, "a") as log:
            log.write(json.dumps(record, default=str) + "\n")
        proposed = [call for call in record.get("calls", []) if call["tool"] == "propose"]
        print(f"tick {tick} ({', '.join(record['why'])}): {len(proposed)} propose call(s) "
              f"in {record['latency_s']:.1f} s · {record.get('error') or record.get('text', '')[:120]}")
        last_tick, last_state = time.monotonic(), state
        if args.once:
            break
        time.sleep(POLL_S)
    bridge.close()


if __name__ == "__main__":
    main()
