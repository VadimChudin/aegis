"""Install and manage the project-local Ollama runtime used by the AI paper."""

import argparse
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path, PurePosixPath

MODEL = "qwen3:8b"
HOST = "127.0.0.1:11434"
API_ROOT = "http://127.0.0.1:11434"
RELEASE_API = "https://api.github.com/repos/ollama/ollama/releases/latest"
ARCHIVE_NAME = "ollama-linux-amd64.tar.zst"
MAX_DOWNLOAD = 15 * 1024**3
MAX_EXTRACTED = 32 * 1024**3
PULL_TIMEOUT = 90 * 60


class SetupError(Exception):
    """An expected, safe-to-report setup failure."""


def runtime_binary(root):
    """Return the project runtime first, then an already installed Ollama."""
    root = Path(root)
    for candidate in (root / "runtime" / "bin" / "ollama", root / "bin" / "ollama", root / "ollama"):
        if candidate.is_file() and os.access(candidate, os.X_OK):
            return str(candidate)
    return shutil.which("ollama")


def _ensure_runtime_entrypoint(root, binary):
    """Expose a preinstalled Ollama at the stable project-local launch path."""
    local = Path(root) / "runtime" / "bin" / "ollama"
    if local.is_file() and os.access(local, os.X_OK):
        return str(local)
    local.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        local.symlink_to(Path(binary).resolve())
    except FileExistsError as exc:
        raise SetupError("The project-local Ollama executable path is not usable") from exc
    return str(local)


def runtime_environment(root):
    root = Path(root).resolve()
    return {
        "OLLAMA_MODELS": str(root / "models"),
        "OLLAMA_HOST": HOST,
        "OLLAMA_NO_CLOUD": "1",
        "OLLAMA_CONTEXT_LENGTH": "8192",
    }


def _emit(status, progress):
    print(json.dumps({"status": str(status), "progress": max(0, min(100, float(progress)))}, separators=(",", ":")), flush=True)


def _supported_platform():
    if sys.platform != "linux" or platform.machine().lower() not in ("x86_64", "amd64"):
        raise SetupError("AI runtime setup supports Linux x86_64 only")


def _ensure_root(root):
    root = Path(root).expanduser().absolute()
    root.mkdir(parents=True, exist_ok=True, mode=0o700)
    if root.is_symlink() or not root.is_dir():
        raise SetupError("AI runtime directory must be a real directory")
    os.chmod(root, 0o700)
    models = root / "models"
    if models.is_symlink() or (models.exists() and not models.is_dir()):
        raise SetupError("AI model directory must stay inside the project runtime directory")
    models.mkdir(mode=0o700, exist_ok=True)
    os.chmod(models, 0o700)
    return root


def _request(url, *, data=None, headers=None, timeout=30):
    request = urllib.request.Request(url, data=data, headers=headers or {}, method="POST" if data is not None else "GET")
    try:
        return urllib.request.urlopen(request, timeout=timeout)
    except (OSError, urllib.error.URLError, TimeoutError) as exc:
        raise SetupError("Could not reach the Ollama service or official release host") from exc


def _latest_archive_url():
    with _request(RELEASE_API, headers={"Accept": "application/vnd.github+json", "User-Agent": "aegis-lab-ai-setup"}) as response:
        try:
            release = json.load(response)
        except (ValueError, OSError) as exc:
            raise SetupError("The official Ollama release response was invalid") from exc
    tag = release.get("tag_name", "")
    if not re.fullmatch(r"v[0-9]+(?:\.[0-9]+){1,3}", tag):
        raise SetupError("The official Ollama release version was invalid")
    asset = next((item for item in release.get("assets", []) if item.get("name") == ARCHIVE_NAME), None)
    if not asset:
        raise SetupError("The official Ollama release has no Linux x86_64 runtime archive")
    url = asset.get("browser_download_url", "")
    parsed = urllib.parse.urlparse(url)
    expected_path = f"/ollama/ollama/releases/download/{tag}/{ARCHIVE_NAME}"
    if parsed.scheme != "https" or parsed.hostname != "github.com" or parsed.path != expected_path:
        raise SetupError("The official Ollama release archive URL was invalid")
    size = asset.get("size")
    if not isinstance(size, int) or size < 1 or size > MAX_DOWNLOAD:
        raise SetupError("The official Ollama release archive size was invalid")
    digest = asset.get("digest")
    if digest is not None and (not isinstance(digest, str) or not re.fullmatch(r"sha256:[0-9a-fA-F]{64}", digest)):
        raise SetupError("The official Ollama release archive digest was invalid")
    return tag, url, size, digest


