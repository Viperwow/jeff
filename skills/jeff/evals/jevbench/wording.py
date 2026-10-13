"""Ask every JevBench public item twice in one call: as published, and with its option descriptions stripped.

Usage: python wording.py DATA_DIR MODEL PAUSE_SECONDS
"""
import json, subprocess, sys, time, urllib.request
from pathlib import Path

D, MODEL, PAUSE = Path(sys.argv[1]), sys.argv[2], float(sys.argv[3])
BASE = "https://raw.githubusercontent.com/fstandhartinger/jevbench/HEAD/datasets/public/"
TIERS = ("easy", "hard", "original")


def bare(q):
    q = dict(q)
    if q["type"] == "noul":
        q.pop("criteria", None)
    elif q["type"] == "choice":
        q["criteria"] = {k: k for k in q["criteria"]}
    else:
        q["criteria"] = [f"Level {i}" for i in range(len(q["criteria"]))]
    return q


def answer(a):
    if a["type"] == "noul":
        return ("yes" if a["noul"] >= 0.5 else "no"), max(a["noul"], 1 - a["noul"])
    if a["type"] == "score":
        k = max(a["probabilities"], key=a["probabilities"].get)
        return int(k), a["probabilities"][k]
    return a["choice"], a["confidence"]


D.mkdir(parents=True, exist_ok=True)
rows = []
for tier in TIERS:
    path = D / f"{tier}.jsonl"
    if not path.exists():
        path.write_bytes(urllib.request.urlopen(BASE + f"{tier}.jsonl").read())
    for line in path.read_text(encoding="utf-8").splitlines():
        it = json.loads(line)
        body = json.dumps({"state": it["state"], "questions": {"full": it["question"], "bare": bare(it["question"])}})
        for _ in range(5):
            time.sleep(PAUSE)
            r = subprocess.run(["jeff", "ask", "--request", "-", "--model", MODEL, "--max-time", "180"],
                               input=body, capture_output=True, text=True, encoding="utf-8")
            if "rate_limit" not in r.stdout + r.stderr:
                break
            time.sleep(30)
        row = {"id": it["id"], "tier": tier, "family": it.get("family"), "type": it["question"]["type"],
               "expected": it["expected"]}
        try:
            out = json.loads(r.stdout)
            row["input_tokens"] = out.get("usage", {}).get("input_tokens")
            for k in ("full", "bare"):
                got, conf = answer(out["answers"][k])
                row[k] = {"got": got, "confidence": conf, "ok": str(got).lower() == str(it["expected"]).lower()}
        except Exception:
            row["error"] = (r.stdout + r.stderr).strip()[:300]
        rows.append(row)

name = MODEL.replace("/", "_")
(D / f"wording-{name}.jsonl").write_text("\n".join(json.dumps(r) for r in rows), encoding="utf-8")
ok = [r for r in rows if "error" not in r]
print(f"{MODEL}: {len(ok)} answered, {len(rows) - len(ok)} errors")
for group in ("noul", "choice", "score", *TIERS):
    g = [r for r in ok if group in (r["type"], r["tier"])]
    print(f"  {group:9} full {sum(r['full']['ok'] for r in g)}/{len(g)}  bare {sum(r['bare']['ok'] for r in g)}/{len(g)}")
