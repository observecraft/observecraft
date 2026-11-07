#!/usr/bin/env python3
"""ObserveCraft's Cloud Security scanner (docs/adr/0006-cloud-posture-scanning.md).

Runs inside Prowler's official image, pinned by digest and otherwise unmodified: this file is mounted next to it
(compose volume, Helm ConfigMap) and replaces the image's entrypoint. Standard library only.

Loop: claim a scan from the server's internal listener (`OC_SCANNER_SERVER`, bearer `OC_SCANNER_TOKEN`), run
Prowler on the account with the short-lived, read-only credentials the server hands out, renew the claim while it
runs, upload the `json-ocsf` results (gzip) or the error, and claim the next one; sleep while there's nothing to do.
The credentials live only in Prowler's environment and are never logged or written to disk.
"""

import gzip
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request

SERVER = os.environ.get("OC_SCANNER_SERVER", "http://server:8090").rstrip("/")
TOKEN = os.environ.get("OC_SCANNER_TOKEN", "")
NAME = os.environ.get("OC_SCANNER_NAME") or socket.gethostname()
IDLE_SECS = int(os.environ.get("OC_SCANNER_IDLE_SECS", "30"))
# The server's credentials last an hour; stop Prowler before they expire.
SCAN_TIMEOUT_SECS = int(os.environ.get("OC_SCANNER_TIMEOUT_SECS", "3300"))
HEARTBEAT_SECS = 120
# The official image installs Prowler in a virtualenv (its entrypoint).
PROWLER = os.environ.get("OC_PROWLER_BIN") or ("/home/prowler/.venv/bin/prowler" if os.path.exists("/home/prowler/.venv/bin/prowler") else "prowler")


def log(message, **fields):
    print(json.dumps({"ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "msg": message, **fields}), flush=True)


def call(method, path, body=None, headers=None, timeout=60):
    """(status, body) of a request to the server; network errors are status 0."""
    request = urllib.request.Request(
        SERVER + path,
        data=body,
        method=method,
        headers={"Authorization": "Bearer " + TOKEN, "X-OC-Scanner": NAME, **(headers or {})},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.status, response.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except (urllib.error.URLError, OSError) as e:
        return 0, str(e).encode()


def report_failure(scan_id, error):
    call("POST", f"/internal/posture/scans/{scan_id}/fail", json.dumps({"error": error[-2000:]}).encode(), {"Content-Type": "application/json"})


def keep_claim(scan_id, lost, done):
    """Renews the claim until `done`; sets `lost` when the server no longer has the scan running."""
    while not done.wait(HEARTBEAT_SECS):
        status, _ = call("POST", f"/internal/posture/scans/{scan_id}/heartbeat")
        if status == 409:
            lost.set()
            return


def prowler_command(assignment, output_dir):
    if assignment["provider"] != "aws":
        raise RuntimeError(f"provider {assignment['provider']} isn't supported by this scanner yet")
    command = [
        PROWLER, "aws",
        "--output-formats", "json-ocsf",
        "--output-directory", output_dir,
        "--output-filename", "results",
        "--no-banner",
        "--ignore-exit-code-3",
    ]
    if assignment.get("regions"):
        command += ["--region", *assignment["regions"]]
    return command


def scan(assignment):
    scan_id = assignment["scanId"]
    creds = assignment["credentials"]
    output_dir = tempfile.mkdtemp(prefix="oc-scan-")
    # Prowler gets only these credentials, not the scanner's own environment's AWS or ObserveCraft settings.
    env = {k: v for k, v in os.environ.items() if not k.startswith(("AWS_", "OC_"))}
    env.update(
        AWS_ACCESS_KEY_ID=creds["accessKeyId"],
        AWS_SECRET_ACCESS_KEY=creds["secretAccessKey"],
        AWS_SESSION_TOKEN=creds["sessionToken"],
        AWS_DEFAULT_REGION=(assignment.get("regions") or ["us-east-1"])[0],
    )
    lost, done = threading.Event(), threading.Event()
    threading.Thread(target=keep_claim, args=(scan_id, lost, done), daemon=True).start()
    started = time.monotonic()
    log("scan started", scan=scan_id, account=assignment["accountId"], regions=len(assignment.get("regions") or []))
    try:
        stderr_path = os.path.join(output_dir, "prowler.log")
        with open(stderr_path, "w") as stderr:
            proc = subprocess.Popen(prowler_command(assignment, output_dir), env=env, stdout=subprocess.DEVNULL, stderr=stderr)
            while proc.poll() is None:
                if lost.is_set() or time.monotonic() - started > SCAN_TIMEOUT_SECS:
                    proc.kill()
                    proc.wait()
                    raise RuntimeError("the scan was cancelled by the server" if lost.is_set() else f"Prowler ran longer than {SCAN_TIMEOUT_SECS} s")
                time.sleep(2)
        if proc.returncode not in (0, 3):
            with open(stderr_path, errors="replace") as f:
                tail = f.read()[-1500:]
            raise RuntimeError(f"Prowler exited with {proc.returncode}: {tail}")
        results = os.path.join(output_dir, "results.ocsf.json")
        if not os.path.exists(results):
            raise RuntimeError("Prowler wrote no json-ocsf results")
        with open(results, "rb") as f:
            body = gzip.compress(f.read())
        status, answer = call("POST", f"/internal/posture/scans/{scan_id}/results", body, {"Content-Type": "application/json", "Content-Encoding": "gzip"}, timeout=600)
        if status != 200:
            raise RuntimeError(f"uploading the results failed: {status} {answer[:300]!r}")
        log("scan stored", scan=scan_id, seconds=round(time.monotonic() - started), **json.loads(answer or b"{}"))
    except Exception as e:  # noqa: BLE001 - every failure is reported to the server
        log("scan failed", scan=scan_id, error=str(e)[:500])
        report_failure(scan_id, str(e))
    finally:
        done.set()
        shutil.rmtree(output_dir, ignore_errors=True)


def main():
    if len(TOKEN) < 16:
        log("OC_SCANNER_TOKEN is missing or shorter than 16 characters; the server hands out no scans without it")
        sys.exit(1)
    log("scanner ready", server=SERVER, name=NAME)
    while True:
        status, body = call("POST", "/internal/posture/claim")
        if status == 200:
            scan(json.loads(body))
            continue
        if status not in (204, 0):
            log("claiming a scan failed", status=status, body=body[:300].decode(errors="replace"))
        elif status == 0:
            log("the server can't be reached", error=body[:300].decode(errors="replace"))
        time.sleep(IDLE_SECS)


if __name__ == "__main__":
    main()
