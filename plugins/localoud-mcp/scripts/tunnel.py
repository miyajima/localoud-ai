#!/usr/bin/env python3
"""Run the Localoud standalone MCP server through a managed stdio tunnel."""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[3]
DEFAULT_ALIAS = "localoud-mcp-plugin"
RUNTIME_KEY_ENV = "CONTROL_PLANE_API_KEY"
KEYCHAIN_SERVICE = "dev.localoud.mcp-tunnel"
KEYCHAIN_ACCOUNT = "localoud-tunnel-runtime"
RUNTIME_KEY_REFERENCE = f"env:{RUNTIME_KEY_ENV}"
DEFAULT_HEALTH_PORT = 8794
MAX_DIAGNOSTIC_CHARS = 512
SAFE_ENVIRONMENT_NAMES = (
    "PATH",
    "HOME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_RUNTIME_DIR",
    "XDG_CACHE_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
)
STATUS_FIELDS = (
    "alias",
    "tunnel_id",
    "process_running",
    "healthy",
    "ready",
    "runtime_state",
    "stopped",
)
STATUS_BOOLEAN_FIELDS = {"process_running", "healthy", "ready", "stopped"}
ALIAS_PATTERN = re.compile(r"[a-z0-9]+(?:-[a-z0-9]+)*\Z")
TUNNEL_ID_PATTERN = re.compile(r"[A-Za-z0-9][A-Za-z0-9_-]{0,127}\Z")
BEARER_CREDENTIAL = re.compile(r"(?i)\bBearer\s+[^\s,;]+")
SENSITIVE_ASSIGNMENT = re.compile(
    r"(?i)\b(authorization|api[_ -]?key|access[_ -]?token|refresh[_ -]?token)"
    r"(\s*[:=]\s*)(?:Bearer\s+)?(\"[^\"]*\"|'[^']*'|[^\s,;]+)"
)


class LauncherError(Exception):
    """A safe, user-visible setup or runtime error."""


def _default_client() -> str:
    return str(Path.home() / "Library" / "Application Support" / "LocaloudTunnel" / "tunnel-client")


def _default_binary() -> str:
    return str(REPO_ROOT / "target" / "release" / "localoud-mcp")


def _default_profile_dir() -> str:
    return str(REPO_ROOT / "target" / "localoud-mcp-tunnel" / "profiles")


def _alias(value: str) -> str:
    if len(value) > 64 or not ALIAS_PATTERN.fullmatch(value):
        raise argparse.ArgumentTypeError(
            "alias must use lowercase letters, digits, and single hyphens"
        )
    if value == "localoud-readonly":
        raise argparse.ArgumentTypeError("localoud-readonly is reserved for the existing desktop profile")
    return value


def _tunnel_id(value: str) -> str:
    if not TUNNEL_ID_PATTERN.fullmatch(value):
        raise argparse.ArgumentTypeError("tunnel ID contains unsupported characters")
    return value


def _health_port(value: str) -> int:
    try:
        port = int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("health port must be an integer") from exc
    if not 1 <= port <= 65535:
        raise argparse.ArgumentTypeError("health port must be between 1 and 65535")
    return port


def _add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--client", default=_default_client(), help="tunnel-client executable")
    parser.add_argument("--alias", type=_alias, default=DEFAULT_ALIAS)


def _build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)

    start = commands.add_parser("start", help="start the managed stdio runtime")
    _add_common_arguments(start)
    start.add_argument("--binary", default=_default_binary(), help="localoud-mcp executable")
    start.add_argument("--config", required=True, help="existing Localoud MCP projects JSON")
    start.add_argument("--tunnel-id", required=True, type=_tunnel_id)
    start.add_argument("--profile-dir", default=_default_profile_dir())
    start.add_argument("--health-port", type=_health_port, default=DEFAULT_HEALTH_PORT)

    status = commands.add_parser("status", help="show sanitized status for the alias")
    _add_common_arguments(status)

    stop = commands.add_parser("stop", help="stop the managed runtime for the alias")
    _add_common_arguments(stop)

    doctor = commands.add_parser("doctor", help="diagnose the selected local tunnel profile")
    _add_common_arguments(doctor)
    doctor.add_argument("--profile-dir", default=_default_profile_dir())

    return parser


def _resolve_executable(raw_path: str, label: str) -> Path:
    try:
        path = Path(raw_path).expanduser().resolve(strict=True)
    except (OSError, RuntimeError):
        raise LauncherError(f"{label} executable was not found") from None
    if not path.is_file() or not os.access(path, os.X_OK):
        raise LauncherError(f"{label} must be an executable file")
    return path


def _resolve_profile_dir(raw_path: str, *, require_existing: bool) -> Path:
    target = (REPO_ROOT / "target").resolve()
    try:
        profile_dir = Path(raw_path).expanduser().resolve(strict=False)
        profile_dir.relative_to(target)
    except (OSError, RuntimeError, ValueError):
        raise LauncherError("profile directory must stay under this repository's target directory") from None
    if profile_dir.exists() and not profile_dir.is_dir():
        raise LauncherError("profile directory is not a directory")
    if require_existing and not profile_dir.is_dir():
        raise LauncherError("profile directory does not exist; start the runtime first")
    return profile_dir


