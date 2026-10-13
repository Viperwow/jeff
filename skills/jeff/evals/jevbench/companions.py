"""Ask every JevBench public item alone, then beside two neutral questions about the same text, and compare accuracy.

Usage: python companions.py DATA_DIR MODEL PAUSE_SECONDS
"""
import json, subprocess, sys, time, urllib.request
from pathlib import Path

D, MODEL, PAUSE = Path(sys.argv[1]), sys.argv[2], float(sys.argv[3])
BASE = "https://raw.githubusercontent.com/fstandhartinger/jevbench/HEAD/datasets/public/"
COMPANIONS = {
    "english": {"type": "noul", "instructions": "Is the text written in English?",
                "criteria": {"true": "The text is in English", "false": "The text is in another language"}},
    "length": {"type": "choice", "instructions": "How long is the text?",
               "criteria": {"short": "A few sentences", "medium": "A few paragraphs", "long": "A page or more"}},
}


def label(a):
    if a["type"] == "noul":
        return "yes" if a["noul"] >= 0.5 else "no"
    if a["type"] == "score":
        return int(max(a["probabilities"], key=a["probabilities"].get))
    return a["choice"]


def ask(state, questions):
    for _ in range(5):
        time.sleep(PAUSE)
        r = subprocess.run(["jeff", "ask", "--request", "-", "--model", MODEL, "--max-time", "180"],
                           input=json.dumps({"state": state, "questions": questions}),
                           capture_output=True, text=True, encoding="utf-8")
        if "rate_limit" not in r.stdout + r.stderr:
            return json.loads(r.stdout)["answers"]["q"]
        time.sleep(30)
    raise RuntimeError(r.stdout + r.stderr)


D.mkdir(parents=True, exist_ok=True)
rows = []
for tier in ("easy", "hard", "original"):
    path = D / f"{tier}.jsonl"
    if not path.exists():
        path.write_bytes(urllib.request.urlopen(BASE + f"{tier}.jsonl").read())
    for line in path.read_text(encoding="utf-8").splitlines():
        it = json.loads(line)
        row = {"id": it["id"], "tier": tier, "type": it["question"]["type"], "expected": it["expected"]}
        try:
            for name, qs in (("alone", {"q": it["question"]}), ("companions", {"q": it["question"], **COMPANIONS})):
                got = label(ask(it["state"], qs))
                row[name] = {"got": got, "ok": str(got).lower() == str(it["expected"]).lower()}
        except Exception as e:
            row["error"] = str(e)[:300]
        rows.append(row)

(D / f"companions-{MODEL.replace('/', '_')}.jsonl").write_text("\n".join(json.dumps(r) for r in rows), encoding="utf-8")
ok = [r for r in rows if "error" not in r]
print(f"{MODEL}: {len(ok)} answered, {len(rows) - len(ok)} errors")
for group in ("noul", "choice", "score", "easy", "hard", "original"):
    g = [r for r in ok if group in (r["type"], r["tier"])]
    print(f"  {group:9} alone {sum(r['alone']['ok'] for r in g)}/{len(g)}  companions {sum(r['companions']['ok'] for r in g)}/{len(g)}"
          f"  flipped {sum(r['alone']['got'] != r['companions']['got'] for r in g)}")
