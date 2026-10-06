"""Bundle the dependency closure of Git for Windows tools, not the full Git install."""
import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import urllib.request
from pathlib import Path

root = Path(__file__).resolve().parent.parent
args = argparse.ArgumentParser()
args.add_argument("--git-root", type=Path)
args.add_argument("--proxy", default="")
options = args.parse_args()
git_root = options.git_root or Path(os.environ.get("ProgramFiles", r"C:\Program Files")) / "Git"
assert (git_root / "usr/bin/bash.exe").is_file(), "Install Git for Windows before packaging"
target = root / "bundle/resources/ip-check"
shutil.copytree(root / "vendor/region-restriction-check", target, dirs_exist_ok=True)
runtime = target / "runtime"
bin_dir = runtime / "usr/bin"
bin_dir.mkdir(parents=True, exist_ok=True)
(runtime / "tmp").mkdir(exist_ok=True)
(runtime / "tmp/.keep").write_text("Temporary files use the user's temp directory.\n", encoding="utf-8")
paths = [git_root / "usr/bin", git_root / "mingw64/bin"]

def imports(path):
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3c)[0]
    assert data[pe:pe+4] == b"PE\0\0"
    optional = pe + 24
    directory = optional + (112 if struct.unpack_from("<H", data, optional)[0] == 0x20b else 96)
    import_rva = struct.unpack_from("<I", data, directory + 8)[0]
    sections = optional + struct.unpack_from("<H", data, pe+20)[0]
    count = struct.unpack_from("<H", data, pe+6)[0]
    def address(rva):
        for i in range(count):
            length, start, size, offset = struct.unpack_from("<IIII", data, sections+i*40+8)
            if start <= rva < start + max(length, size):
                return offset+rva-start
        raise ValueError("Invalid PE address")
    if not import_rva:
        return []
    result = []
    at = address(import_rva)
    while any(data[at:at+20]):
        name = address(struct.unpack_from("<I", data, at+12)[0])
        result.append(data[name:data.index(b"\0", name)].decode("ascii"))
        at += 20
    return result

pending = [paths[0] / (name + ".exe") for name in
           ("bash", "awk", "grep", "sed", "cut", "tr", "head", "tail", "sort", "uniq",
            "wc", "cat", "date", "dirname", "sleep", "printf", "openssl", "cygpath", "rm", "md5sum", "xargs", "mktemp")]
pending.append(paths[1] / "curl.exe")
copied = set()
while pending:
    path = pending.pop()
    if path.name.lower() in copied:
        continue
    if not path.is_file():
        raise FileNotFoundError(path)
    copied.add(path.name.lower())
    shutil.copy2(path, bin_dir / path.name)
    for library in imports(path):
        dependency = next((folder / library for folder in [path.parent, *paths] if (folder / library).is_file()), None)
        if dependency:
            pending.append(dependency)
        elif not (Path(os.environ["SystemRoot"]) / "System32" / library).exists() and not library.lower().startswith(("api-ms-", "ext-ms-")):
            raise FileNotFoundError(f"Unresolved dependency {library} of {path}")

downloads = root / "tools/ip-check-downloads"
downloads.mkdir(parents=True, exist_ok=True)
opener = urllib.request.build_opener(urllib.request.ProxyHandler({"https": options.proxy} if options.proxy else {}))
def download(url, path, digest=None):
    if not path.exists() or (digest and hashlib.sha256(path.read_bytes()).hexdigest() != digest):
        with opener.open(url, timeout=90) as response:
            path.write_bytes(response.read())
    if digest and hashlib.sha256(path.read_bytes()).hexdigest() != digest:
        raise ValueError(f"Checksum mismatch: {path.name}")
digest = "23cb60a1354eed6bcc8d9b9735e8c7b388cd1fdcb75726b93bc299ef22dd9334"
download("https://github.com/jqlang/jq/releases/download/jq-1.8.1/jq-windows-amd64.exe", downloads / "jq.exe", digest)
download("https://raw.githubusercontent.com/jqlang/jq/jq-1.8.1/COPYING", downloads / "jq-LICENSE.txt")
shutil.copy2(downloads / "jq.exe", bin_dir / "jq.exe")
shutil.copy2(downloads / "jq-LICENSE.txt", runtime / "jq-LICENSE.txt")
shutil.copy2(git_root / "mingw64/etc/ssl/certs/ca-bundle.crt", runtime / "ca-bundle.crt")
shutil.copy2(git_root / "LICENSE.txt", runtime / "Git-for-Windows-LICENSE.txt")
shutil.copy2(root / "LICENSE", runtime / "COPYING-GPL-3.0.txt")
shutil.copy2(git_root / "etc/package-versions.txt", runtime / "package-versions.txt")
for folder in ("usr/share/licenses", "mingw64/share/licenses"):
    shutil.copytree(git_root / folder, runtime / folder, dirs_exist_ok=True)
records = [{"name": path.name, "sha256": hashlib.sha256(path.read_bytes()).hexdigest(), "size": path.stat().st_size}
           for path in sorted(bin_dir.iterdir())]
(runtime / "manifest.json").write_text(json.dumps({"files": records,
    "sources": ["https://github.com/git-for-windows/MSYS2-packages", "https://github.com/git-for-windows/MINGW-packages", "https://github.com/git-for-windows/msys2-runtime", "https://github.com/jqlang/jq/tree/jq-1.8.1"],
    "git_distribution": subprocess.check_output([str(git_root / "cmd/git.exe"), "--version"], text=True).strip(),
    "source_build_instructions": "https://gitforwindows.org/package-management"}, indent=2) + "\n", encoding="utf-8")
print(f"Bundled {len(records)} runtime files, {sum(item['size'] for item in records):,} bytes")