def _download(url, destination, expected_size, expected_digest=None):
    partial = destination.with_suffix(destination.suffix + ".part")
    try:
        with _request(url, headers={"User-Agent": "aegis-lab-ai-setup"}, timeout=60) as response:
            raw_length = response.headers.get("Content-Length")
            content_length = int(raw_length) if raw_length and raw_length.isdigit() else None
            if content_length is not None and content_length != expected_size:
                raise SetupError("Ollama runtime archive size did not match the official release metadata")
            if expected_size > MAX_DOWNLOAD:
                raise SetupError("Ollama runtime archive exceeds the 15 GiB download limit")
            digest = hashlib.sha256()
            received = 0
            last_report = 0
            with open(partial, "xb") as output:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    received += len(chunk)
                    if received > MAX_DOWNLOAD:
                        raise SetupError("Ollama runtime archive exceeds the 15 GiB download limit")
                    output.write(chunk)
                    digest.update(chunk)
                    percent = min(24, int(received * 24 / expected_size))
                    if percent > last_report:
                        _emit("downloading_runtime", percent)
                        last_report = percent
                output.flush()
                os.fsync(output.fileno())
            if received != expected_size:
                raise SetupError("Ollama runtime archive download was incomplete")
            if expected_digest and digest.hexdigest().lower() != expected_digest.partition(":")[2].lower():
                raise SetupError("Ollama runtime archive checksum did not match the official release")
        os.replace(partial, destination)
    except Exception:
        try:
            partial.unlink()
        except FileNotFoundError:
            pass
        raise


def _safe_member_path(name):
    components = name.split("/")
    if components and components[-1] == "":
        components.pop()
    if not components or any(part in ("", ".", "..") for part in components):
        raise SetupError("Ollama archive contains an unsafe path")
    path = PurePosixPath(name)
    if path.is_absolute():
        raise SetupError("Ollama archive contains an unsafe path")
    return path


def _safe_link_target(name):
    if not name or name.startswith("/") or "\\" in name or any(part == ".." for part in name.split("/")):
        raise SetupError("Ollama archive contains an unsafe symbolic link")


def safe_extract(stream, destination):
    """Extract files without following archive links; create confined links last."""
    destination = Path(destination)
    destination.mkdir(mode=0o700, parents=True, exist_ok=False)
    total = 0
    seen = set()
    links = []
    try:
        with tarfile.open(fileobj=stream, mode="r|") as archive:
            for member in archive:
                relative = _safe_member_path(member.name)
                key = relative.as_posix().rstrip("/")
                if key in seen:
                    raise SetupError("Ollama archive contains duplicate paths")
                seen.add(key)
                if member.issym():
                    _safe_link_target(member.linkname)
                    links.append((relative, member.linkname))
                    continue
                if not (member.isdir() or member.isfile()):
                    raise SetupError("Ollama archive contains a link or special file")
                target = destination.joinpath(*relative.parts)
                if member.isdir():
                    target.mkdir(mode=0o700, parents=True, exist_ok=True)
                    continue
                total += member.size
                if member.size < 0 or total > MAX_EXTRACTED:
                    raise SetupError("Ollama runtime archive expands beyond the safe limit")
                target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                source = archive.extractfile(member)
                if source is None:
                    raise SetupError("Ollama runtime archive is corrupt")
                with source, open(target, "xb") as output:
                    shutil.copyfileobj(source, output, length=1024 * 1024)
                    output.flush()
                mode = stat.S_IMODE(member.mode) & 0o755
                os.chmod(target, mode & ~0o022)
        for relative, linkname in links:
            target = destination.joinpath(*relative.parts)
            target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            os.symlink(linkname, target)
        return total
    except (tarfile.TarError, EOFError, OSError) as exc:
        if isinstance(exc, SetupError):
            raise
        raise SetupError("Ollama runtime archive is corrupt or could not be extracted") from exc


