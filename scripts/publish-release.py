"""Publish reviewed release files using the current repository's Git credentials.

Credentials stay in memory, are never printed and are not saved by this script.
Python standard library only; supports an optional development proxy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request


class SafeRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected and urllib.parse.urlparse(newurl).hostname != urllib.parse.urlparse(req.full_url).hostname:
            redirected.remove_header("Authorization")
        return redirected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--proxy", default="")
    parser.add_argument("--check", action="store_true")
    parser.add_argument("--tag")
    parser.add_argument("--notes", type=Path)
    parser.add_argument("--asset", action="append", type=Path, default=[])
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    remote = subprocess.check_output(
        ["git", "remote", "get-url", "origin"], cwd=root, text=True
    ).strip()
    parsed = urllib.parse.urlparse(remote)
    if parsed.scheme != "https" or parsed.hostname != "github.com":
        raise RuntimeError("Expected an HTTPS github.com origin")
    repo = parsed.path.strip("/").removesuffix(".git")
    if len(repo.split("/")) != 2:
        raise RuntimeError("Invalid repository path")
    env = dict(os.environ, GIT_TERMINAL_PROMPT="0", GCM_INTERACTIVE="never")
    result = subprocess.run(
        ["git", "credential", "fill"], cwd=root, env=env,
        input=f"protocol=https\nhost=github.com\npath={repo}.git\n\n",
        text=True, capture_output=True,
    )
    credential = dict(
        line.split("=", 1) for line in result.stdout.splitlines() if "=" in line
    )
    token = credential.get("password")
    if result.returncode or not token:
        raise RuntimeError("No stored GitHub credential; authenticate Git locally first")
    opener = urllib.request.build_opener(
        urllib.request.ProxyHandler({"https": args.proxy} if args.proxy else {}),
        SafeRedirect(),
    )

    def request(path, method="GET", data=None, content_type="application/json", raw=False):
        url = path if path.startswith("https://uploads.github.com/") else "https://api.github.com" + path
        if isinstance(data, dict):
            data = json.dumps(data, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(url, data=data, method=method, headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2026-03-10",
            "User-Agent": "Clash-of-Rust-release",
            "Content-Type": content_type,
        })
        with opener.open(req, timeout=180) as response:
            return response.read() if raw else json.load(response)

    base = "/repos/" + repo
    info = request(base)
    if not info.get("permissions", {}).get("push"):
        raise RuntimeError("GitHub credential lacks repository push permission")
    if args.check:
        print("Authenticated repository:", info["full_name"])
        commit = subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=root, text=True
        ).strip()
        runs = request(base + "/actions/runs?per_page=20")
        matching_runs = [
            run for run in runs.get("workflow_runs", [])
            if run["head_sha"] == commit and run["event"] == "push"
        ]
        if not matching_runs:
            print("No push CI found for commit:", commit)
        for run in matching_runs[:1]:
            print("CI commit:", run["head_sha"])
            print("Latest CI:", run["status"], run["conclusion"], run["html_url"])
            if run["conclusion"] == "failure":
                jobs = request(base + f"/actions/runs/{run['id']}/jobs")
                for job in jobs["jobs"]:
                    if job["conclusion"] != "failure":
                        continue
                    print("Failed job:", job["name"])
                    for step in job["steps"]:
                        if step["conclusion"] == "failure":
                            print("Failed step:", step["name"])
                    check = job["check_run_url"].split("https://api.github.com", 1)[1]
                    for annotation in request(check + "/annotations"):
                        print(annotation.get("path"), annotation.get("message"))
                    logs = request(base + f"/actions/jobs/{job['id']}/logs", raw=True)
                    print("\n".join(logs.decode("utf-8", errors="replace").splitlines()[-45:]))
        return
    if not args.tag or not args.notes or not args.asset:
        raise RuntimeError("Provide --tag, --notes and --asset before publishing")
    notes = args.notes.read_text(encoding="utf-8")
    files = [(path, path.read_bytes()) for path in args.asset]
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip()
    releases = request(base + "/releases?per_page=100")
    release = next((item for item in releases if item["tag_name"] == args.tag), None)
    if release and not release["draft"]:
        raise RuntimeError("Release is already published; refusing to replace it")
    payload = {
        "tag_name": args.tag, "target_commitish": commit,
        "name": f"Clash of Rust {args.tag.removeprefix('v')}",
        "body": notes, "prerelease": False, "draft": True, "make_latest": "false",
    }
    release = request(base + "/releases" + (f"/{release['id']}" if release else ""),
                      "PATCH" if release else "POST", payload)
    upload = release["upload_url"].split("{", 1)[0]
    existing = {item["name"]: item for item in release["assets"]}
    for path, data in files:
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        asset = existing.get(path.name)
        if asset is None:
            asset = request(upload + "?" + urllib.parse.urlencode({"name": path.name}),
                            "POST", data, "application/octet-stream")
        if asset["size"] != len(data) or asset.get("digest") != digest:
            raise RuntimeError("Uploaded asset verification failed: " + path.name)
        print("Verified asset:", path.name)
    published = request(base + f"/releases/{release['id']}", "PATCH",
                        {"draft": False, "prerelease": False, "make_latest": "true"})
    print("Published:", published["html_url"])


if __name__ == "__main__":
    try:
        main()
    except urllib.error.HTTPError as error:
        print(f"GitHub API request failed (HTTP {error.code}); no credentials printed", file=sys.stderr)
        sys.exit(1)
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
