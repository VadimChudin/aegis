"""Offline regressions: no runtime or model downloads are performed."""
import io
import json
import sys
from pathlib import Path
from unittest import mock

import unittest
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from aegis_lab.bridges import ai_setup


class Response(io.BytesIO):
    def __init__(self, payload, headers=None):
        super().__init__(payload)
        self.headers = headers or {}


class AiSetupRegressionTests(unittest.TestCase):
    def test_malformed_tags_are_not_healthy_or_ready_0(self):
        tags = None
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_1(self):
        tags = []
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_2(self):
        tags = {}
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_3(self):
        tags = {'models': None}
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_4(self):
        tags = {'models': {}}
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_5(self):
        tags = {'models': [None]}
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_malformed_tags_are_not_healthy_or_ready_6(self):
        tags = {'models': [{'name': 123}]}
        with mock.patch.object(ai_setup, "_api_json", return_value=tags):
            assert not ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_empty_valid_tags_are_healthy_but_not_model_ready(self):
        with mock.patch.object(ai_setup, "_api_json", return_value={"models": []}):
            assert ai_setup._api_reachable()
            assert not ai_setup._model_ready()

    def test_setup_does_not_claim_foreign_service_models(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            with mock.patch.object(ai_setup, "_supported_platform"), \
                 mock.patch.object(ai_setup, "runtime_binary", return_value="/usr/bin/ollama"), \
                 mock.patch.object(ai_setup, "_ensure_runtime_entrypoint", return_value="/usr/bin/ollama"), \
                 mock.patch.object(ai_setup, "_model_ready", return_value=True), \
                 mock.patch.object(ai_setup, "_api_reachable", return_value=True), \
                 mock.patch.object(ai_setup, "_pull_model") as pull, \
                 mock.patch.object(ai_setup, "_emit") as emit:
                with self.assertRaisesRegex(ai_setup.SetupError, "already"):
                    ai_setup.setup(tmp_path)
                pull.assert_not_called()
                assert mock.call("ready", 100) not in emit.call_args_list

    def test_download_ignores_stale_partial_without_deleting_it(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            destination = tmp_path / "runtime.tar.zst"
            stale = tmp_path / "runtime.tar.zst.part"
            stale.write_bytes(b"another attempt or interrupted download")
            with mock.patch.object(ai_setup, "_request", return_value=Response(b"good")), \
                 mock.patch.object(ai_setup, "_emit"):
                ai_setup._download("https://example.invalid/runtime", destination, 4)
            assert destination.read_bytes() == b"good"
            assert stale.read_bytes() == b"another attempt or interrupted download"

    def test_download_stops_as_soon_as_official_size_is_exceeded(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            response = Response(b"x" * (2 * 1024 * 1024))
            response.read = mock.Mock(wraps=response.read)
            with mock.patch.object(ai_setup, "_request", return_value=response), \
                 mock.patch.object(ai_setup, "_emit"):
                with self.assertRaises(ai_setup.SetupError):
                    ai_setup._download("https://example.invalid/runtime", tmp_path / "runtime.tar.zst", 4)
            assert response.read.call_count == 1
            assert not list(tmp_path.iterdir())

    def test_pull_requires_terminal_success_even_if_model_tags_exist_0(self):
        payload = b''
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "success"):
                ai_setup._pull_model()

    def test_pull_requires_terminal_success_even_if_model_tags_exist_1(self):
        payload = b'{"status":"pulling manifest"}\n'
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "success"):
                ai_setup._pull_model()

    def test_pull_rejects_malformed_event_as_setup_error_0(self):
        event = None
        payload = json.dumps(event).encode() + b"\n"
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "invalid"):
                ai_setup._pull_model()

    def test_pull_rejects_malformed_event_as_setup_error_1(self):
        event = []
        payload = json.dumps(event).encode() + b"\n"
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "invalid"):
                ai_setup._pull_model()

    def test_pull_rejects_malformed_event_as_setup_error_2(self):
        event = 1
        payload = json.dumps(event).encode() + b"\n"
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "invalid"):
                ai_setup._pull_model()

    def test_pull_rejects_malformed_event_as_setup_error_3(self):
        event = {'total': 'a', 'completed': 1}
        payload = json.dumps(event).encode() + b"\n"
        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response(payload)), \
             mock.patch.object(ai_setup, "_model_ready", return_value=True), \
             mock.patch.object(ai_setup, "_emit"):
            with self.assertRaisesRegex(ai_setup.SetupError, "invalid"):
                ai_setup._pull_model()

    def test_health_probe_cannot_mark_exited_child_ready(self):
        process = mock.Mock()
        process.poll.side_effect = [None, 1]
        with mock.patch.object(ai_setup, "_api_reachable", return_value=True):
            assert not ai_setup._wait_for_api(process)

    def test_serve_uses_fixed_arguments_and_project_environment(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            with mock.patch.object(ai_setup, "_supported_platform"), \
                 mock.patch.object(ai_setup, "runtime_binary", return_value="/fake/ollama"), \
                 mock.patch.object(ai_setup.os, "execvpe") as execute:
                ai_setup.serve(tmp_path)
            binary, args, env = execute.call_args.args
            assert binary == "/fake/ollama"
            assert args == [binary, "serve"]
            assert env["OLLAMA_HOST"] == "127.0.0.1:11434"
            assert env["OLLAMA_MODELS"] == str((tmp_path / "models").resolve())
            assert env["OLLAMA_NO_CLOUD"] == "1"

    def test_windows_is_explicitly_unsupported_on_this_branch(self):
        with mock.patch.object(ai_setup.sys, "platform", "win32"), \
             mock.patch.object(ai_setup.platform, "machine", return_value="AMD64"):
            with self.assertRaisesRegex(ai_setup.SetupError, "Linux x86_64 only"):
                ai_setup._supported_platform()

    @unittest.skipIf(sys.platform != "linux", "Linux runtime regression")
    def test_setup_launches_and_reaps_real_local_server_without_models(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            import os
            import socket
            import textwrap

            # Reserve a transient loopback port, then let the child bind it. This is a
            # tiny protocol fixture, not Ollama: it never fetches or loads any weights.
            with socket.socket() as reservation:
                reservation.bind(("127.0.0.1", 0))
                port = reservation.getsockname()[1]
            binary = tmp_path / "runtime" / "bin" / "ollama"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!" + sys.executable + "\n" + textwrap.dedent('''\
                import json
                import os
                import sys
                from pathlib import Path
                from http.server import BaseHTTPRequestHandler, HTTPServer

                store = Path(os.environ["OLLAMA_MODELS"])
                (store.parent / "launch.json").write_text(json.dumps({
                    "pid": os.getpid(), "args": sys.argv[1:],
                    "models": str(store), "host": os.environ["OLLAMA_HOST"],
                    "no_cloud": os.environ["OLLAMA_NO_CLOUD"],
                }))
                ready = False

                class Handler(BaseHTTPRequestHandler):
                    def log_message(self, *_args):
                        pass

                    def reply(self, payload):
                        self.send_response(200)
                        self.send_header("Content-Length", str(len(payload)))
                        self.end_headers()
                        self.wfile.write(payload)

                    def do_GET(self):
                        models = [{"name": "qwen3:8b"}] if ready else []
                        self.reply(json.dumps({"models": models}).encode())

                    def do_POST(self):
                        global ready
                        assert self.path == "/api/pull"
                        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                        assert request == {"name": "qwen3:8b", "stream": True}
                        ready = True
                        self.reply(b'{"status":"success"}\\n')

                host, port = os.environ["OLLAMA_HOST"].split(":")
                HTTPServer((host, int(port)), Handler).serve_forever()
            '''))
            binary.chmod(0o700)
            host = f"127.0.0.1:{port}"
            with mock.patch.object(ai_setup, "HOST", host), \
                 mock.patch.object(ai_setup, "API_ROOT", f"http://{host}"), \
                 mock.patch.object(ai_setup, "_emit") as emit:
                ai_setup.setup(tmp_path)
            launch = json.loads((tmp_path / "launch.json").read_text())
            assert launch["args"] == ["serve"]
            assert launch["models"] == str((tmp_path / "models").resolve())
            assert launch["host"] == host
            assert launch["no_cloud"] == "1"
            assert mock.call("ready", 100) in emit.call_args_list
            with self.assertRaises(ProcessLookupError):
                os.kill(launch["pid"], 0)

    def test_setup_failed_health_reaps_child_and_never_pulls(self):
        with tempfile.TemporaryDirectory() as directory:
            tmp_path = Path(directory)
            process = mock.Mock(pid=123456)
            with mock.patch.object(ai_setup, "_supported_platform"), \
                 mock.patch.object(ai_setup, "runtime_binary", return_value="/fake/ollama"), \
                 mock.patch.object(ai_setup, "_ensure_runtime_entrypoint", return_value="/fake/ollama"), \
                 mock.patch.object(ai_setup, "_api_reachable", return_value=False), \
                 mock.patch.object(ai_setup, "_wait_for_api", return_value=False), \
                 mock.patch.object(ai_setup.subprocess, "Popen", return_value=process) as spawn, \
                 mock.patch.object(ai_setup.os, "killpg", create=True) as kill, \
                 mock.patch.object(ai_setup, "_pull_model") as pull, \
                 mock.patch.object(ai_setup, "_emit"):
                with self.assertRaisesRegex(ai_setup.SetupError, "did not start"):
                    ai_setup.setup(tmp_path)
            assert spawn.call_args.args[0] == ["/fake/ollama", "serve"]
            assert spawn.call_args.kwargs["start_new_session"] is True
            assert spawn.call_args.kwargs["env"]["OLLAMA_MODELS"] == str((tmp_path / "models").resolve())
            kill.assert_called_once_with(process.pid, ai_setup.signal.SIGTERM)
            process.wait.assert_called_once_with(timeout=10)
            pull.assert_not_called()
