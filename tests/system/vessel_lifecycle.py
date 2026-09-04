#!/usr/bin/env python3
"""Adversarial black-box coverage for Vessel's durable worker lifecycle."""
import concurrent.futures, json, pathlib, socket, subprocess, tempfile, time, urllib.error, urllib.request, uuid
ROOT = pathlib.Path(__file__).resolve().parents[2]; BINARY = ROOT / "target/release/vessel"

def request(base, method, path, body=None, token=None, expected=200, timeout=30):
    data = None if body is None else json.dumps(body).encode(); headers = {"Content-Type": "application/json"}
    if token: headers["Authorization"] = f"Bearer {token}"
    try:
        with urllib.request.urlopen(urllib.request.Request(base+path, data=data, headers=headers, method=method), timeout=timeout) as response: status, payload = response.status, response.read()
    except urllib.error.HTTPError as error: status, payload = error.code, error.read()
    assert status == expected, f"{method} {path}: expected {expected}, got {status}: {payload!r}"
    return json.loads(payload) if payload else None

def start(database, pairing_ttl=30):
    with socket.socket() as candidate: candidate.bind(("127.0.0.1",0)); port=candidate.getsockname()[1]
    base=f"http://127.0.0.1:{port}"; process=subprocess.Popen([str(BINARY),"--bind",f"127.0.0.1:{port}","--database",str(database),"--lease-secs","1","--stale-after-secs","1","--pairing-ttl-secs",str(pairing_ttl)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
    for _ in range(50):
        try:
            if request(base,"GET","/health",timeout=1)["status"]=="ok": return process,base
        except Exception: time.sleep(.1)
    raise AssertionError("Vessel failed to become healthy")

def stop(process):
    process.terminate()
    try: process.wait(timeout=5)
    except subprocess.TimeoutExpired: process.kill()

def begin(base, helm_id=None, version=1):
    helm_id=helm_id or str(uuid.uuid4()); body={"protocol_version":version,"helm":{"id":helm_id,"name":"system-test","version":"test","model":"mock","capabilities":["shell"]}}
    return helm_id,request(base,"POST","/v1/pairings/start",body,expected=200 if version==1 else 426)

def claim(base,pairing): return request(base,"POST","/v1/pairings/claim",{"connection_string":f"voyage:v1:{pairing['code']}"})
def heartbeat(base,token,version=1,expected=200): return request(base,"POST","/v1/worker/heartbeat",{"status":"online","protocol_version":version},token,expected)
def enqueue(base,helm_id,prompt="task"): return request(base,"POST",f"/v1/helms/{helm_id}/tasks",{"prompt":prompt,"session_id":None})

def main():
    assert BINARY.is_file(),f"missing {BINARY}; run cargo build --release"
    with tempfile.TemporaryDirectory() as temporary:
      database=pathlib.Path(temporary)/"vessel.db"; server,base=start(database)
      try:
        begin(base,version=999); helm_id,pairing=begin(base); token=pairing["worker_token"]
        request(base,"GET",f"/v1/pairings/{pairing['code']}",token="wrong",expected=401); claim(base,pairing)
        request(base,"POST","/v1/pairings/claim",{"connection_string":pairing["code"]},expected=409); heartbeat(base,token,999,426); heartbeat(base,token)
        with concurrent.futures.ThreadPoolExecutor() as pool:
            started=time.monotonic(); future=pool.submit(request,base,"GET","/v1/worker/tasks/next",None,token); time.sleep(.2); task=enqueue(base,helm_id,"wake"); envelope=future.result(timeout=3)
            assert envelope["id"]==task["id"] and time.monotonic()-started<3
        other_id,other=begin(base); claim(base,other)
        completion={"lease_id":envelope["lease_id"],"result":{"task_id":task["id"],"session_id":str(uuid.uuid4()),"answer":"done","input_tokens":1,"output_tokens":1}}
        request(base,"POST",f"/v1/worker/tasks/{task['id']}/result",completion,other["worker_token"],403)
        request(base,"POST",f"/v1/worker/tasks/{task['id']}/result",{**completion,"lease_id":str(uuid.uuid4())},token,409)
        assert request(base,"POST",f"/v1/worker/tasks/{task['id']}/result",completion,token)["state"]=="completed"
        cancelled=enqueue(base,helm_id,"cancel"); env=request(base,"GET","/v1/worker/tasks/next",token=token); request(base,"POST",f"/v1/tasks/{cancelled['id']}/cancel")
        late={"lease_id":env["lease_id"],"result":{**completion["result"],"task_id":cancelled["id"]}}; request(base,"POST",f"/v1/worker/tasks/{cancelled['id']}/result",late,token,409)
        failed=enqueue(base,helm_id,"fail")
        for attempt in range(3):
            env=request(base,"GET","/v1/worker/tasks/next",token=token)
            failure={"task_id":failed["id"],"lease_id":env["lease_id"],"error":f"failure {attempt}","retryable":True}
            if attempt == 0: request(base,"POST",f"/v1/worker/tasks/{failed['id']}/failure",failure,other["worker_token"],403)
            state=request(base,"POST",f"/v1/worker/tasks/{failed['id']}/failure",failure,token)
        assert state["state"]=="failed" and state["attempt"]==3
        leased=enqueue(base,helm_id,"lease"); first=request(base,"GET","/v1/worker/tasks/next",token=token); time.sleep(2.2); second=request(base,"GET","/v1/worker/tasks/next",token=token)
        assert second["id"]==leased["id"] and second["lease_id"]!=first["lease_id"] and second["attempt"]==2
        queued=enqueue(base,helm_id,"restart")
      finally: stop(server)
      server,base=start(database)
      try:
        heartbeat(base,token); assert request(base,"GET",f"/v1/tasks/{queued['id']}")["state"]=="queued"; assert request(base,"GET","/v1/worker/tasks/next",token=token)["id"]==queued["id"]
      finally: stop(server)
      server,base=start(database,pairing_ttl=1)
      try:
        _,expiring=begin(base); time.sleep(1.2); request(base,"POST","/v1/pairings/claim",{"connection_string":expiring["code"]},expected=410)
      finally: stop(server)
    print("Vessel adversarial lifecycle passed"); return 0

if __name__=="__main__": raise SystemExit(main())
