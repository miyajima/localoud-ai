import contextlib
import importlib.util
import io
import json
import os
import shlex
import tempfile
import unittest
from pathlib import Path
from subprocess import CompletedProcess, TimeoutExpired
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "tunnel.py"
SPEC = importlib.util.spec_from_file_location("localoud_tunnel", SCRIPT)
assert SPEC and SPEC.loader
tunnel = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(tunnel)


class TunnelLauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="localoud tunnel tests ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "checkout with spaces"
        self.root.mkdir()
        (self.root / "target" / "release").mkdir(parents=True)
        self.binary = self.root / "target" / "release" / "localoud-mcp"
        self.binary.write_text("fake executable", encoding="utf-8")
        self.binary.chmod(0o755)
        self.client = Path(self.temp.name) / "Tunnel Client" / "tunnel-client"
        self.client.parent.mkdir()
        self.client.write_text("fake executable", encoding="utf-8")
        self.client.chmod(0o755)
        self.config = Path(self.temp.name) / "test projects" / "mcp-projects.json"
        self.config.parent.mkdir()
        self.config.write_text(
            json.dumps(
                {
                    "projects": [
                        {
                            "id": "test-project",
                            "name": "test-project",
                            "root": str(self.root),
                            "write_enabled": True,
                            "execution_enabled": True,
                        }
                    ]
                }
            ),
            encoding="utf-8",
        )
        self.profile_dir = self.root / "target" / "localoud-mcp-tunnel" / "profiles"
        self.profile_dir.mkdir(parents=True)

    def call(self, *args):
        stdout = io.StringIO()
        stderr = io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            try:
                code = tunnel.main(list(args))
            except SystemExit as exc:
                code = int(exc.code)
        return code, stdout.getvalue(), stderr.getvalue()

    @staticmethod
    def result(args, *, code=0, stdout="", stderr=""):
        return CompletedProcess(args, code, stdout, stderr)

    def test_start_uses_stdio_command_and_runtime_key_reference_without_widening_scope(self):
        key = "runtime-secret-value"
        connect = self.result([], stdout='{"tunnel_id":"tunnel_test"}', stderr=f"debug {key}")
        status = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "tunnel_id": "tunnel_test",
                    "process_running": True,
                    "healthy": True,
                    "ready": True,
                    "runtime_state": "running",
                    "private_token": key,
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(
                os.environ,
                {
                    "CONTROL_PLANE_API_KEY": key,
                    "OPENAI_ADMIN_KEY": "fixture-admin-key",
                    "UNRELATED_SECRET": "fixture-unrelated-secret",
                    "PATH": "/usr/bin:/bin",
                    "HOME": "/tmp/fake home",
                    "HTTPS_PROXY": "http://proxy.invalid:8080",
                    "SSL_CERT_FILE": "/tmp/fake-ca.pem",
                },
                clear=True,
            ),
            patch.object(tunnel.subprocess, "run", side_effect=[connect, status]) as run,
        ):
            code, out, err = self.call(
                "start",
                "--client",
                str(self.client),
                "--config",
                str(self.config),
                "--tunnel-id",
                "tunnel_test",
            )

        self.assertEqual(code, 0, err)
        self.assertEqual(err, "")
        self.assertNotIn(key, out)
        self.assertNotIn("private_token", out)
        self.assertIn('"ready": true', out)
        connect_argv = run.call_args_list[0].args[0]
        self.assertEqual(connect_argv[0], str(self.client.resolve()))
        self.assertIn("--runtime-api-key", connect_argv)
        self.assertEqual(
            connect_argv[connect_argv.index("--runtime-api-key") + 1],
            "env:CONTROL_PLANE_API_KEY",
        )
        command = connect_argv[connect_argv.index("--mcp-command") + 1]
        self.assertEqual(
            command,
            shlex.join([str(self.binary.resolve()), "--config", str(self.config.resolve())]),
        )
        self.assertIn("--profile-dir", connect_argv)
        self.assertEqual(
            connect_argv[connect_argv.index("--profile-dir") + 1],
            str(self.profile_dir.resolve()),
        )
        self.assertEqual(
            connect_argv[connect_argv.index("--profile") + 1], "localoud-mcp-plugin"
        )
        self.assertEqual(
            run.call_args_list[0].kwargs["env"]["CONTROL_PLANE_API_KEY"], key
        )
        self.assertEqual(
            run.call_args_list[0].kwargs["env"]["HEALTH_LISTEN_ADDR"],
            "127.0.0.1:8794",
        )
        self.assertEqual(run.call_args_list[0].kwargs["env"]["PATH"], "/usr/bin:/bin")
        self.assertEqual(run.call_args_list[0].kwargs["env"]["HOME"], "/tmp/fake home")
        self.assertEqual(
            run.call_args_list[0].kwargs["env"]["HTTPS_PROXY"],
            "http://proxy.invalid:8080",
        )
        self.assertEqual(
            run.call_args_list[0].kwargs["env"]["SSL_CERT_FILE"], "/tmp/fake-ca.pem"
        )
        self.assertNotIn("OPENAI_ADMIN_KEY", run.call_args_list[0].kwargs["env"])
        self.assertNotIn("UNRELATED_SECRET", run.call_args_list[0].kwargs["env"])
        self.assertNotIn("--allow-write", command)
        self.assertNotIn("--allow-exec", command)
        self.assertNotIn(key, " ".join(connect_argv))
        status_argv = run.call_args_list[1].args[0]
        self.assertEqual(status_argv[1:4], ["runtimes", "status", "localoud-mcp-plugin"])
        self.assertNotIn("CONTROL_PLANE_API_KEY", run.call_args_list[1].kwargs["env"])

    def test_start_uses_keychain_fallback_privately(self):
        key = "keychain-runtime-secret"
        connect = self.result([], stdout="{}")
        status = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "tunnel_id": "tunnel_test",
                    "process_running": True,
                    "healthy": True,
                    "ready": True,
                    "runtime_state": "running",
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(
                os.environ,
                {
                    "PATH": "/usr/bin:/bin",
                    "HOME": "/tmp/keychain home",
                    "OPENAI_ADMIN_KEY": "fixture-admin-key",
                },
                clear=True,
            ),
            patch.object(
                tunnel.subprocess,
                "run",
                side_effect=[self.result([], stdout=key + "\n"), connect, status],
            ) as run,
        ):
            code, out, err = self.call(
                "start",
                "--client",
                str(self.client),
                "--binary",
                str(self.binary),
                "--config",
                str(self.config),
                "--tunnel-id",
                "tunnel_test",
                "--profile-dir",
                str(self.profile_dir),
            )

        self.assertEqual(code, 0, err)
        self.assertNotIn(key, out + err)
        keychain_argv = run.call_args_list[0].args[0]
        self.assertEqual(
            keychain_argv,
            [
                "security",
                "find-generic-password",
                "-s",
                "dev.localoud.mcp-tunnel",
                "-a",
                "localoud-tunnel-runtime",
                "-w",
            ],
        )
        keychain_env = run.call_args_list[0].kwargs["env"]
        self.assertEqual(keychain_env["PATH"], "/usr/bin:/bin")
        self.assertEqual(keychain_env["HOME"], "/tmp/keychain home")
        self.assertNotIn("OPENAI_ADMIN_KEY", keychain_env)
        connect_argv = run.call_args_list[1].args[0]
        self.assertNotIn(key, connect_argv)
        self.assertEqual(run.call_args_list[1].kwargs["env"]["CONTROL_PLANE_API_KEY"], key)

    def test_start_failure_redacts_runtime_key_from_bounded_diagnostic(self):
        key = "sensitive-runtime-key"
        failure = self.result([], code=7, stderr=f"request failed with {key} " + "x" * 2000)
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {"CONTROL_PLANE_API_KEY": key}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=failure),
        ):
            code, out, err = self.call(
                "start",
                "--client",
                str(self.client),
                "--binary",
                str(self.binary),
                "--config",
                str(self.config),
                "--tunnel-id",
                "tunnel_test",
                "--profile-dir",
                str(self.profile_dir),
            )

        self.assertNotEqual(code, 0)
        self.assertNotIn(key, out + err)
        self.assertIn("request failed", err)
        self.assertLessEqual(len(err), 700)

    def test_invalid_local_inputs_fail_before_keychain_or_client_calls(self):
        malformed = self.config.parent / "malformed.json"
        malformed.write_text('{"projects": []', encoding="utf-8")
        cases = [
            ("--config", str(malformed)),
            ("--client", str(self.client.parent / "missing-client")),
            ("--binary", str(self.binary.parent / "missing-binary")),
            ("--profile-dir", str(Path(self.temp.name) / "outside-target")),
            ("--health-port", "0"),
            ("--alias", "../unsafe-alias"),
            ("--alias", "localoud-readonly"),
            ("--tunnel-id", "../unsafe-tunnel"),
        ]
        for option, value in cases:
            with self.subTest(option=option), patch.object(
                tunnel, "REPO_ROOT", self.root
            ), patch.dict(os.environ, {}, clear=True), patch.object(
                tunnel.subprocess, "run"
            ) as run:
                code, _out, err = self.call(
                    "start",
                    "--client",
                    str(self.client),
                    "--binary",
                    str(self.binary),
                    "--config",
                    str(self.config),
                    "--tunnel-id",
                    "tunnel_test",
                    "--profile-dir",
                    str(self.profile_dir),
                    option,
                    value,
                )
            self.assertNotEqual(code, 0)
            self.assertTrue(err)
            run.assert_not_called()

    def test_status_filters_client_fields_and_never_reads_keychain(self):
        result = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "tunnel_id": "tunnel_test",
                    "process_running": True,
                    "healthy": False,
                    "ready": False,
                    "runtime_state": "degraded",
                    "stopped": False,
                    "credential": "must-not-be-printed",
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result) as run,
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertEqual(code, 0, err)
        self.assertNotIn("must-not-be-printed", out)
        self.assertIn('"runtime_state": "degraded"', out)
        self.assertEqual(len(run.call_args_list), 1)
        self.assertNotIn("CONTROL_PLANE_API_KEY", run.call_args.kwargs["env"])

    def test_status_rejects_unrecognized_json_instead_of_reporting_alias_only(self):
        result = self.result([], stdout='{"credential":"hidden"}')
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn("hidden", out + err)
        self.assertIn("invalid status", err)

    def test_status_skips_null_optional_id_when_other_state_fields_are_valid(self):
        result = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "tunnel_id": None,
                    "process_running": False,
                    "healthy": False,
                    "ready": False,
                    "runtime_state": "stopped",
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertEqual(code, 0, err)
        self.assertNotIn('"tunnel_id"', out)
        self.assertIn('"runtime_state": "stopped"', out)

    def test_status_failure_redacts_bearer_token_without_runtime_key(self):
        token = "FIXTURE_TOKEN"
        result = self.result(
            [], code=5, stderr=f"Authorization: Bearer {token}"
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_status_failure_redacts_quoted_bearer_token_without_runtime_key(self):
        token = "FIXTURE_QUOTED_STATUS_TOKEN"
        result = self.result(
            [], code=5, stderr=f'Authorization: "Bearer {token}"'
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_status_timeout_redacts_bearer_token_without_runtime_key(self):
        token = "FIXTURE_TOKEN"
        timeout = TimeoutExpired(
            ["tunnel-client", "runtimes", "status"],
            30,
            stderr=f"Authorization: Bearer {token}",
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", side_effect=timeout),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_status_timeout_redacts_single_quoted_bearer_token_without_runtime_key(self):
        token = "FIXTURE_QUOTED_TIMEOUT_TOKEN"
        timeout = TimeoutExpired(
            ["tunnel-client", "runtimes", "status"],
            30,
            stderr=f"Authorization='Bearer {token}'",
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", side_effect=timeout),
        ):
            code, out, err = self.call("status", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_stop_failure_redacts_bearer_token_without_runtime_key(self):
        token = "FIXTURE_TOKEN"
        result = self.result([], code=5, stderr=f"Authorization: Bearer {token}")
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_stop_failure_redacts_single_quoted_bearer_token_without_runtime_key(self):
        token = "FIXTURE_QUOTED_STOP_TOKEN"
        result = self.result(
            [], code=5, stderr=f"Authorization='Bearer {token}'"
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result),
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn(token, out + err)

    def test_stop_rejects_unverified_running_status(self):
        stop_result = self.result([], stdout="{}")
        running_status = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "tunnel_id": "tunnel_test",
                    "process_running": True,
                    "healthy": True,
                    "ready": True,
                    "runtime_state": "running",
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(
                tunnel.subprocess, "run", side_effect=[stop_result, running_status]
            ) as run,
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn('"stopped": true', out)
        self.assertEqual(len(run.call_args_list), 2)

    def test_stop_rejects_unrecognized_independent_status(self):
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(
                tunnel.subprocess,
                "run",
                side_effect=[self.result([], stdout="{}"), self.result([], stdout='{"other":"x"}')],
            ),
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn('"stopped": true', out)

    def test_stop_rejects_conflicting_stopped_status(self):
        status = self.result(
            [],
            stdout=json.dumps(
                {
                    "alias": "localoud-mcp-plugin",
                    "stopped": True,
                    "process_running": True,
                    "healthy": True,
                    "ready": True,
                    "runtime_state": "running",
                }
            ),
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(
                tunnel.subprocess, "run", side_effect=[self.result([]), status]
            ),
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertNotEqual(code, 0)
        self.assertNotIn('"stopped": true', out)

    def test_stop_does_not_read_keychain_and_prints_only_safe_status(self):
        result = self.result(
            [], stdout='{"alias":"localoud-mcp-plugin","stopped":true,"api_key":"secret"}'
        )
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result) as run,
        ):
            code, out, err = self.call("stop", "--client", str(self.client))

        self.assertEqual(code, 0, err)
        self.assertNotIn("api_key", out)
        self.assertIn('"stopped": true', out)
        calls = run.call_args_list
        self.assertEqual(len(calls), 2)
        self.assertEqual(
            calls[0].args[0][1:4], ["runtimes", "stop", "localoud-mcp-plugin"]
        )
        self.assertEqual(
            calls[1].args[0][1:4], ["runtimes", "status", "localoud-mcp-plugin"]
        )
        for call in calls:
            self.assertNotIn("CONTROL_PLANE_API_KEY", call.kwargs["env"])

    def test_doctor_uses_profile_environment_and_redacts_failure(self):
        key = "doctor-runtime-secret"
        result = self.result([], code=3, stderr=f"missing profile or key {key}")
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {"CONTROL_PLANE_API_KEY": key}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result) as run,
        ):
            code, out, err = self.call(
                "doctor",
                "--client",
                str(self.client),
                "--profile-dir",
                str(self.profile_dir),
            )

        self.assertNotEqual(code, 0)
        self.assertNotIn(key, out + err)
        self.assertIn("doctor failed", err)
        argv = run.call_args.args[0]
        self.assertEqual(argv[1], "doctor")
        self.assertIn("--profile-dir", argv)
        self.assertIn("--profile", argv)
        self.assertIn("--health.listen-addr", argv)
        self.assertEqual(
            argv[argv.index("--health.listen-addr") + 1], "127.0.0.1:0"
        )
        self.assertIn("--json", argv)
        self.assertNotIn(key, argv)
        self.assertEqual(run.call_args.kwargs["env"]["CONTROL_PLANE_API_KEY"], key)

    def test_doctor_success_prints_safe_summary_and_uses_ephemeral_health_port(self):
        key = "doctor-runtime-secret"
        result = self.result([], stdout=json.dumps({"result": "ok", "api_key": key}))
        with (
            patch.object(tunnel, "REPO_ROOT", self.root),
            patch.dict(os.environ, {"CONTROL_PLANE_API_KEY": key}, clear=True),
            patch.object(tunnel.subprocess, "run", return_value=result) as run,
        ):
            code, out, err = self.call(
                "doctor",
                "--client",
                str(self.client),
                "--profile-dir",
                str(self.profile_dir),
            )

        self.assertEqual(code, 0, err)
        self.assertEqual(out, "doctor: ok\n")
        self.assertNotIn(key, out + err)
        argv = run.call_args.args[0]
        self.assertEqual(
            argv[argv.index("--health.listen-addr") + 1], "127.0.0.1:0"
        )


if __name__ == "__main__":
    unittest.main()
