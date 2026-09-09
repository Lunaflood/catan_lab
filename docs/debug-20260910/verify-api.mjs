import fs from 'node:fs';
import assert from 'node:assert/strict';
import {spawn} from 'node:child_process';
const child=spawn('target/release/catan-server.exe',['8878','web'],{windowsHide:true,stdio:'ignore'});
const base='http://127.0.0.1:8878/api/';
const api=async(k,b)=>{const r=await fetch(base+k,{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(b)});return {status:r.status,body:await r.json()};};
try {
 let up=false;
 for(let i=0;i<50;i++){try{await fetch('http://127.0.0.1:8878/');up=true;break;}catch{await new Promise(r=>setTimeout(r,100));}}
 assert.ok(up);
 const first=await api('create',{name:'Audit 0',players:3});assert.equal(first.status,200);
 const room=first.body.room;
 const players=[first.body];
 for(let i=1;i<3;i++){const r=await api('join',{room,name:'Audit '+i});assert.equal(r.status,200,JSON.stringify(r.body));players.push(r.body);}
 assert.equal((await api('start',{room,token:first.body.token})).status,200);
 const polls=await Promise.all(players.map(async p=>{const r=await fetch(base+`poll?room=${room}&token=${p.token}&since=0`);return r.json();}));
 const pi=polls.findIndex(d=>d.msgs.some(m=>m.t==='start'&&m.yourSeat===0));assert.ok(pi>=0);
 const start=polls[pi].msgs.find(m=>m.t==='start');assert.equal(start.players,3);assert.deepEqual(start.humans,[true,true,true]);
 const {instance}=await WebAssembly.instantiate(fs.readFileSync('web/catan_wasm.wasm'),{});const w=instance.exports;
 w.game_new(...start.seeds,3,1,0,7);
 const token=players[pi].token, fp=w.fingerprint()>>>0;
 assert.equal((await api('act',{room,token,i:0})).status,409);
 assert.equal((await api('act',{room,token,i:0,fp})).status,200);
 assert.equal(w.apply_index(0),1);const next=w.fingerprint()>>>0;
 assert.notEqual(fp,next);
 assert.equal((await api('act',{room,token,i:0,fp})).status,409);
 assert.equal((await api('act',{room,token,i:-1,fp:next})).status,400);
 assert.equal((await api('act',{room,token,fp:next})).status,400);
 assert.equal((await api('act',{room,token,i:0,fp:next})).status,200);
 assert.equal(w.apply_index(0),1);
 const r=await fetch(base+`poll?room=${room}&token=${token}&since=${polls[pi].seq}`);
 const messages=(await r.json()).msgs.filter(m=>m.t==='act');
 assert.equal(messages.length,2);assert.equal(messages.at(-1).fp,w.fingerprint()>>>0);
 console.log(JSON.stringify({passed:true,checks:10,actualCommittedActions:messages.length}));
} finally { await new Promise(resolve=>{child.once('exit',resolve);child.kill();}); }
