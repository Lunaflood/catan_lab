"""最終テストの生の勝敗を集計して、全対局を分母にした勝率と Wilson 95% 区間を出す。

使い方: python docs/cpu-v2/aggregate.py docs/cpu-v2/runs/final4p-*.txt
"""
import glob
import json
import math
import re
import sys


def wilson(k, n, z=1.96):
    if n == 0:
        return (0.0, 0.0, 0.0)
    p = k / n
    d = 1 + z * z / n
    c = (p + z * z / (2 * n)) / d
    h = z * math.sqrt(p * (1 - p) / n + z * z / (4 * n * n)) / d
    return (p, c - h, c + h)


def main(patterns):
    jsonl = None
    if "--jsonl" in patterns:
        i = patterns.index("--jsonl")
        jsonl = patterns[i + 1]
        patterns = patterns[:i] + patterns[i + 2 :]
    files = []
    for pat in patterns:
        files.extend(sorted(glob.glob(pat)))
    rows = []
    games = finished = stalled = 0
    wins = None
    names = None
    turns = 0.0
    quant = []
    seeds = []
    for f in files:
        txt = open(f, encoding="utf-8").read()
        m = re.search(r"(\d+) 戦 \(決着 (\d+)\) / 平均 ([\d.]+) 手番", txt)
        if not m:
            print("未完:", f)
            continue
        g, fin, avg = int(m.group(1)), int(m.group(2)), float(m.group(3))
        games += g
        finished += fin
        turns += avg * fin
        ms = re.search(r"未決着 (\d+) / (\d+)", txt)
        if ms:
            stalled += int(ms.group(1))
        rows_ = re.findall(r"^\s+(\S+)\s+勝率\s+([\d.]+)% ± [\d.]+\s+平均VP ([\d.]+)", txt, re.M)
        if names is None:
            names = [r[0] for r in rows_]
            wins = [0] * len(rows_)
        for i, r in enumerate(rows_):
            wins[i] += round(float(r[1]) / 100.0 * fin)
        q = re.search(r"判断時間 p50 ([\d.]+)ms / p95 ([\d.]+)ms / max ([\d.]+)ms", txt)
        if q:
            quant.append(tuple(float(x) for x in q.groups()))
        sm = re.search(r"-s(\d+)\.txt$", f)
        if sm:
            seeds.append(int(sm.group(1)))
        rows.append({
            "file": f,
            "seed": int(sm.group(1)) if sm else None,
            "games": g,
            "finished": fin,
            "stalled": int(ms.group(1)) if ms else None,
            "avg_turns": avg,
            "wins": {f"{i}:{r[0]}": round(float(r[1]) / 100.0 * fin) for i, r in enumerate(rows_)},
            "decision_ms": {"p50": q.group(1), "p95": q.group(2), "max": q.group(3)} if q else None,
        })
    if not names:
        print("集計できる出力が無い")
        return
    n = len(names)
    null = 1.0 / n
    print(f"ファイル {len(files)} / 対局 {games} / 決着 {finished} / 未決着 {stalled} / 平均手番 {turns / max(finished, 1):.1f}")
    print(f"seed ブロック: {seeds}")
    print(f"帰無仮説 {null * 100:.1f}%（全対局を分母にした勝率と Wilson 95% 区間）")
    for i, nm in enumerate(names):
        p, lo, hi = wilson(wins[i], games)
        print(f"  {nm:<16} {wins[i]:>5} / {games}  {p * 100:5.1f}%  [{lo * 100:.1f}, {hi * 100:.1f}]")
    if jsonl:
        with open(jsonl, "w", encoding="utf-8") as out:
            for r in rows:
                out.write(json.dumps(r, ensure_ascii=False) + chr(10))
        print(f"JSONL: {jsonl}（{len(rows)} 行）")
    if quant:
        p50 = max(q[0] for q in quant)
        p95 = max(q[1] for q in quant)
        mx = max(q[2] for q in quant)
        print(f"候補の判断時間（各ブロックの最大値）: p50 {p50:.3f}ms / p95 {p95:.3f}ms / max {mx:.1f}ms")


if __name__ == "__main__":
    main(sys.argv[1:] or ["docs/cpu-v2/runs/final4p-*.txt"])
