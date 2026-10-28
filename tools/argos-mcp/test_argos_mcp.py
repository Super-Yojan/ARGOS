"""Tests for the ARGOS MCP server and recommendation loop against a fake window.

    python3 -m unittest discover tools/argos-mcp
"""
import asyncio
import json
import pathlib
import socket
import tempfile
import threading
import unittest

import argos_mcp
import recommend_loop
from argos_bridge import Bridge, BridgeError, state_digest, system_prompt, tick_prompt

STATE = {
    "time_s": 12.0,
    "mode": "advise",
    "rovers": [{"id": 1, "level": "waypoint", "objects": [], "goal": None}],
    "recommendations": {"open": [], "recent": []},
}


class FakeWindow:
    """Answers bridge requests the way the ARGOS window does, and records them."""

    def __init__(self, folder, token="secret"):
        self.token = token
        self.requests = []
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen()
        self.path = pathlib.Path(folder) / "agent-bridge.json"
        self.path.write_text(json.dumps({"port": self.listener.getsockname()[1], "token": token}))
        threading.Thread(target=self.accept, daemon=True).start()

    def accept(self):
        while True:
            try:
                connection, _ = self.listener.accept()
            except OSError:
                return
            threading.Thread(target=self.serve, args=(connection,), daemon=True).start()

    def serve(self, connection):
        with connection, connection.makefile("rwb") as stream:
            for line in stream:
                request = json.loads(line)
                self.requests.append(request)
                stream.write(json.dumps(self.answer(request)).encode() + b"\n")
                stream.flush()

    def answer(self, request):
        reply = {"id": request["id"], "ok": True}
        if request["token"] != self.token:
            return {"id": request["id"], "ok": False, "error": "Wrong or missing token"}
        op = request["op"]
        if op == "hello":
            reply.update(protocol=1, mode="advise")
        elif op == "state":
            reply.update(STATE)
        elif op == "propose":
            reply["results"] = [{"ok": True, "rec_id": f"r{i}"} for i, _ in enumerate(request["recommendations"])]
        elif op == "set_level":
            return {"id": request["id"], "ok": False, "error": "Agent commands are off: the window is in Advise"}
        return reply

    def ops(self):
        return [request["op"] for request in self.requests]

    def close(self):
        self.listener.close()


