"""Run JevBench public items straight through `jeff ask` and score them against `expected`.

Usage: python direct.py DATA_DIR [MODEL]
"""
import json, subprocess, sys, time, urllib.request
from collections import defaultdict
from pathlib import Path

OUT = Path(sys.argv[1])
MODEL = sys.argv[2] if len(sys.argv) > 2 else "typesafe/openjev-latest"
BASE = "https://raw.githubusercontent.com/fstandhartinger/jevbench/HEAD/datasets/public/"
OUT.mkdir(parents=True, exist_ok=True)

def label(a):
    if a["type"] == "noul":
        return "yes" if a["noul"] >= 0.5 else "no"
    if a["type"] == "score":
        return int(max(a["probabilities"], key=lambda k: a["probabilities"][k]))
    return a["choice"]


score = defaultdict(lambda: [0, 0])
rows = []
for tier in ("easy", "hard", "original"):
    path = OUT / f"{tier}.jsonl"
    if not path.exists():
        path.write_bytes(urllib.request.urlopen(BASE + f"{tier}.jsonl").read())
    for line in path.read_text(encoding="utf-8").splitlines():
        item = json.loads(line)
        body = json.dumps({"state": item["state"], "questions": {"q": item["question"]}})
        for attempt in range(5):
            # Fixed pacing stays under the provider limit of 60 requests a minute per key.
            time.sleep(1.1)
            t0 = time.monotonic()
            r = subprocess.run(["jeff", "ask", "--request", "-", "--model", MODEL, "--max-time", "120"],
                               input=body, capture_output=True, text=True, encoding="utf-8")
            secs = time.monotonic() - t0
            if "rate_limit" not in r.stdout + r.stderr:
                break
            time.sleep(30)
        tokens = None
        try:
            out = json.loads(r.stdout)
            tokens = out.get("usage", {}).get("input_tokens")
            got = label(out["answers"]["q"])
        except Exception:
            got = "ERROR: " + (r.stdout + r.stderr).strip()[:200]
        ok = str(got).lower() == str(item["expected"]).lower()
        score[tier][0] += ok
        score[tier][1] += 1
        rows.append({"id": item["id"], "tier": tier, "family": item.get("family"),
                     "type": item["question"]["type"], "expected": item["expected"], "got": got, "ok": ok,
                     "input_tokens": tokens, "seconds": round(secs, 3)})

(OUT / "direct-results.jsonl").write_text("\n".join(json.dumps(r) for r in rows), encoding="utf-8")
total = [sum(v[0] for v in score.values()), sum(v[1] for v in score.values())]
for k, (ok, n) in list(score.items()) + [("total", total)]:
    print(f"{k:9} {ok}/{n} = {ok / n:.1%}")
print("errors:", sum(str(r["got"]).startswith("ERROR") for r in rows))
