from pathlib import Path
import json
ROOT=Path(__file__).resolve().parent
rows=json.loads((ROOT/'runs/summary.json').read_text(encoding='utf-8'))
lines=['## 上限別の実測結果', '', '全 288 局が決着。数値は 1 局あたりの平均。成立数は対案による成立も含む。', '']
for n in (4,3):
    rr=[r for r in rows if r['players']==n]
    lines += [f'### {n} 人戦（各条件 {rr[0]["games"]} 局）','', '| 1 手番の上限 | 新規提案数 | 成立数 | 総手番数 |', '|---:|---:|---:|---:|']
    for r in rr:
        d=r['games']
        lines.append(f'| {r["cap"]} 回 | {r["offers"]/d:.1f} | {r["deals"]/d:.1f} | {r["turns"]/d:.1f} |')
    r2=next(r for r in rr if r['cap']==2); r3=next(r for r in rr if r['cap']==3)
    lines += ['', f'3 回 → 2 回の変更で、提案数は {(1-r2["offers"]/r3["offers"])*100:.1f}% 減、成立数は {(1-r2["deals"]/r3["deals"])*100:.1f}% 減、手番数は {(r2["turns"]/r3["turns"]-1)*100:+.1f}% の変化。', '', '既定の上限 3 回での順番別内訳:', '', '| 提案の順番 | 提案数 | 成立数 | 成立割合 | 同じ手番で成立済みの後の提案 |', '|---:|---:|---:|---:|---:|']
    for i,(o,d,a) in enumerate(zip(r3['offers_by_ordinal'],r3['deals_by_ordinal'],r3['offers_after_deal']),1):
        lines.append(f'| {i} 回目 | {o} | {d} | {100*d/max(o,1):.1f}% | {a} |')
    lines += ['', '順番別の割合は、この条件で実際にそこまで提案した局面での観測値。3 回目を削ったときに失う件数と同義ではない（その後の行動とゲーム展開も変わる）。', '']
lines += ['生データ: [summary.json](runs/summary.json)。各ログも `runs` に保存。`probe.txt` は最初の 1 局の診断用再実行で、288 局の集計には含めない。','']
(ROOT/'results.md').write_text('\n'.join(lines),encoding='utf-8')
print('\n'.join(lines))
