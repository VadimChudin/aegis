import io
import json
import hashlib
import tarfile
import sys
import tempfile
from types import SimpleNamespace
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from aegis_lab.bridges import ai_setup


def make_tar(entries):
    stream = io.BytesIO()
    with tarfile.open(fileobj=stream, mode="w") as archive:
        for name, kind, data in entries:
            item = tarfile.TarInfo(name)
            if kind == "file":
                item.size = len(data)
                archive.addfile(item, io.BytesIO(data))
            elif kind == "directory":
                item.type = tarfile.DIRTYPE
                archive.addfile(item)
            elif kind == "symlink":
                item.type = tarfile.SYMTYPE
                item.linkname = data
                archive.addfile(item)
            else:
                item.type = tarfile.SYMTYPE
                item.linkname = data
                archive.addfile(item)
    stream.seek(0)
    return stream


class SafeExtractionTests(unittest.TestCase):
    def test_extracts_regular_executable_and_directory(self):
        stream = make_tar([
            ("bin", "directory", b""),
            ("bin/ollama", "file", b"runtime"),
        ])
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "stage"
            ai_setup.safe_extract(stream, target)
            binary = target / "bin" / "ollama"
            self.assertEqual(binary.read_bytes(), b"runtime")
            self.assertEqual(binary.stat().st_mode & 0o777, 0o644)

    def test_rejects_traversal_without_writing_outside_destination(self):
        stream = make_tar([("../outside", "file", b"bad")])
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "stage"
            with self.assertRaises(ai_setup.SetupError):
                ai_setup.safe_extract(stream, target)
            self.assertFalse((Path(tmp) / "outside").exists())

    def test_rejects_symlinks(self):
        stream = make_tar([("escape", "symlink", "../../outside")])
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ai_setup.SetupError):
                ai_setup.safe_extract(stream, Path(tmp) / "stage")

    def test_extracts_internal_symlinks_after_regular_files(self):
        stream = make_tar([
            ("lib/ollama/libx.so", "symlink", "libx.so.0"),
            ("lib/ollama/libx.so.0", "symlink", "libx.so.0.1"),
            ("lib/ollama/libx.so.0.1", "file", b"library"),
        ])
        with tempfile.TemporaryDirectory() as tmp:
            target = Path(tmp) / "stage"
            ai_setup.safe_extract(stream, target)
            self.assertTrue((target / "lib/ollama/libx.so").is_symlink())
            self.assertEqual((target / "lib/ollama/libx.so").read_bytes(), b"library")

    def test_rejects_corrupt_archive(self):
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(ai_setup.SetupError):
                ai_setup.safe_extract(io.BytesIO(b"not a tar archive"), Path(tmp) / "stage")


