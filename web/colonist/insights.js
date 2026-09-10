const escape = value => String(value).replace(/[&<>"']/g, c => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;","'":"&#39;"}[c]));
const ink=["#cc3936","#226bca","#626e7a","#ba6509"];
const theory=[1,2,3,4,5,6,5,4,3,2,1];

export function resultCharts(result, players, nameOf) {
  const frames=result?.timeline||[];
  const counts=result?.rollCounts||Array(11).fill(0);
  const total=counts.reduce((a,b)=>a+b,0);
  const bars=counts.map((n,i)=>{
    const expected=total*theory[i]/36;
    const max=Math.max(1,...counts,total/6);
    const x=46+i*55, h=n/max*130, ey=165-expected/max*130;
    return `<g><title>出目${i+2}：${n}回 / ${(total?n/total*100:0).toFixed(1)}%（理論 ${expected.toFixed(1)}回）</title>
      <rect x="${x}" y="${165-h}" width="31" height="${h}" rx="4" fill="${i===5?'#b96524':'#328c9d'}"/>
      <text x="${x+15.5}" y="${155-h}" text-anchor="middle">${n}</text>
      <path d="M${x-3} ${ey}h37" stroke="#c4444c" stroke-width="2"/>
      <text x="${x+15.5}" y="187" text-anchor="middle">${i+2}</text></g>`;
  }).join('');
  let progress='<p class="insight-empty">この対局の推移データはありません。</p>';
  if(frames.length) {
    const last=frames.at(-1), maxTurn=Math.max(1,last.turn), maxVp=Math.max(10,...frames.flatMap(f=>f.points));
    const x=t=>44+t/maxTurn*580, y=v=>220-v/maxVp*180;
    const grid=Array.from({length:Math.floor(maxVp/2)+1},(_,i)=>i*2).map(v=>`<path d="M44 ${y(v)}H624" class="chart-grid"/><text x="31" y="${y(v)+4}" text-anchor="end">${v}</text>`).join('');
    const lines=players.map(p=>{
      let d='';frames.forEach((f,i)=>{d+=i?`H${x(f.turn)}V${y(f.points[p.id])}`:`M${x(f.turn)} ${y(f.points[p.id])}`;});
      return `<path class="vp-line" data-player="${p.id}" d="${d}" stroke="${ink[p.id]}" fill="none" stroke-width="3" stroke-linejoin="round"><title>${escape(nameOf(p.id))}：最終${last.points[p.id]}点</title></path>`;
    }).join('');
    const legend=players.map(p=>`<span><i style="background:${ink[p.id]}"></i>${escape(nameOf(p.id))}</span>`).join('');
    progress=`<div class="chart-legend">${legend}</div>
      <svg class="vp-chart" viewBox="0 0 680 252" role="img" aria-label="全員の勝利点の推移。下のスライダーで各手番の点数を確認できます。">
        ${grid}<path d="M44 ${y(10)}H624" stroke="#b9943f" stroke-dasharray="5 4"/><text x="630" y="${y(10)+4}">10点</text>
        ${lines}<line id="vp-cursor" x1="${x(last.turn)}" x2="${x(last.turn)}" y1="32" y2="223" stroke="#324c5c" stroke-dasharray="3 4"/>
        <text x="44" y="244">初期配置</text><text x="624" y="244" text-anchor="end">${last.turn}手番</text>
      </svg>
      <label class="timeline-control">手番を振り返る<input id="vp-turn" type="range" min="0" max="${frames.length-1}" value="${frames.length-1}" step="1" aria-label="表示する手番"/></label>
      <div id="vp-readout" aria-live="polite"></div>`;
  }
  return `<section class="result-section"><h3>勝利への歩み</h3><p>各手番の最終時点の勝利点。伏せていた勝利点カードを含みます。</p>${progress}</section>
    <section class="result-section"><div class="chart-title"><h3>最終的な出目分布</h3><b>全${total}回</b></div>
      <p>棒が実際の回数、赤い線が理論上の期待回数です。</p>
      ${total?`<svg class="dice-chart" viewBox="0 0 680 201" role="img" aria-label="出目2から12の最終分布">${bars}</svg>`:'<p class="insight-empty">サイコロはまだ振られていません。</p>'}
      <details><summary>出目の数値を見る</summary><div class="chart-table-wrap"><table class="dice-table"><thead><tr><th>出目</th>${counts.map((_,i)=>`<th>${i+2}</th>`).join('')}</tr></thead><tbody><tr><th>回数</th>${counts.map(n=>`<td>${n}</td>`).join('')}</tr><tr><th>割合</th>${counts.map(n=>`<td>${(total?n/total*100:0).toFixed(1)}%</td>`).join('')}</tr></tbody></table></div></details>
    </section>`;
}

export function bindResultCharts(root,result,players,nameOf) {
  const input=root.querySelector('#vp-turn');
  if(!input) return;
  const frames=result.timeline, last=frames.at(-1);
  const update=()=>{
    const f=frames[Number(input.value)];
    root.querySelector('#vp-readout').innerHTML=`<b>${f.turn===0?'初期配置':f.turn+'手番目'}</b>`+players.map(p=>`<span style="color:${ink[p.id]}">${escape(nameOf(p.id))} <strong>${f.points[p.id]}点</strong></span>`).join('');
    const cursor=root.querySelector('#vp-cursor'), x=44+f.turn/Math.max(1,last.turn)*580;
    cursor.setAttribute('x1',x);cursor.setAttribute('x2',x);
  };
  input.addEventListener('input',update);update();
}
