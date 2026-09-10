import fs from 'node:fs';
import assert from 'node:assert/strict';
import {execFileSync} from 'node:child_process';
const load=async p=>(await WebAssembly.instantiate(fs.readFileSync(p),{})).instance.exports;
const old=(await WebAssembly.instantiate(execFileSync('git',['show','a9c6af0:web/colonist/catan_wasm.wasm']),{})).instance.exports;
const now=await load('web/colonist/catan_wasm.wasm');
const read=(w,fn='state_json')=>{const p=w[fn]();return JSON.parse(new TextDecoder().decode(new Uint8Array(w.memory.buffer,p,w.out_len())));};
const results=[],fixtures={};
for(const [n,seed] of [[3,151],[4,249]]) {
 const args=[seed,seed*3,seed*5,seed*7,n,0,228,0];old.game_new(...args);now.game_new(...args);
 const moves=[];let steps=0;
 for(;steps<12000;steps++) {
  const state=read(now);
  if(['MOVE_ROBBER','DISCARD','DECIDE_TRADE','DECIDE_ACCEPTEES'].includes(state.prompt) || (state.prompt==='PLAY_TURN' && state.rolled)) {
   const key=state.prompt;
   if(!fixtures[key])fixtures[key]={setup:{boardSeed:args[0],diceSeed:args[1],devSeed:args[2],stealSeed:args[3],players:n,mySeat:state.toAct,askLast:0},journal:moves.slice()};
  }
  const a=old.bot_step(),b=now.bot_step();assert.equal(a,b);assert.equal(now.fingerprint(),old.fingerprint());
  if(!a)break;
  assert.equal(old.last_action_index(),now.last_action_index());moves.push({i:now.last_action_index()});
 }
 const state=read(now),summary=read(now,'result_json');assert.notEqual(state.winner,null);
 const replay=await load('web/colonist/catan_wasm.wasm');replay.game_new(...args);
 assert.equal(read(replay,'result_json'),null);
 for(const m of moves)assert.equal(replay.apply_index(m.i),1);
 assert.equal(replay.fingerprint(),now.fingerprint());assert.deepEqual(read(replay,'result_json'),summary);
 results.push({players:n,steps,identicalCpuActions:true,replayHistoryMatches:true});
}
fs.writeFileSync('docs/assist-results-v14/parity-results.json',JSON.stringify(results,null,2));
fs.writeFileSync('docs/assist-results-v14/phase-fixtures.json',JSON.stringify(fixtures));
console.log(JSON.stringify({results,phases:Object.keys(fixtures)},null,2));