class RuntimeTests(unittest.TestCase):
    def test_progress_parsing_is_bounded_and_handles_status_only_events(self):
        self.assertEqual(ai_setup._model_progress({"total": 100, "completed": 50}), 62)
        self.assertEqual(ai_setup._model_progress({"total": 0, "completed": 1}), 25)
        self.assertEqual(ai_setup._model_progress({"status": "verifying digest"}), 25)
        self.assertEqual(ai_setup._model_progress({"total": 1, "completed": 100}), 99)

    def test_environment_is_project_scoped_local_and_cloud_disabled(self):
        with tempfile.TemporaryDirectory() as tmp:
            env = ai_setup.runtime_environment(Path(tmp) / "ai-runtime")
        self.assertEqual(env["OLLAMA_HOST"], "127.0.0.1:11434")
        self.assertEqual(env["OLLAMA_MODELS"], str((Path(tmp) / "ai-runtime" / "models").resolve()))
        self.assertEqual(env["OLLAMA_NO_CLOUD"], "1")

    def test_setup_rejects_model_directory_symlink(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "ai-runtime"
            root.mkdir()
            outside = Path(tmp) / "outside"
            outside.mkdir()
            (root / "models").symlink_to(outside, target_is_directory=True)
            with self.assertRaises(ai_setup.SetupError):
                ai_setup._ensure_root(root)

    def test_model_pull_targets_loopback_and_fixed_model(self):
        class Response:
            def __enter__(self):
                return self

            def __exit__(self, *_args):
                return False

            def readline(self):
                if hasattr(self, "read"):
                    return b""
                self.read = True
                return b'{"status":"success"}\n'

        with mock.patch.object(ai_setup.urllib.request, "urlopen", return_value=Response()) as urlopen:
            with mock.patch.object(ai_setup, "_model_ready", return_value=True):
                with mock.patch.object(ai_setup, "_emit"):
                    ai_setup._pull_model()
        request = urlopen.call_args.args[0]
        self.assertEqual(request.full_url, "http://127.0.0.1:11434/api/pull")
        self.assertEqual(json.loads(request.data), {"name": "qwen3:8b", "stream": True})

    def test_installed_binary_is_preferred_to_search_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary = Path(tmp) / "runtime" / "bin" / "ollama"
            binary.parent.mkdir(parents=True)
            binary.write_text("binary")
            binary.chmod(0o700)
            with mock.patch.object(ai_setup.shutil, "which", return_value="/usr/bin/ollama"):
                self.assertEqual(ai_setup.runtime_binary(tmp), str(binary))

    def test_download_checks_official_size_and_sha256_before_atomic_publish(self):
        payload = b"verified archive"
        digest = hashlib.sha256(payload).hexdigest()

        class Response(io.BytesIO):
            headers = {"Content-Length": str(len(payload))}

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                self.close()
                return False

        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "runtime.tar.zst"
            with mock.patch.object(ai_setup, "_request", return_value=Response(payload)):
                with mock.patch.object(ai_setup, "_emit"):
                    ai_setup._download("https://github.com/example/archive", destination, len(payload), "sha256:" + digest)
            self.assertEqual(destination.read_bytes(), payload)
            self.assertFalse(destination.with_suffix(".zst.part").exists())

    def test_download_rejects_bad_digest_without_publishing_archive(self):
        payload = b"untrusted archive"

        class Response(io.BytesIO):
            headers = {"Content-Length": str(len(payload))}

            def __enter__(self):
                return self

            def __exit__(self, *_args):
                self.close()
                return False

        with tempfile.TemporaryDirectory() as tmp:
            destination = Path(tmp) / "runtime.tar.zst"
            with mock.patch.object(ai_setup, "_request", return_value=Response(payload)):
                with mock.patch.object(ai_setup, "_emit"):
                    with self.assertRaises(ai_setup.SetupError):
                        ai_setup._download("https://github.com/example/archive", destination, len(payload), "sha256:" + "0" * 64)
            self.assertFalse(destination.exists())

    def test_preinstalled_ollama_is_exposed_at_primary_runtime_path(self):
        with tempfile.TemporaryDirectory() as tmp:
            system_binary = Path(tmp) / "system-ollama"
            system_binary.write_text("binary")
            system_binary.chmod(0o700)
            root = Path(tmp) / "ai-runtime"
            root.mkdir()
            binary = ai_setup._ensure_runtime_entrypoint(root, str(system_binary))
            self.assertEqual(Path(binary), root / "runtime" / "bin" / "ollama")
            self.assertTrue(Path(binary).is_symlink())
            self.assertEqual(Path(binary).resolve(), system_binary.resolve())

    def test_install_runtime_extracts_into_a_new_stage_and_cleans_it(self):
        archive_stream = make_tar([("bin/ollama", "file", b"ollama binary")])
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            archive_path = root / "ollama-linux-amd64-v1.2.3.tar.zst"

            def download(_url, destination, _size, _digest):
                destination.write_bytes(b"compressed")

            process = SimpleNamespace(stdout=archive_stream, wait=lambda: 0)
            with mock.patch.object(ai_setup.shutil, "which", return_value="/usr/bin/zstd"):
                with mock.patch.object(ai_setup, "_latest_archive_url", return_value=("v1.2.3", "https://github.com/ollama/ollama/releases/download/v1.2.3/ollama-linux-amd64.tar.zst", 10, None)):
                    with mock.patch.object(ai_setup, "_download", side_effect=download):
                        with mock.patch.object(ai_setup.subprocess, "Popen", return_value=process):
                            with mock.patch.object(ai_setup, "_emit"):
                                ai_setup._install_runtime(root)

            installed = root / "runtime/bin/ollama"
            self.assertEqual(installed.read_bytes(), b"ollama binary")
            self.assertTrue(installed.stat().st_mode & 0o100)
            self.assertEqual(list(root.glob(".ollama-stage-*")), [])


if __name__ == "__main__":
    unittest.main()
