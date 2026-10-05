#!/usr/bin/env python3
# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Automatically complete Home Assistant onboarding for the throwaway dev instance.

Waits for Home Assistant to come up, then drives the supported onboarding API so
there is no setup wizard: it creates an owner user (default `dev` / `dev`),
finishes core config, analytics and integration setup, and leaves the instance
ready to use.

Run via `scripts/ha-dev.sh up`; you normally never call this directly.
"""

from __future__ import annotations

import json
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

DEFAULT_URL = "http://localhost:8123"
DEFAULT_USER = "dev"
DEFAULT_PASSWORD = "dev"
READY_TIMEOUT = 300  # seconds (first boot on a new version can be slow)


def request(
    method: str,
    url: str,
    *,
    body: dict | None = None,
    form: dict | None = None,
    token: str | None = None,
    timeout: float = 10.0,
) -> tuple[int, dict | str]:
    """Perform one HTTP request and return ``(status, parsed_body)``."""
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

    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode()
            return resp.status, (json.loads(raw) if raw else {})
    except urllib.error.HTTPError as err:
        raw = err.read().decode()
        try:
            return err.code, (json.loads(raw) if raw else {})
        except json.JSONDecodeError:
            return err.code, raw
    except urllib.error.URLError as err:
        return 0, str(err.reason)


def wait_ready(base: str) -> list | None:
    """Poll the onboarding status until Home Assistant answers."""
    deadline = time.time() + READY_TIMEOUT
    while time.time() < deadline:
        status, payload = request("GET", f"{base}/api/onboarding", timeout=5.0)
        # The endpoint returns a list of {"step": str, "done": bool} entries.
        if status == 200 and isinstance(payload, list):
            return payload
        time.sleep(2)
    return None


def done_steps(status: list) -> set[str]:
    """Extract the set of completed onboarding steps."""
    return {entry["step"] for entry in status if entry.get("done")}


def provision(base: str, username: str, password: str) -> int:
    """Complete onboarding; return a process exit code."""
    base = base.rstrip("/")
    print(f"waiting for Home Assistant at {base} ...")
    status = wait_ready(base)
    if status is None:
        print("error: Home Assistant did not become ready in time", file=sys.stderr)
        return 1

    done = done_steps(status)
    if "user" in done:
        print("onboarding already complete; nothing to do")
        return 0

    client_id = f"{base}/"
    redirect_uri = f"{base}/?auth_callback=1"

    # 1. Owner user. Returns a one-time auth code.
    code, payload = request(
        "POST",
        f"{base}/api/onboarding/users",
        body={
            "client_id": client_id,
            "name": "Screensight Dev",
            "username": username,
            "password": password,
            "language": "en",
        },
    )
    if code != 200 or not isinstance(payload, dict) or "auth_code" not in payload:
        print(f"error: creating user failed ({code}): {payload}", file=sys.stderr)
        return 1
    auth_code = payload["auth_code"]

    # 2. Exchange the auth code for an access token.
    code, payload = request(
        "POST",
        f"{base}/auth/token",
        form={
            "grant_type": "authorization_code",
            "code": auth_code,
            "client_id": client_id,
        },
    )
    if code != 200 or not isinstance(payload, dict) or "access_token" not in payload:
        print(f"error: token exchange failed ({code}): {payload}", file=sys.stderr)
        return 1
    token = payload["access_token"]

    # 3. Remaining steps (idempotent; ignore "already done" responses).
    steps = [
        ("core_config", {}),
        ("analytics", {}),
        (
            "integration",
            {"client_id": client_id, "redirect_uri": redirect_uri},
        ),
    ]
    for step, body in steps:
        code, payload = request(
            "POST", f"{base}/api/onboarding/{step}", body=body, token=token
        )
        if code not in (200, 201, 403, 409):
            print(f"warning: onboarding step {step} returned {code}: {payload}")

    print(f"onboarding complete — log in at {base} as {username} / {password}")
    return 0


def main() -> int:
    base = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_URL
    username = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_USER
    password = sys.argv[3] if len(sys.argv) > 3 else DEFAULT_PASSWORD
    return provision(base, username, password)


if __name__ == "__main__":
    raise SystemExit(main())