class BridgeTest(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.window = FakeWindow(self.folder.name)
        self.bridge = Bridge(self.window.path, engine="test")

    def tearDown(self):
        self.bridge.close()
        self.window.close()
        self.folder.cleanup()

    def test_says_hello_then_drops_empty_fields(self):
        self.assertEqual(self.bridge.call("state")["rovers"][0]["id"], 1)
        self.assertEqual(self.window.ops(), ["hello", "state"])
        self.assertEqual(self.window.requests[0]["engine"], "test")
        self.bridge.call("go_to", rover=1, x=2.0, y=None)
        self.assertNotIn("y", self.window.requests[-1])

    def test_refusals_carry_the_reason(self):
        with self.assertRaisesRegex(BridgeError, "Advise"):
            self.bridge.call("set_level", rover=1, level="teleop")

    def test_missing_window(self):
        with self.assertRaisesRegex(BridgeError, "not running"):
            Bridge(pathlib.Path(self.folder.name) / "nope.json").call("state")

    def test_reconnects_with_a_new_token(self):
        self.bridge.call("state")
        self.bridge._sock.close()
        self.assertEqual(self.bridge.call("state")["mode"], "advise")

    def test_prompt(self):
        prompt = system_prompt()
        for needle in ("NEXT", "assisted_teleop", "target_search", "propose", "get_field_state"):
            self.assertIn(needle, prompt)
        self.assertIn('"rovers"', tick_prompt(STATE))
        self.assertEqual(state_digest(STATE), state_digest({**STATE, "time_s": 99.0}))


class ServerTest(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.window = FakeWindow(self.folder.name)
        self.bridge = Bridge(self.window.path, engine="test")

    def tearDown(self):
        self.bridge.close()
        self.window.close()
        self.folder.cleanup()

    def tools(self, profile):
        server = argos_mcp.build_server(self.bridge, profile)
        return server, {tool.name for tool in asyncio.run(server.list_tools())}

    def test_profiles(self):
        _, advise = self.tools("advise")
        self.assertEqual(advise, {"get_field_state", "read_rover", "list_recommendations", "propose", "withdraw"})
        _, operator = self.tools("operator")
        self.assertTrue({"set_level", "go_to", "start_task", "stop_all", "request_drive", "drive"} <= operator)
        self.assertNotIn("dump", operator)
        with self.assertRaises(ValueError):
            argos_mcp.build_server(self.bridge, "god")

    def test_propose_round_trip(self):
        server, _ = self.tools("advise")
        card = {"rover": 1, "kind": "attention", "action": {}, "urgency": 2, "reason": "stuck 40 s"}
        result = asyncio.run(server.call_tool("propose", {"recommendations": [card]}))
        self.assertIn("r0", json.dumps(result, default=str))
        self.assertEqual(self.window.requests[-1]["recommendations"], [card])

    def test_prompt_and_resources(self):
        server, _ = self.tools("advise")
        prompt = asyncio.run(server.get_prompt("recommend_tick"))
        text = json.dumps(prompt.model_dump(), default=str)
        self.assertIn("NEXT", text)
        self.assertIn("waypoint", text)
        resources = {str(r.uri) for r in asyncio.run(server.list_resources())}
        self.assertEqual(resources, {"argos://rules", "argos://topics"})


class LoopTest(unittest.TestCase):
    def test_changes(self):
        rover = {"id": 1, "objects": [], "hold": None, "stopped": False,
                 "exploration": {"phase": "running"}, "task": None, "goal": {"state": "driving"}}
        before = {"rovers": [rover], "recommendations": {"recent": []}}
        self.assertEqual(recommend_loop.changes(None, before), [])
        self.assertEqual(recommend_loop.changes(before, before), [])
        after = {"rovers": [{**rover, "objects": [{"id": 7}], "exploration": {"phase": "ended"},
                             "task": {"question": "Which bucket?"}, "goal": {"state": "arrived"}}],
                 "recommendations": {"recent": [{"rec_id": "r1", "state": "rejected"}]}}
        reasons = recommend_loop.changes(before, after)
        for reason in ("found something", "finished exploring", "asks a question", "arrived", "decided"):
            self.assertTrue(any(reason in r for r in reasons), reason)
        self.assertEqual(recommend_loop.changes(before, {"rovers": [], "recommendations": {}}), ["fleet changed"])

    def test_tools_run_against_the_window(self):
        folder = tempfile.TemporaryDirectory()
        window = FakeWindow(folder.name)
        bridge = Bridge(window.path)
        try:
            text, error = recommend_loop.run_tool(bridge, "propose", {"recommendations": [{"rover": 1}]})
            self.assertFalse(error)
            self.assertIn("r0", text)
            text, error = recommend_loop.run_tool(bridge, "set_level", {})
            self.assertTrue(error)
            self.assertEqual(window.ops()[-1], "propose")
        finally:
            bridge.close()
            window.close()
            folder.cleanup()

    def test_local_backend_calls_tools(self):
        """The OpenAI-style backend runs a tool call, then stops on a plain answer."""
        folder = tempfile.TemporaryDirectory()
        window = FakeWindow(folder.name)
        bridge = Bridge(window.path)
        replies = iter([
            {"choices": [{"message": {"content": "", "tool_calls": [{"id": "c1", "function": {
                "name": "propose", "arguments": json.dumps({"recommendations": [{"rover": 1}]})}}]}}]},
            {"choices": [{"message": {"content": "done"}}]},
        ])

        class Reply:
            def __init__(self, body):
                self.body = json.dumps(body).encode()

            def read(self, *_):
                return self.body

            def __enter__(self):
                return self

            def __exit__(self, *_):
                return False

        original = recommend_loop.urllib.request.urlopen
        recommend_loop.urllib.request.urlopen = lambda *a, **k: Reply(next(replies))
        try:
            record = recommend_loop.Local("http://x/v1", "qwen").tick(bridge, "sys", "user")
        finally:
            recommend_loop.urllib.request.urlopen = original
            bridge.close()
            window.close()
            folder.cleanup()
        self.assertEqual(record["text"], "done")
        self.assertEqual([c["tool"] for c in record["calls"]], ["propose"])


if __name__ == "__main__":
    unittest.main()
