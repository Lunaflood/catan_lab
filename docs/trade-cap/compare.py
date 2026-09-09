from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
import json, subprocess, time
ROOT = Path(__file__).resolve().parents[2]
RUNS = ROOT / 'docs/trade-cap/runs'
def run(cap, players):
    games, seed = (48, 10020000) if players == 4 else (24, 10030000)
    cmd = [str(ROOT / 'target/release/catan.exe'), 'match', *(['v4']*players), '--games', str(games), '--seed', str(seed), '--max-offers', str(cap)]
    t = time.monotonic()
    p = subprocess.run(cmd, cwd=ROOT, capture_output=True, encoding='utf-8', errors='replace')
    name = f'{players}p-cap{cap}'
    (RUNS / f'{name}.txt').write_text(p.stdout+p.stderr, encoding='utf-8')
    if p.returncode: raise RuntimeError(f'{name}: {p.returncode}: {p.stderr}')
    row = json.loads(next(line.removeprefix('TRADE_JSON ') for line in p.stdout.splitlines() if line.startswith('TRADE_JSON ')))
    row['elapsed_s'] = round(time.monotonic()-t, 2)
    assert row['finished'] == games, row
    assert sum(row['offers_by_ordinal']) == row['offers']
    assert sum(row['deals_by_ordinal']) == row['deals']
    assert sum(row['responses_by_ordinal']) == row['offers']*(players-1)
    (RUNS / f'{name}.json').write_text(json.dumps(row, indent=2), encoding='utf-8')
    print(json.dumps(row), flush=True)
    return row
if __name__ == '__main__':
    with ThreadPoolExecutor(max_workers=4) as pool:
        jobs = [pool.submit(run, c, n) for n in (4,3) for c in (1,2,3,5)]
        rows = [f.result() for f in as_completed(jobs)]
    rows.sort(key=lambda r:(r['players'],r['cap']))
    (RUNS / 'summary.json').write_text(json.dumps(rows,indent=2), encoding='utf-8')
