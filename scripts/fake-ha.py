# /// script
# requires-python = ">=3.12"
# dependencies = ["websockets"]
# ///
"""Minimal Home Assistant stand-in for exercising the Screensight device.

Two modes:

  Pair (device shows a code and its pairing window is open):

      uv run scripts/fake-ha.py pair --code 123456

    After the device shows "request received", tap "Yes · pair this display"
    on the panel; the script prints the token.

  Push text (already paired):

      uv run scripts/fake-ha.py text --token <token> "it is going to rain"

This is a developer aid, not the integration.
"""

import argparse
import asyncio
import json
import uuid

import websockets


def ws_url(host: str, port: int) -> str:
    return f"ws://{host}:{port}/ws"


async def pair(host: str, port: int, code: str, device_id: str | None) -> None:
    ha_id = str(uuid.uuid4())
    async with websockets.connect(ws_url(host, port)) as ws:
        await ws.send(
            json.dumps(
                {
                    "type": "pair",
                    "device_id": device_id or "",
                    "code": code,
                    "ha_id": ha_id,
                    "ha_name": "Fake Home Assistant",
                }
            )
        )
        async for raw in ws:
            message = json.loads(raw)
            kind = message.get("type")
            print(f"<- {kind}: {message}")
            if kind == "pair_pending":
                print(">>> tap 'Yes · pair this display' on the panel now <<<")
            elif kind == "pair_success":
                print(f"token = {message['token']}")
                return
            elif kind in {"pair_error", "pair_rejected"}:
                return


async def text(host: str, port: int, token: str, value: str) -> None:
    async with websockets.connect(
        ws_url(host, port), additional_headers={"Authorization": f"Bearer {token}"}
    ) as ws:
        # The device pushes its current state as soon as we connect.
        print(f"<- {await ws.recv()}")
        await ws.send(json.dumps({"type": "set_text", "text": value}))
        await ws.send(json.dumps({"type": "get_state"}))
        # Read replies until we see the state carrying our new text (the device
        # also answers the get_state we sent, so there may be two frames).
        while True:
            message = json.loads(await ws.recv())
            kind = message.get("type")
            if kind == "state":
                print(f"<- {message}")
                if message.get("text") == value:
                    return


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    sub = parser.add_subparsers(dest="command", required=True)

    pair_cmd = sub.add_parser("pair")
    pair_cmd.add_argument("--code", required=True)
    pair_cmd.add_argument("--device-id", default=None)

    text_cmd = sub.add_parser("text")
    text_cmd.add_argument("--token", required=True)
    text_cmd.add_argument("value")

    args = parser.parse_args()
    if args.command == "pair":
        asyncio.run(pair(args.host, args.port, args.code, args.device_id))
    else:
        asyncio.run(text(args.host, args.port, args.token, args.value))


if __name__ == "__main__":
    main()
