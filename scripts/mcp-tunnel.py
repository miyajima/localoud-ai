#!/usr/bin/env python3
"""Start or inspect the configured Localoud tunnel; secrets stay in Keychain."""
import argparse
import json
import os
from pathlib import Path
import subprocess

BASE = Path.home() / "Library/Application Support/LocaloudTunnel"
CLIENT = BASE / "tunnel-client"
ALIAS = "localoud-readonly"
TUNNEL = "tunnel_6aa00b3dab50819198663f6f3c0e85ff"


def credential(service, account):
    result = subprocess.run(
        ["security", "find-generic-password", "-s", service, "-a", account, "-w"],
        capture_output=True, text=True, timeout=30, check=True,
    )
    return result.stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["start", "status", "stop"])
    args = parser.parse_args()
    env = os.environ.copy()
    command = [str(CLIENT), "runtimes"]
    secrets = []
    if args.action == "start":
        control = credential("dev.localoud.mcp-tunnel", "localoud-tunnel-runtime")
        local = "Bearer " + credential("dev.locloud.read-mcp", "bearer")
        secrets = [control, local]
        env.update({
            "CONTROL_PLANE_API_KEY": control,
            "LOCALOUD_MCP_AUTH": local,
            "MCP_EXTRA_HEADERS": "Authorization: env:LOCALOUD_MCP_AUTH",
            "MCP_DISCOVERY_EXTRA_HEADERS": "Authorization: env:LOCALOUD_MCP_AUTH",
            "HEALTH_LISTEN_ADDR": "127.0.0.1:8793",
        })
        command += [
            "connect", "--alias", ALIAS, "--tunnel-id", TUNNEL,
            "--profile-dir", str(BASE / "profiles"), "--profile", ALIAS,
            "--runtime-api-key", "env:CONTROL_PLANE_API_KEY",
            "--mcp-server-url", "http://127.0.0.1:8792/mcp", "--json",
        ]
    else:
        command += [args.action, ALIAS, "--json"]
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=90)
    if result.returncode:
        error = result.stderr
        for secret in secrets:
            error = error.replace(secret, "[REDACTED]")
        raise SystemExit(error or "Tunnel command failed")
    data = json.loads(result.stdout)
    fields = ["alias", "process_running", "healthy", "ready", "runtime_state", "stopped"]
    print(json.dumps({key: data[key] for key in fields if key in data}, indent=2))


if __name__ == "__main__":
    main()
