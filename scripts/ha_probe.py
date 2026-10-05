# /// script
# requires-python = ">=3.12"
# dependencies = ["websockets"]
# ///
"""Probe the throwaway Home Assistant for in-progress config flows.

Usage: uv run --no-project scripts/ha_probe.py [base-url]
"""

from __future__ import annotations

import asyncio
import json
import sys
import urllib.parse
import urllib.request

import websockets

BASE = (sys.argv[1] if len(sys.argv) > 1 else "http://localhost:8123").rstrip("/")
CLIENT = f"{BASE}/"
WS_URL = BASE.replace("http", "ws", 1) + "/api/websocket"


def call(method: str, path: str, *, body=None, form=None, token=None):
    headers = {"Accept": "application/json"}
    data = None
    if form is not None:
        data = urllib.parse.urlencode(form).encode()
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    elif body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(BASE + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as err:
        raw = err.read().decode()
        return err.code, raw


def login() -> str:
    status, payload = call(
        "POST",
        "/auth/login_flow",
        body={
            "client_id": CLIENT,
            "handler": ["homeassistant", None],
            "redirect_uri": CLIENT,
        },
    )
    print("login_flow start:", status, payload)
    flow_id = payload["flow_id"]
    status, payload = call(
        "POST",
        f"/auth/login_flow/{flow_id}",
        body={"client_id": CLIENT, "username": "dev", "password": "dev"},
    )
    print("login_flow submit:", status, {k: payload.get(k) for k in ("type", "errors")})
    code = payload.get("result")
    status, payload = call(
        "POST",
        "/auth/token",
        form={"grant_type": "authorization_code", "code": code, "client_id": CLIENT},
    )
    print("token:", status)
    return payload["access_token"]


def main() -> None:
    token = login()
    asyncio.run(dump_flows(token))


async def dump_flows(token: str) -> None:
    """Subscribe to config flows and print the active ones."""
    async with websockets.connect(WS_URL) as ws:
        await ws.recv()  # auth_required
        await ws.send(json.dumps({"type": "auth", "access_token": token}))
        auth = json.loads(await ws.recv())
        if auth.get("type") != "auth_ok":
            print("auth failed:", auth)
            return
        await ws.send(json.dumps({"id": 2, "type": "config_entries/flow/subscribe"}))
        # The subscribe reply carries the current snapshot, then updates.
        while True:
            raw = await ws.recv()
            message = json.loads(raw)
            flows = message.get("result")
            if flows is None:
                event = message.get("event")
                if isinstance(event, list):
                    flows = [entry.get("flow") for entry in event if entry.get("flow")]
            if isinstance(flows, list):
                screensight = [f for f in flows if f.get("handler") == "screensight"]
                print(f"total flows: {len(flows)}; screensight: {len(screensight)}")
                for flow in screensight:
                    print(
                        json.dumps(
                            {
                                "flow_id": flow.get("flow_id"),
                                "step_id": flow.get("step_id"),
                                "source": (flow.get("context") or {}).get("source"),
                                "unique_id": flow.get("unique_id"),
                                "type": flow.get("type"),
                                "errors": flow.get("errors"),
                            }
                        )
                    )
                if message.get("id") == 2:
                    return


if __name__ == "__main__":
    main()
