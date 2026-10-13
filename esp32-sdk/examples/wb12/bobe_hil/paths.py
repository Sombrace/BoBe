import fcntl
import hashlib
import os
import subprocess
from contextlib import contextmanager
from collections.abc import Iterator
from pathlib import Path


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
PRIVATE_ROOT = REPOSITORY_ROOT / "private"
HIL_PRIVATE_ROOT = PRIVATE_ROOT / "hil"
RUNS_ROOT = PRIVATE_ROOT / "hil-runs"
SCENARIOS_ROOT = REPOSITORY_ROOT / "hil" / "scenarios"
APP_OFFSET = 0x20000


def git_metadata(root: Path) -> tuple[str, bool]:
    commit = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=root, check=True,
        capture_output=True, text=True,
    ).stdout.strip()
    status = subprocess.run(
        ["git", "status", "--porcelain", "--untracked-files=normal"],
        cwd=root, check=True, capture_output=True, text=True,
    ).stdout
    return commit, bool(status.strip())


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def private_root(override: Path | None = None) -> Path:
    """Repository-local private configuration root for HIL hardware."""

    return HIL_PRIVATE_ROOT if override is None else Path(override)


def regular_file(path: Path, description: str) -> Path:
    """Reject symbolic links and non-regular files before reading them."""

    if path.is_symlink() or not path.is_file():
        raise ValueError(f"{description} is unavailable or is not a regular file")
    return path


def private_key_file(path: Path, description: str) -> Path:
    """Require 0600-style modes on real private key material."""

    regular_file(path, description)
    if path.stat().st_mode & 0o077:
        raise ValueError(
            f"{description} must not be readable or writable by group or others"
        )
    return path


@contextmanager
def exclusive_device_access(name: str, *, root: Path | None = None) -> Iterator[Path]:
    """Serialize access to one physical device across processes."""

    lock_root = private_root(root)
    lock_root.mkdir(mode=0o700, parents=True, exist_ok=True)
    lock_path = lock_root / f"{name}.lock"
    descriptor = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            raise RuntimeError(f"{name} is in use by another process") from error
        yield lock_path
    finally:
        os.close(descriptor)
