"""Build the Windows deliverables from a compiled Rust executable."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import urllib.request
from zipfile import ZipFile, ZIP_DEFLATED

ROOT = Path(__file__).resolve().parents[1]
DEPS = json.loads((ROOT / "packaging" / "dependencies.json").read_text(encoding="utf-8"))


def digest(file):
    with file.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def dependency(name, cache):
    entry = DEPS[name]
    target = cache / entry["file"]
    if target.exists() and digest(target) == entry["sha256"]:
        return target
    temp = target.with_suffix(".download")
    request = urllib.request.Request(entry["url"], headers={"User-Agent": "KumaCapture-Packager"})
    with urllib.request.urlopen(request, timeout=120) as source, temp.open("wb") as writer:
        shutil.copyfileobj(source, writer)
    if digest(temp) != entry["sha256"]:
        temp.unlink()
        raise RuntimeError(f"SHA-256 verification failed for {name}")
    temp.replace(target)
    return target


def write_zip(stage, dest):
    with ZipFile(dest, "w", compression=ZIP_DEFLATED, compresslevel=5) as bundle:
        for path in sorted(stage.rglob("*")):
            if path.is_file() and path.name != "VC_redist.x64.exe":
                bundle.write(path, "KumaCapture/" + path.relative_to(stage).as_posix())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--cache", type=Path, default=ROOT / ".package-cache")
    parser.add_argument("--output", type=Path, default=ROOT / "dist")
    parser.add_argument("--nsis", default="makensis")
    args = parser.parse_args()
    cache, output = args.cache.resolve(), args.output.resolve()
    cache.mkdir(parents=True, exist_ok=True)
    output.mkdir(parents=True, exist_ok=True)
    stage = output / "stage"
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir()
    shutil.copy2(args.exe, stage / "KumaCapture.exe")
    with ZipFile(dependency("obs", cache)) as bundle:
        bundle.extractall(stage / "obs")
    with ZipFile(dependency("rclone", cache)) as bundle:
        member = next(name for name in bundle.namelist() if name.endswith("/rclone.exe"))
        (stage / "rclone").mkdir()
        with bundle.open(member) as source, (stage / "rclone" / "rclone.exe").open("wb") as target:
            shutil.copyfileobj(source, target)
    for name in ["TESTER.md", "THIRD-PARTY.md", "ShareX-LICENSE.txt", "rclone-LICENSE.txt"]:
        shutil.copy2(ROOT / "docs" / name, stage / name)
    portable = output / "KumaCapture-Windows-x64.zip"
    write_zip(stage, portable)
    # Microsoft's redistributable is included in setup for clean Windows installs.
    shutil.copy2(dependency("vc", cache), stage / "VC_redist.x64.exe")
    setup = output / "KumaCaptureSetup.exe"
    define = "/D" if os.name == "nt" else "-D"
    subprocess.run([args.nsis, f"{define}STAGE={stage}", f"{define}DEST={setup}", str(ROOT / "packaging" / "setup.nsi")], check=True)
    checksum = output / "SHA256SUMS.txt"
    checksum.write_text("".join(f"{digest(path)}  {path.name}\n" for path in (setup, portable)), encoding="ascii")


if __name__ == "__main__":
    main()
