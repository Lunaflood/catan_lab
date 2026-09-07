import fs from 'node:fs';
import assert from 'node:assert/strict';
const {instance}=await WebAssembly.instantiate(fs.readFileSync('web/catan_wasm.wasm'),{});
const w=instance.exports;
function state(){const ptr=w.state_json();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,ptr,w.out_len())));}
const results=[],timings=[];
for(let seed=1;seed<=8;seed++){
 const n=seed%2?4:3;
 w.game_new(1500000+seed,seed*971,seed*677,seed*331,n,0,255);
 let steps=0;
 for(;steps<12000;steps++){
  const t=performance.now(),ok=w.bot_step();timings.push(performance.now()-t);
  if(!ok)break;
 }
 const s=state();assert.notEqual(s.winner,null);assert.ok(steps<12000);
 results.push({seed,players:n,steps,turn:s.turn,winner:s.winner});
}
timings.sort((a,b)=>a-b);
console.log(JSON.stringify({results,decisionMs:{median:timings[Math.floor(timings.length*.5)],p95:timings[Math.floor(timings.length*.95)],max:timings.at(-1)}},null,2));