def _install_runtime(root):
    zstd = shutil.which("zstd")
    if not zstd:
        raise SetupError("The zstd command is required to install the local Ollama runtime")
    tag, url, size, digest = _latest_archive_url()
    archive = root / f"ollama-linux-amd64-{tag}.tar.zst"
    valid_cached_archive = False
    if archive.is_file() and archive.stat().st_size == size:
        hasher = hashlib.sha256()
        with open(archive, "rb") as cached:
            for chunk in iter(lambda: cached.read(1024 * 1024), b""):
                hasher.update(chunk)
        valid_cached_archive = not digest or hasher.hexdigest().lower() == digest.partition(":")[2].lower()
    if not valid_cached_archive:
        try:
            archive.unlink()
        except FileNotFoundError:
            pass
        _emit("downloading_runtime", 1)
        _download(url, archive, size, digest)
    stage_parent = Path(tempfile.mkdtemp(prefix=".ollama-stage-", dir=str(root)))
    os.chmod(stage_parent, 0o700)
    stage = stage_parent / "runtime"
    try:
        with open(archive, "rb") as compressed:
            process = subprocess.Popen([zstd, "-dc"], stdin=compressed, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
            try:
                safe_extract(process.stdout, stage)
                trailing = 0
                while True:
                    chunk = process.stdout.read(1024 * 1024)
                    if not chunk:
                        break
                    trailing += len(chunk)
                    if trailing > 64 * 1024 * 1024:
                        raise SetupError("Ollama runtime archive has excessive data after the tar stream")
            finally:
                if process.stdout:
                    process.stdout.close()
                result = process.wait()
            if result != 0:
                raise SetupError("Ollama runtime archive decompression failed")
        binary = stage / "bin" / "ollama"
        if not binary.is_file():
            raise SetupError("Ollama runtime archive did not contain its executable")
        os.chmod(binary, os.stat(binary).st_mode | stat.S_IXUSR)
        install = root / "runtime"
        if install.exists():
            backup = root / ".runtime-previous"
            if backup.exists():
                shutil.rmtree(backup)
            os.replace(install, backup)
            try:
                os.replace(stage, install)
            except Exception:
                os.replace(backup, install)
                raise
            shutil.rmtree(backup)
        else:
            os.replace(stage, install)
    finally:
        if stage_parent.exists():
            shutil.rmtree(stage_parent, ignore_errors=True)


def _api_json(path, payload=None, timeout=10):
    data = json.dumps(payload).encode("utf-8") if payload is not None else None
    headers = {"Content-Type": "application/json"} if data is not None else {}
    with _request(f"{API_ROOT}{path}", data=data, headers=headers, timeout=timeout) as response:
        try:
            return json.load(response)
        except (ValueError, OSError) as exc:
            raise SetupError("The local Ollama service returned invalid data") from exc


def _model_ready():
    try:
        tags = _api_json("/api/tags")
    except SetupError:
        return False
    return any(item.get("name") == MODEL for item in tags.get("models", []))


def _pull_model():
    body = json.dumps({"name": MODEL, "stream": True}).encode("utf-8")
    request = urllib.request.Request(
        f"{API_ROOT}/api/pull", data=body, headers={"Content-Type": "application/json"}, method="POST"
    )
    started = time.monotonic()
    last_report = -1
    try:
        response = urllib.request.urlopen(request, timeout=60)
    except (OSError, urllib.error.URLError, TimeoutError) as exc:
        raise SetupError("Could not start the local Qwen3 model download") from exc
    with response:
        while True:
            if time.monotonic() - started > PULL_TIMEOUT:
                raise SetupError("Qwen3 model download exceeded the 90 minute time limit")
            line = response.readline()
            if not line:
                break
            try:
                event = json.loads(line)
            except (ValueError, UnicodeDecodeError) as exc:
                raise SetupError("The local Ollama service returned an invalid download status") from exc
            if event.get("error"):
                raise SetupError("The local Qwen3 model download failed")
            progress = _model_progress(event)
            if progress > last_report:
                _emit("downloading_model", progress)
                last_report = progress
    if not _model_ready():
        raise SetupError("Qwen3 model download finished without a ready model")


def _model_progress(event):
    total = event.get("total")
    completed = event.get("completed")
    if isinstance(total, int) and total > 0 and isinstance(completed, int):
        return 25 + min(74, max(0, completed * 74 // total))
    return 25


def setup(root):
    _supported_platform()
    root = _ensure_root(root)
    _emit("checking_runtime", 0)
    binary = runtime_binary(root)
    if not binary:
        _install_runtime(root)
        binary = runtime_binary(root)
    if not binary:
        raise SetupError("Ollama runtime is not available after installation")
    binary = _ensure_runtime_entrypoint(root, binary)

    if _model_ready():
        _emit("ready", 100)
        return

    env = os.environ.copy()
    env.update(runtime_environment(root))
    owned = False
    process = None
    try:
        if not _api_reachable():
            _emit("starting_runtime", 24)
            process = subprocess.Popen(
                [binary, "serve"], env=env, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL, start_new_session=True,
            )
            owned = True
            if not _wait_for_api(process):
                raise SetupError("The local Ollama service did not start")
        if _model_ready():
            _emit("ready", 100)
            return
        _emit("preparing_model", 25)
        _pull_model()
        _emit("ready", 100)
    finally:
        if owned and process is not None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
            except (OSError, subprocess.TimeoutExpired):
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except OSError:
                    pass
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass


def _api_reachable():
    try:
        _api_json("/api/tags")
        return True
    except SetupError:
        return False


def _wait_for_api(process):
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        if process.poll() is not None:
            return False
        if _api_reachable():
            return True
        time.sleep(1)
    return False


def serve(root):
    _supported_platform()
    root = _ensure_root(root)
    binary = runtime_binary(root)
    if not binary:
        raise SetupError("Ollama runtime is not installed; run setup first")
    env = os.environ.copy()
    env.update(runtime_environment(root))
    os.execvpe(binary, [binary, "serve"], env)


def main(argv=None):
    parser = argparse.ArgumentParser(description="Prepare the project-local Qwen3 AI runtime")
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("setup", "serve"):
        command = commands.add_parser(name)
        command.add_argument("--root", required=True, help="project-local AI runtime directory")
    args = parser.parse_args(argv)
    try:
        if args.command == "setup":
            setup(args.root)
        else:
            serve(args.root)
        return 0
    except SetupError as exc:
        print(str(exc), file=sys.stderr)
        _emit("error", 0)
        return 1
    except Exception:
        print("AI runtime setup failed unexpectedly", file=sys.stderr)
        _emit("error", 0)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
