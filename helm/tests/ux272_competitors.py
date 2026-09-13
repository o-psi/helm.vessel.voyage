#!/usr/bin/env python3
"""Run pinned dependency-free competitor paths offline, never install or log in.

Uses a network namespace and fresh HOME. Missing distributable/dependencies are
recorded as blocked, never called passing UX journeys. Node >=23 is needed to
execute Pi's unmodified dependency-free TypeScript components.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--node", type=Path, required=True, help="resolved executable, not a version-manager shim")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    home = args.output / "home"
    home.mkdir(mode=0o700)
    env = {"PATH": "/usr/bin:/bin", "HOME": str(home.resolve()), "LANG": "C.UTF-8",
           "XDG_CONFIG_HOME": str(home / "config"), "XDG_CACHE_HOME": str(home / "cache"),
           "XDG_DATA_HOME": str(home / "data"), "NO_COLOR": "1"}
    result = {"utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
              "isolation": "unshare --user --map-root-user --net; fresh HOME; no inherited credentials",
              "node": str(args.node), "runs": [],
              "limitations": "Component/launcher probes only, not competitor interactive onboarding/composer certification"}
    def run(name, command, sources, expected=None):
        item = {"name": name, "command": command,
                "sources": {str(p.relative_to(args.pins)): digest(p) for p in sources}}
        invocation = ["/usr/bin/unshare", "--user", "--map-root-user", "--net", *command]
        try:
            proc = subprocess.run(invocation, env=env, cwd=home, capture_output=True, timeout=12)
            text = (proc.stdout + proc.stderr).decode("utf-8", "replace")
            (args.output / (name + ".txt")).write_text(text)
            item.update(exit_code=proc.returncode,
                        outcome="passed component probe" if proc.returncode == 0 and expected and expected in text
                        else "blocked launch; see bounded diagnostic" if proc.returncode != 0
                        else "executed; no journey assertion")
        except subprocess.TimeoutExpired:
            item["outcome"] = "timed out; no journey result"
        result["runs"].append(item)
    # Namespace check must succeed before any foreign executable is evaluated.
    subprocess.run(["/usr/bin/unshare", "--user", "--map-root-user", "--net", "/usr/bin/true"],
                   env=env, check=True, timeout=5)
    pi = args.pins / "pi/source/packages/tui/src"
    files = [pi / name for name in ("keys.ts", "kill-ring.ts", "undo-stack.ts")]
    code = f'''
import assert from 'node:assert/strict';
import {{parseKey, matchesKey}} from {json.dumps(files[0].as_uri())};
import {{KillRing}} from {json.dumps(files[1].as_uri())};
import {{UndoStack}} from {json.dumps(files[2].as_uri())};
assert.equal(parseKey('\\x03'), 'ctrl+c');
assert.equal(matchesKey('\\x1b', 'escape'), true);
assert.equal(parseKey('\\x1b[A'), 'up');
const text = 'café e\\u0301 中文 👩‍💻\\nsecond line';
const ring = new KillRing();
ring.push(text, {{prepend:false}});
ring.push(' tail', {{prepend:false, accumulate:true}});
assert.equal(ring.peek(), text + ' tail');
ring.push('older', {{prepend:false}}); ring.rotate();
assert.equal(ring.peek(), text + ' tail');
const undo = new UndoStack();
const state = {{text}}; undo.push(state); state.text = 'changed';
assert.deepEqual(undo.pop(), {{text}}); assert.equal(undo.pop(), undefined);
console.log('PASS: pinned Pi key decoding, exact Unicode kill/yank, detached undo snapshot');
'''
    probe = args.output / "pi-components.mjs"
    probe.write_text(code)
    run("pi-components", [str(args.node), str(probe.resolve())], files, "PASS:")
    run("pi-launch", [str(args.node), str(args.pins / "pi/source/packages/coding-agent/dist/cli.js"), "--help"],
        [args.pins / "pi/source/packages/coding-agent/package.json"])
    opencode = args.pins / "opencode/source/packages/opencode/bin/opencode"
    run("opencode-launch", [str(args.node), str(opencode), "--help"], [opencode])
    codex = next((args.pins / "codex").glob("openai-codex-*/codex-cli/bin/codex.js"))
    run("codex-launch", [str(args.node), str(codex), "--help"], [codex])
    for product, path in [("pi", "pi/manifest.json"), ("opencode", "opencode/pin.json"), ("codex", "codex/commit.json")]:
        pin = json.loads((args.pins / path).read_text())
        result[product + "_revision"] = pin.get("revision", pin.get("sha", pin.get("commit")))
    (args.output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
