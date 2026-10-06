# /// script
# requires-python = ">=3.12"
# dependencies = ["websockets", "noiseprotocol", "cryptography"]
# ///
"""Minimal Home Assistant stand-in for exercising the Screensight device.

Pairing runs Noise XX over the device's ``/ws`` endpoint, exactly as Home
Assistant does. The device shows an 8-digit SAS on its panel once the handshake
completes; type it here to finish pairing. No token is issued: the script stores
Home Assistant's static key and the device's static key so ``text`` can reconnect
with Noise IK.

The keys are written to ``--keys`` (default ``/tmp/screensight-fake-ha.json``)
and reused by the ``text`` command.

  Pair:

      uv run scripts/fake-ha.py pair

  Push text (already paired):

      uv run scripts/fake-ha.py text "it is going to rain"

This is a developer aid, not the integration.
"""

from __future__ import annotations

import argparse
import asyncio
import hashlib
import json
import uuid
from pathlib import Path

import websockets
from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric import x25519
from noise.connection import Keypair, NoiseConnection

XX_PATTERN = b"Noise_XX_25519_ChaChaPoly_SHA256"
IK_PATTERN = b"Noise_IK_25519_ChaChaPoly_SHA256"
SAS_DOMAIN = b"screensight/pairing-sas/v1"
SAS_MODULUS = 100_000_000
DEFAULT_KEYS = str(Path.home() / ".screensight-fake-ha.json")


def ws_url(host: str, port: int) -> str:
    """Return the device WebSocket URL."""
    return f"ws://{host}:{port}/ws"


def generate_keypair() -> tuple[bytes, bytes]:
    """Generate an X25519 keypair as raw ``(private, public)`` bytes."""
    private = x25519.X25519PrivateKey.generate()
    private_bytes = private.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    public_bytes = private.public_key().public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    return private_bytes, public_bytes


def sas_from_handshake_hash(handshake_hash: bytes) -> str:
    """Derive the 8-digit SAS exactly as the device does."""
    digest = hashlib.sha256(SAS_DOMAIN + handshake_hash).digest()
    limit = (0xFFFFFFFF // SAS_MODULUS) * SAS_MODULUS
    for offset in range(0, len(digest), 4):
        value = int.from_bytes(digest[offset : offset + 4], "big")
        if value < limit:
            return f"{value % SAS_MODULUS:08d}"
    return f"{int.from_bytes(digest[:4], 'big') % SAS_MODULUS:08d}"


async def receive_binary(ws: websockets.ClientConnection) -> bytes:
    """Await the next binary frame."""
    async for raw in ws:
        if isinstance(raw, (bytes, bytearray)):
            return bytes(raw)
        msg = "expected a binary frame from the device"
        raise RuntimeError(msg)
    msg = "device closed the connection"
    raise RuntimeError(msg)


async def pair(host: str, port: int, keys_path: Path) -> None:
    """Run Noise XX, collect the SAS, and persist the pairing keys."""
    ha_id = str(uuid.uuid4())
    ha_private, ha_public = generate_keypair()
    connection = NoiseConnection.from_name(XX_PATTERN)
    connection.set_as_initiator()
    connection.set_keypair_from_private_bytes(Keypair.STATIC, ha_private)
    connection.start_handshake()

    async with websockets.connect(
        ws_url(host, port), subprotocols=["screensight.noise.xx"]
    ) as ws:
        await ws.send(bytes(connection.write_message()))
        connection.read_message(await receive_binary(ws))
        device_static = bytes(connection.noise_protocol.handshake_state.rs.public_bytes)
        await ws.send(bytes(connection.write_message()))

        sas = sas_from_handshake_hash(connection.get_handshake_hash())
        typed = await asyncio.to_thread(input, "Code on the panel: ")
        if typed.strip() != sas:
            print(f"SAS mismatch (expected {sas}); aborting")
            return

        frame = {"type": "pair", "ha_id": ha_id, "ha_name": "Fake Home Assistant"}
        await ws.send(connection.encrypt(json.dumps(frame).encode()))
        while True:
            message = json.loads(connection.decrypt(await receive_binary(ws)))
            kind = message.get("type")
            print(f"<- {kind}: {message}")
            if kind == "pair_pending":
                print(">>> tap 'Pair' on the panel now <<<")
            elif kind == "pair_success":
                break
            elif kind in {"pair_error", "pair_rejected"}:
                return

    await asyncio.to_thread(
        keys_path.write_text,
        json.dumps(
            {
                "ha_id": ha_id,
                "ha_private": ha_private.hex(),
                "ha_public": ha_public.hex(),
                "device_static": device_static.hex(),
            }
        ),
    )
    print(f"paired; keys written to {keys_path}")


async def text(host: str, port: int, keys_path: Path, value: str) -> None:
    """Reconnect with Noise IK and push a display value."""
    keys = json.loads(await asyncio.to_thread(keys_path.read_text))
    connection = NoiseConnection.from_name(IK_PATTERN)
    connection.set_as_initiator()
    connection.set_keypair_from_private_bytes(
        Keypair.STATIC, bytes.fromhex(keys["ha_private"])
    )
    connection.set_keypair_from_public_bytes(
        Keypair.REMOTE_STATIC, bytes.fromhex(keys["device_static"])
    )
    connection.start_handshake()

    async with websockets.connect(
        ws_url(host, port), subprotocols=["screensight.noise.ik"]
    ) as ws:
        await ws.send(bytes(connection.write_message()))
        connection.read_message(await receive_binary(ws))
        print(f"<- {json.loads(connection.decrypt(await receive_binary(ws)))}")
        frame = {"type": "set_value", "key": "text", "value": value}
        await ws.send(connection.encrypt(json.dumps(frame).encode()))
        while True:
            message = json.loads(connection.decrypt(await receive_binary(ws)))
            if message.get("type") == "state":
                print(f"<- {message}")
                if message.get("values", {}).get("text") == value:
                    return


def main() -> None:
    """Parse arguments and run the requested mode."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--keys", default=DEFAULT_KEYS)
    sub = parser.add_subparsers(dest="command", required=True)

    sub.add_parser("pair")

    text_cmd = sub.add_parser("text")
    text_cmd.add_argument("value")

    args = parser.parse_args()
    keys_path = Path(args.keys)
    if args.command == "pair":
        asyncio.run(pair(args.host, args.port, keys_path))
    else:
        asyncio.run(text(args.host, args.port, keys_path, args.value))


if __name__ == "__main__":
    main()