def _validate_projects_config(raw_path: str) -> Path:
    try:
        config_path = Path(raw_path).expanduser().resolve(strict=True)
        if not config_path.is_file():
            raise LauncherError("config must be a JSON projects file")
        document = json.loads(config_path.read_text(encoding="utf-8"))
    except LauncherError:
        raise
    except (OSError, RuntimeError, UnicodeError, json.JSONDecodeError):
        raise LauncherError("config must be readable JSON containing a projects array") from None

    projects = document.get("projects") if isinstance(document, dict) else None
    if not isinstance(projects, list) or not projects:
        raise LauncherError("config must contain at least one project")
    for project in projects:
        if not isinstance(project, dict):
            raise LauncherError("config projects must be objects")
        if not isinstance(project.get("id"), str) or not project["id"]:
            raise LauncherError("each config project must have an id")
        root = project.get("root")
        if not isinstance(root, str) or not root or not Path(root).is_absolute():
            raise LauncherError("each config project must have an absolute root")
        for flag in ("write_enabled", "execution_enabled"):
            if flag in project and not isinstance(project[flag], bool):
                raise LauncherError(f"config project {flag} must be true or false")
    return config_path


def _safe_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        value = value.decode("utf-8", errors="replace")
    return value if isinstance(value, str) else str(value)


def _redact(value: Any, secret: str | None = None, *, limit: int = MAX_DIAGNOSTIC_CHARS) -> str:
    text = _safe_text(value)
    if secret:
        text = text.replace(secret, "[REDACTED]")
    text = SENSITIVE_ASSIGNMENT.sub(r"\1\2[REDACTED]", text)
    text = BEARER_CREDENTIAL.sub("Bearer [REDACTED]", text)
    text = "".join(character if character in "\n\t" or ord(character) >= 32 else " " for character in text)
    text = text.strip()
    if len(text) > limit:
        text = text[:limit].rstrip() + "…"
    return text


def _safe_environment(*, key: str | None = None, health_port: int | None = None) -> dict[str, str]:
    environment = {
        name: os.environ[name]
        for name in SAFE_ENVIRONMENT_NAMES
        if name in os.environ
    }
    if key is not None:
        environment[RUNTIME_KEY_ENV] = key
    if health_port is not None:
        environment["HEALTH_LISTEN_ADDR"] = f"127.0.0.1:{health_port}"
    return environment


