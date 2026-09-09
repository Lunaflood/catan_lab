"""Aggregate fixed-size CLI match blocks; recover exact counts from 0.1% rates.
For these 120/240-game blocks each possible win count has a unique printed rate.
"""
import glob, json, math, re, sys

def wilson(wins, total):
    z = 1.959963984540054
    p = wins / total
    d = 1 + z*z/total
    c = (p + z*z/(2*total)) / d
    r = z*math.sqrt(p*(1-p)/total + z*z/(4*total*total)) / d
    return [c-r, c+r]

def aggregate(pattern):
    blocks = []
    for path in sorted(glob.glob(pattern)):
        with open(path, encoding='utf-8-sig') as f:
            s = f.read()
        match = re.search(r'(\d+) 戦 \(決着 (\d+)\)', s)
        if not match:
            raise ValueError(f'Incomplete result: {path}')
        n, finished = map(int, match.groups())
        rate = float(re.search(r'^\s*v3\s+勝率\s+([\d.]+)%', s, re.M)[1])
        counts = [w for w in range(finished+1) if abs(w / max(finished,1) * 100 - rate) < .050001]
        if len(counts) != 1:
            raise ValueError(f'Ambiguous rounded win count: {path}')
        ms = re.search(r'^\s*v3\s+判断時間 p50 ([\d.]+)ms / p95 ([\d.]+)ms / max ([\d.]+)ms', s, re.M)
        blocks.append(dict(path=path, games=n, finished=finished, wins=counts[0],
                           p50_ms=float(ms[1]), p95_ms=float(ms[2]), max_ms=float(ms[3])))
    if not blocks:
        raise ValueError(f'No blocks: {pattern}')
    n = sum(b['games'] for b in blocks)
    wins = sum(b['wins'] for b in blocks)
    return dict(games=n, wins=wins, unfinished=sum(b['games']-b['finished'] for b in blocks),
                win_rate=wins/n, wilson95=wilson(wins,n),
                max_block_p95_ms=max(b['p95_ms'] for b in blocks),
                max_ms=max(b['max_ms'] for b in blocks), blocks=blocks)

if __name__ == '__main__':
    print(json.dumps({pattern: aggregate(pattern) for pattern in sys.argv[1:]}, indent=2))
