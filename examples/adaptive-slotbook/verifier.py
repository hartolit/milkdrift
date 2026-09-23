#!/usr/bin/env python3
"""Operator-owned finite HTTP verifier. Never import candidate source or trust its test report."""
import concurrent.futures
import datetime
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request

spec = json.loads(Path(sys.argv[1]).read_bytes())
root = Path(spec["directory"])
token = Path(spec["token_file"]).read_text().strip()
config = root / "config"
config.mkdir(mode=0o755)
data = root / "data"
data.mkdir(mode=0o777)
os.chmod(data, 0o777)
(config / "application.json").write_text(json.dumps(spec["application"]))
(config / "token").write_text(token)
(config / "clock").write_text("2027-04-10T09:00:00Z")
for p in config.iterdir():
    os.chmod(p, 0o444)
start, end = "2027-04-10T10:00:00Z", "2027-04-10T11:00:00Z"
name = spec["container"]
cidfile = root / "container.cid"
identity = None


def podman(*args):
    p = subprocess.run(["/usr/bin/podman", *args], stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, timeout=60, check=True)
    if len(p.stdout) > 1048576:
        raise RuntimeError("bounded engine observation overflow")
    return p.stdout


def request(method, path, body=None, authorization=None):
    headers = {"Content-Type": "application/json"}
    if authorization is not None:
        headers["Authorization"] = "Bearer " + authorization
    req = urllib.request.Request(base + path, data=None if body is None else json.dumps(body).encode(),
                                 headers=headers, method=method)
    try:
        response = urllib.request.urlopen(req, timeout=3)
    except urllib.error.HTTPError as exc:
        response = exc
    with response:
        content = response.read(16385)
        assert len(content) <= 16384 and token.encode() not in content
        return response.status, json.loads(content or b"null")


def ready():
    for _ in range(100):
        try:
            if request("GET", "/health")[0] == 200:
                return
        except Exception:
            pass
        time.sleep(.05)
    raise RuntimeError("candidate readiness not observed")


def booking(name="Ada", a=start, b=end, quantity=1, auth=token):
    return request("POST", "/reservations", {"name": name, "start": a, "end": b, "quantity": quantity}, auth)


def remaining(a=start, b=end):
    code, body = request("GET", "/availability?" + urllib.parse.urlencode({"start": a, "end": b}))
    assert code == 200
    assert set(body) == {"resource", "start", "end", "remaining"}
    assert body["resource"] == "pottery" and body["start"] == a and body["end"] == b
    assert "Ada" not in json.dumps(body) and "Bo" not in json.dumps(body) and token not in json.dumps(body)
    return body["remaining"]


def active():
    code, body = request("GET", "/reservations", authorization=token)
    assert code == 200 and isinstance(body, list)
    assert token not in json.dumps(body)
    return body


def clear():
    (config / "clock").chmod(0o644)
    (config / "clock").write_text("2027-04-10T09:00:00Z")
    (config / "clock").chmod(0o444)
    for item in active():
        assert request("DELETE", "/reservations/" + str(item["id"]), authorization=token)[0] == 200


def public_check():
    clear()
    assert remaining() == 2
    assert booking()[0] == 201
    assert remaining() == 1


def authentication_check():
    clear()
    before = active()
    for auth in (None, "incorrect"):
        assert booking(auth=auth)[0] == 401
        assert active() == before
    code, item = booking()
    assert code == 201
    for auth in (None, "incorrect"):
        assert request("DELETE", "/reservations/" + str(item["id"]), authorization=auth)[0] == 401
        assert remaining() == 1
    assert request("DELETE", "/reservations/" + str(item["id"]), authorization=token)[0] == 200
    assert remaining() == 2


def capacity_check():
    clear()
    for a, b, n in ((end, start, 1), (start, start, 1), ("not-a-date", end, 1), (start, end, 0)):
        assert booking(a=a, b=b, quantity=n)[0] == 400
    assert active() == []
    assert booking()[0] == 201
    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
        statuses = sorted(pool.map(lambda n: booking(name=n)[0], ["Bo", "Ci"]))
    assert statuses == [201, 409]
    assert remaining() == 0 and len(active()) == 2
    before = active()
    assert booking()[0] == 409 and active() == before
    assert booking(a=end, b="2027-04-10T12:00:00Z")[0] == 201
    assert remaining(end, "2027-04-10T12:00:00Z") == 1