def _runtime_key() -> str:
    existing = os.environ.get(RUNTIME_KEY_ENV)
    if existing:
        return existing

    try:
        result = subprocess.run(
            [
                "security",
                "find-generic-password",
                "-s",
                KEYCHAIN_SERVICE,
                "-a",
                KEYCHAIN_ACCOUNT,
                "-w",
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=15,
            env=_safe_environment(),
        )
    except (OSError, subprocess.TimeoutExpired):
        raise LauncherError("runtime API key could not be read from Keychain") from None
    if result.returncode != 0:
        raise LauncherError("runtime API key could not be read from Keychain")

    key = _safe_text(result.stdout).rstrip("\r\n")
    if not key or "\n" in key or "\r" in key or "\0" in key:
        raise LauncherError("runtime API key was empty or invalid")
    return key


def _invoke(client: Path, args: list[str], *, environment: dict[str, str], timeout: int) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(
            [str(client), *args],
            capture_output=True,
            text=True,
            check=False,
            timeout=timeout,
            env=environment,
        )
    except subprocess.TimeoutExpired as exc:
        detail = _redact(exc.stderr or exc.stdout, environment.get(RUNTIME_KEY_ENV))
        message = "tunnel-client command timed out"
        if detail:
            message += f": {detail}"
        raise LauncherError(message) from None
    except OSError as exc:
        detail = _redact(str(exc), environment.get(RUNTIME_KEY_ENV))
        message = "tunnel-client command could not be run"
        if detail:
            message += f": {detail}"
        raise LauncherError(message) from None


def _command_failure(label: str, result: subprocess.CompletedProcess[str], key: str | None) -> LauncherError:
    detail = _redact(result.stderr or result.stdout, key)
    message = f"{label} failed (exit {result.returncode})"
    if detail:
        message += f": {detail}"
    return LauncherError(message)


def _read_status(
    stdout: str,
    alias: str,
    *,
    key: str | None = None,
    tunnel_id: str | None = None,
    allow_empty: bool = False,
) -> dict[str, Any]:
    try:
        document = json.loads(stdout)
    except (TypeError, json.JSONDecodeError):
        raise LauncherError("tunnel-client returned invalid status JSON") from None
    if not isinstance(document, dict):
        raise LauncherError("tunnel-client returned an unexpected status response")

    fields: dict[str, Any] = {}
    has_state = False
    for name in STATUS_FIELDS:
        if name not in document:
            continue
        value = document[name]
        if name in STATUS_BOOLEAN_FIELDS:
            if isinstance(value, bool):
                fields[name] = value
                has_state = True
        elif isinstance(value, str):
            fields[name] = _redact(value, key, limit=128)
            if name != "alias" and value:
                has_state = True
    if not allow_empty and not has_state:
        raise LauncherError("tunnel-client returned invalid status JSON")
    fields.setdefault("alias", alias)
    if tunnel_id is not None:
        fields.setdefault("tunnel_id", _redact(tunnel_id, key, limit=128))
    return fields


def _print_status(fields: dict[str, Any]) -> None:
    print(json.dumps(fields, ensure_ascii=False, sort_keys=True))


def _run_status(client: Path, alias: str, *, key: str | None = None, tunnel_id: str | None = None) -> dict[str, Any]:
    environment = _safe_environment()
    result = _invoke(
        client,
        ["runtimes", "status", alias, "--json"],
        environment=environment,
        timeout=30,
    )
    if result.returncode != 0:
        raise _command_failure("runtime status", result, key)
    return _read_status(result.stdout, alias, key=key, tunnel_id=tunnel_id)


def _start(args: argparse.Namespace) -> int:
    if not args.tunnel_id:
        raise LauncherError("--tunnel-id is required for start")

    client = _resolve_executable(args.client, "tunnel-client")
    binary = _resolve_executable(args.binary, "localoud-mcp")
    config = _validate_projects_config(args.config)
    profile_dir = _resolve_profile_dir(args.profile_dir, require_existing=False)

    key = _runtime_key()
    environment = _safe_environment(key=key, health_port=args.health_port)
    mcp_command = shlex.join([str(binary), "--config", str(config)])
    connect_args = [
        "runtimes",
        "connect",
        "--alias",
        args.alias,
        "--profile",
        args.alias,
        "--profile-dir",
        str(profile_dir),
        "--runtime-api-key",
        RUNTIME_KEY_REFERENCE,
        "--tunnel-id",
        args.tunnel_id,
        "--mcp-command",
        mcp_command,
        "--json",
    ]
    result = _invoke(client, connect_args, environment=environment, timeout=120)
    if result.returncode != 0:
        raise _command_failure("runtime start", result, key)

    fields = _run_status(client, args.alias, key=key, tunnel_id=args.tunnel_id)
    _print_status(fields)
    if (
        fields.get("process_running") is not True
        or fields.get("healthy") is not True
        or fields.get("ready") is not True
    ):
        raise LauncherError("runtime started but did not report process_running, healthy, and ready")
    return 0


def _status(args: argparse.Namespace) -> int:
    client = _resolve_executable(args.client, "tunnel-client")
    fields = _run_status(client, args.alias)
    _print_status(fields)
    return 0


def _stop(args: argparse.Namespace) -> int:
    client = _resolve_executable(args.client, "tunnel-client")
    result = _invoke(
        client,
        ["runtimes", "stop", args.alias, "--json"],
        environment=_safe_environment(),
        timeout=30,
    )
    if result.returncode != 0:
        raise _command_failure("runtime stop", result, None)

    fields = _run_status(client, args.alias)
    state = fields.get("runtime_state")
    contradictory = (
        fields.get("stopped") is False
        or fields.get("process_running") is True
        or fields.get("healthy") is True
        or fields.get("ready") is True
        or (isinstance(state, str) and state.lower() in {"running", "starting", "stopping"})
    )
    confirmed_stopped = fields.get("stopped") is True or fields.get("runtime_state") == "stopped"
    confirmed_not_running = fields.get("process_running") is False
    if contradictory or not (confirmed_stopped or confirmed_not_running):
        raise LauncherError("runtime stop completed but status did not confirm stopped")
    fields.setdefault("stopped", True)
    _print_status(fields)
    return 0


def _doctor(args: argparse.Namespace) -> int:
    client = _resolve_executable(args.client, "tunnel-client")
    profile_dir = _resolve_profile_dir(args.profile_dir, require_existing=True)
    key = _runtime_key()
    environment = _safe_environment(key=key)
    result = _invoke(
        client,
        [
            "doctor",
            "--profile-dir",
            str(profile_dir),
            "--profile",
            args.alias,
            "--health.listen-addr",
            "127.0.0.1:0",
            "--json",
        ],
        environment=environment,
        timeout=45,
    )
    if result.returncode != 0:
        raise _command_failure("doctor", result, key)
    print("doctor: ok")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = _build_parser()
    try:
        args = parser.parse_args(argv)
    except SystemExit as exc:
        return int(exc.code)

    try:
        if args.action == "start":
            return _start(args)
        if args.action == "status":
            return _status(args)
        if args.action == "stop":
            return _stop(args)
        if args.action == "doctor":
            return _doctor(args)
        raise LauncherError("unknown action")
    except LauncherError as exc:
        print(f"error: {_redact(str(exc))}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
