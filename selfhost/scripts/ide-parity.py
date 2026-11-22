#!/usr/bin/env python3
"""vcc's IDE against `vandorf serve`, route by route.

From the repo root, start both on the same directory, then run this:

    selfhost/rs/target/release/ide --addr 127.0.0.1:8080 &
    target/release/vandorf serve --addr 127.0.0.1:8081 &
    python3 selfhost/scripts/ide-parity.py

Known differences: the Go preview (vcc has no Go backend yet) and the
wording of parse errors (vcc's parser has its own messages); positions and
the rendering agree. Pages are
embedded when each server is built, so rebuild both after editing web/.
"""
import json, urllib.request

VCC, REF = 8080, 8081

def call(port, path, method="GET", body=None, headers={}):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=body, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()

def diff(a, b, path=""):
    if type(a) != type(b):
        return [f"{path}: {type(a).__name__} vs {type(b).__name__}"]
    if isinstance(a, dict):
        out = []
        for k in sorted(set(a) | set(b)):
            if k not in a or k not in b:
                out.append(f"{path}.{k}: only in {'vcc' if k in a else 'reference'}")
            else:
                out += diff(a[k], b[k], f"{path}.{k}")
        return out
    if isinstance(a, list):
        if len(a) != len(b):
            return [f"{path}: {len(a)} vs {len(b)} items"]
        return [d for i, (x, y) in enumerate(zip(a, b)) for d in diff(x, y, f"{path}[{i}]")]
    return [] if a == b else [f"{path}: {str(a)[:80]!r} vs {str(b)[:80]!r}"]

def report(name, a, b):
    (sa, a), (sb, b) = a, b
    if a == b:
        print(f"{name}: {sa}/{sb} same")
        return
    try:
        d = diff(json.loads(a), json.loads(b))
    except ValueError:
        d = ["not JSON, and different"]
    print(f"{name}: {sa}/{sb} {'same JSON' if not d else str(len(d)) + ' differences'}")
    for x in d[:4]:
        print("   ", x)

for p in ["/api/sources", "/api/tree", "/api/project?dir=examples/demo", "/api/project?dir=selfhost", "/api/project?dir=.", "/api/project?dir=src", "/api/project", "/ui.css", "/api/source?path=examples/demo/sys/calc.sys", "/api/source?path=../etc/passwd", "/api/source"]:
    report(p, call(VCC, p), call(REF, p))

demo = open("examples/demo/sys/calc.sys").read()
for name, body in [
    ("compile a project file", {"path": "examples/demo/sys/calc.sys"}),
    ("compile it, edited", {"path": "examples/demo/sys/calc.sys", "source": demo.replace("scale = 2", "scale = 3")}),
    ("compile vcc's own file", {"path": "selfhost/sys/sema/types.sys"}),
    ("compile a lone source", {"source": "proc Main(A): int { func A(n: int): int; A -> id(this + 1); }"}),
    ("compile a lone file", {"path": "tests/fixtures/test1.sys"}),
    ("compile a sema error", {"source": "proc P(A): int { func A(): string; }"}),
    ("compile a parse error", {"source": "proc P(A): int { func A(): int; A -> ; }"}),
    ("compile nothing", {}),
]:
    body = json.dumps(body).encode()
    h = {"Content-Type": "application/json"}
    report(name, call(VCC, "/api/compile", "POST", body, h), call(REF, "/api/compile", "POST", body, h))

# Pages, with each server's token blanked.
def token(port):
    import re
    return re.search(r"[0-9a-f]{32}", call(port, "/viewer")[1].decode()).group(0)
for p in ["/", "/project", "/explorer", "/viewer", "/terminal", "/workbench", "/viz"]:
    a, b = call(VCC, p), call(REF, p)
    a = (a[0], a[1].decode().replace(token(VCC), "TOKEN").encode())
    b = (b[0], b[1].decode().replace(token(REF), "TOKEN").encode())
    report(f"page {p}", a, b)

# Writes and terminals need the token and a loopback Host.
for name, h in [("no token", {}), ("token", {"X-Vandorf-Token": token(VCC)})]:
    print(f"terminal list, {name}:", call(VCC, "/api/term/list", headers={"Host": "127.0.0.1", **h})[0])

# Saving a manifest: written if it parses as one, refused (and left as it
# was) if not. On a scratch project both servers see.
import os, shutil
scratch = ".ide-parity"
os.makedirs(scratch, exist_ok=True)
good = '[project]\nname = "scratch"\n'
for name, text in [
    ("valid", '[project]\nname = "scratch"\n[rust]\nout = "rs/src/x"\n'),
    ("syntax error", '[project\nname = "x"\n'),
    ("unknown field", '[project]\nname = "x"\n[go]\nout = "a"\nmodul = "m"\n'),
    ("missing field", '[http.w]\naddr = "a"\n'),
    ("wrong type", '[project]\nname = 1\n'),
]:
    got = []
    for port in (VCC, REF):
        with open(f"{scratch}/vandorf.toml", "w") as f:
            f.write(good)
        body = json.dumps({"path": f"{scratch}/vandorf.toml", "source": text}).encode()
        h = {"Host": "127.0.0.1", "X-Vandorf-Token": token(port), "Content-Type": "application/json"}
        status, out = call(port, "/api/save", "POST", body, h)
        with open(f"{scratch}/vandorf.toml") as f:
            got.append((status, out + b"\n" + f.read().encode()))
    report(f"save manifest, {name}", *[(s, o.split(b"\n")[0]) for s, o in got])
    print("   file after:", "same" if got[0][1] == got[1][1] else "different")
for name, path in [("not a manifest or .sys", "README.md"), ("missing manifest", f"{scratch}/sub/vandorf.toml")]:
    body = json.dumps({"path": path, "source": "x"}).encode()
    report(f"save {name}", *[call(p, "/api/save", "POST", body, {"Host": "127.0.0.1", "X-Vandorf-Token": token(p)}) for p in (VCC, REF)])
shutil.rmtree(scratch)