def durability_check():
    global identity
    clear()
    assert booking()[0] == 201 and booking(name="Bo")[0] == 201
    assert booking()[0] == 409
    item = active()[0]
    assert request("DELETE", "/reservations/" + str(item["id"]), authorization=token)[0] == 200
    assert booking()[0] == 201
    before = active()
    podman("stop", "--time=5", identity)
    state = json.loads(podman("inspect", identity))[0]
    assert state["State"]["Running"] is False
    # The managed supervisor also recreates its container while retaining the owned data.
    podman("rm", identity)
    identity = None
    cidfile.unlink(missing_ok=True)
    launch()
    ready()
    assert active() == before and remaining() == 0


def cancellation_check():
    clear()
    code, item = booking()
    assert code == 201
    path = "/reservations/" + str(item["id"])
    assert request("DELETE", path, authorization=token)[0] == 200
    assert request("DELETE", path, authorization=token)[0] == 200 and remaining() == 2
    code, item = booking()
    assert code == 201
    for clock in (start, "2027-04-10T10:30:00Z"):
        (config / "clock").chmod(0o644)
        (config / "clock").write_text(clock)
        (config / "clock").chmod(0o444)
        assert request("DELETE", "/reservations/" + str(item["id"]), authorization=token)[0] == 409
        assert remaining() == 1


def exact_check():
    state = json.loads(podman("inspect", identity))[0]
    assert state["Image"].removeprefix("sha256:") == spec["image_identity"]
    assert state["HostConfig"]["ReadonlyRootfs"] is True
    mounts = {m["Destination"]: m for m in state["Mounts"]}
    assert mounts["/candidate/app.py"]["RW"] is False
    assert Path(mounts["/candidate/app.py"]["Source"]) == Path(spec["candidate"])
    assert mounts["/config"]["RW"] is False
    assert set(mounts).issubset({"/candidate/app.py", "/config", "/data", "/tmp", "/dev/shm", "/etc/hosts", "/etc/hostname", "/etc/resolv.conf"})


diagnostics = {}
checks = dict(zip(spec["required_checks"], [None] * len(spec["required_checks"])))
functions = {"public-availability": public_check, "authenticated-mutation": authentication_check,
             "capacity-and-intervals": capacity_check, "durable-bookings": durability_check,
             "cancellation-policy": cancellation_check, "exact-deployment": exact_check}
def launch():
    global identity, base
    lim = spec["limits"]
    podman("run", "--detach", "--pull=never", "--replace=false", "--name=" + name,
           "--cidfile=" + str(cidfile), "--label=org.milkdrift.verification=" + spec["evaluation"],
           "--label=org.milkdrift.platform=" + spec["platform_owner"],
           "--log-driver=none", "--read-only", "--read-only-tmpfs=false", "--tmpfs=/tmp:rw,size=" + str(lim["temporary_bytes"]),
           "--cap-drop=all", "--security-opt=no-new-privileges", "--userns=auto:size=65536",
           "--network=pasta:--no-map-gw", "--publish=127.0.0.1::8080", "--memory=" + str(lim["memory_bytes"]),
           "--memory-swap=" + str(lim["memory_bytes"]), "--cpus=" + str(lim["cpu_percent"] / 100), "--pids-limit=" + str(lim["pids"]),
           "--volume=" + spec["candidate"] + ":/candidate/app.py:ro", "--volume=" + str(config) + ":/config:ro",
           "--volume=" + str(data) + ":/data:U", "--entrypoint=" + spec["executable"], spec["image"], "-I", "/candidate/app.py")
    identity = cidfile.read_text().strip()
    hostport = podman("port", identity, "8080/tcp").decode().strip().rsplit(":", 1)[1]
    base = "http://127.0.0.1:" + hostport


try:
    launch()
    ready()
    for key in checks:
        try:
            functions[key]()
            checks[key] = True
        except Exception as error:
            checks[key] = False if isinstance(error, AssertionError) else None
            diagnostics[key] = "trusted check failed at verifier line " + str(next((f.lineno for f in reversed(traceback.extract_tb(error.__traceback__)) if f.filename == __file__), 0))
finally:
    if identity is None and cidfile.exists():
        identity = cidfile.read_text().strip()
    if identity:
        podman("rm", "--force", "--time=0", identity)
print(json.dumps([{"name": key, "passed": passed, "diagnostic": "observed declared check" if passed else diagnostics.get(key, "declared check could not be observed")} for key, passed in checks.items()]))
