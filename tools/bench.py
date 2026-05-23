#!/usr/bin/env python3
"""bench.py — sobe o ragd local, roda lotes de query e mede latência (stdlib só).
Uso: bench.py [PORT] [BIN]   (acha ragfiles/drivers subindo de tools/ se preciso)."""
import sys, os, glob, time, json, subprocess, urllib.request, platform

PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 11599
BIN  = sys.argv[2] if len(sys.argv) > 2 else "./ragd"

here = os.path.dirname(os.path.abspath(__file__))
root = here if os.path.isdir(os.path.join(here, "ragfiles")) else os.path.dirname(here)
os.chdir(root)

args = [BIN, "--port", str(PORT)]
for f in sorted(glob.glob("ragfiles/*/*-tokenized.json")):
    coll = os.path.basename(os.path.dirname(f))
    name = os.path.basename(f)[:-len("-tokenized.json")]
    args += ["--preload", f"{coll}/{name}={f}"]

log = open("/tmp/ragd-bench.log", "w")
proc = subprocess.Popen(args, stdout=log, stderr=subprocess.STDOUT)
H = f"http://127.0.0.1:{PORT}"

def post(payload):
    req = urllib.request.Request(H + "/search", data=json.dumps(payload).encode(),
                                 headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=60) as r:
        return r.read()

def get(path):
    with urllib.request.urlopen(H + path, timeout=5) as r:
        return r.read()

hb = None
for _ in range(900):   # até 90s: load tokeniza todos os chunks (pesado em CPU lenta)
    try:
        hb = get("/health"); break
    except Exception:
        time.sleep(0.1)
if hb is None:
    print("ERRO: daemon não subiu. log:"); print(open('/tmp/ragd-bench.log').read()[:500])
    proc.terminate(); sys.exit(1)

print(f"host={platform.node()}  arch={platform.machine()}  {hb.decode()}")
post({"base": "*", "query": "anel", "k": 5})  # warmup

N = 25
def run(label, queries):
    ts = []
    for q in queries:
        for _ in range(N):
            t0 = time.perf_counter()
            post(q)
            ts.append((time.perf_counter() - t0) * 1000)
    ts.sort()
    avg = sum(ts) / len(ts)
    print(f"  {label:<22} {len(ts):4d} reqs  média {avg:7.2f} ms  p50 {ts[len(ts)//2]:7.2f} ms")

run("GLOBAL (25 bases)", [
    {"base": "*", "query": "índice invertido posicional", "k": 5},
    {"base": "*", "query": "matched filter cosseno", "k": 5},
    {"base": "*", "query": "Frodo Bolseiro montanha", "k": 5},
])
run("livros/sda (1 base)", [
    {"collection": "livros", "base": "sda", "query": "Frodo Bolseiro montanha", "k": 5},
    {"collection": "livros", "base": "sda", "query": "o anel de poder", "k": 5},
])
run("ragnarock (22 bases)", [
    {"collection": "ragnarock", "base": "*", "query": "índice invertido posicional", "k": 5},
    {"collection": "ragnarock", "base": "*", "query": "histograma esparso embedding", "k": 5},
])

proc.terminate()
try: proc.wait(timeout=5)
except Exception: proc.kill()
