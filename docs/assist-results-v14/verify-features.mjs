import fs from 'node:fs';
import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const {chromium}=require('C:/Users/payji/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const browser=await chromium.launch({headless:true,executablePath:'C:/Users/payji/AppData/Local/ms-playwright/chromium_headless_shell-1217/chrome-headless-shell-win64/chrome-headless-shell.exe'});
const page=await browser.newPage({viewport:{width:1280,height:720}});
const errors=[];page.on('pageerror',e=>errors.push(e.message));
await page.route('**/colonist/app.js*',async route=>route.fulfill({contentType:'text/javascript',body:fs.readFileSync('web/colonist/app.js','utf8')+`
window.__test={
 ready:()=>!!wasm,
 setup:(n=4,seat=0)=>{
  clearTimeout(botTimer);closeAssist(false);net.on=false;watching=false;mySeat=seat;
  assistSetup={boardSeed:42,diceSeed:2,devSeed:3,stealSeed:4,players:n,mySeat:seat,askLast:1<<seat};
  gameSetup={...assistSetup,humansMask:1<<seat,levelBits:0,names:['あなた','カイ','ミナ','ソラ'],levels:[0,0,0,0]};
  seatNames=gameSetup.names.slice();seatLevel=gameSetup.levels.slice();journal=[];
  wasm.game_new(42,2,3,4,n,1<<seat,0,1<<seat);
  while(!wasm.is_human_turn()){wasm.bot_step();noteMove({i:wasm.last_action_index()});}
  board=readJson(wasm.board_json());refreshState();
  document.getElementById('home').style.display='none';document.getElementById('app').hidden=false;drawBoard();render();
  return {fp:wasm.fingerprint(),journal:journal.length};
 },
 fixture:(fixture,online=false)=>{
  closeAssist(false);clearTimeout(botTimer);watching=false;net.on=online;
  const su=fixture.setup;mySeat=su.mySeat;assistSetup=su;seatNames=['あなた','カイ','ミナ','ソラ'];seatLevel=[0,0,0,0];
  gameSetup=online?null:{...su,humansMask:1<<mySeat,levelBits:0,names:seatNames.slice(),levels:seatLevel.slice()};
  wasm.game_new(su.boardSeed,su.diceSeed,su.devSeed,su.stealSeed,su.players,1<<mySeat,0,su.askLast);
  journal=[];netBulk=true;
  for(const move of fixture.journal) {
   if(online) netApply(move);else {if(!replayOne(move))throw Error('Bad fixture');journal.push(move);}
  }
  netBulk=false;board=readJson(wasm.board_json());refreshState();drawBoard();render();
  return {fp:wasm.fingerprint(),journal:journal.length,prompt:state.prompt};
 },
 restore:()=>{saveGame();closeAssist(false);wasm.game_new(1,2,3,4,4,1,0,1);return restoreGame();},
 current:()=>({fp:wasm.fingerprint(),advice:assistAdvice,state:readJson(wasm.state_json()),journal:journal.length}),
 advance:()=>{const a=assistAdvice;wasm.apply_index(a.i);noteMove({i:a.i});refreshState();render();},
 finish:(n)=>{
  closeAssist(false);clearTimeout(botTimer);net.on=false;watching=false;mySeat=0;
  wasm.game_new(142,29,33,44,n,0,0,0);journal=[];resultKey=null;
  let steps=0;while(wasm.bot_step() && steps++<12000){}
  board=readJson(wasm.board_json());refreshState();drawBoard();render();
  return {steps,state:readJson(wasm.state_json()),result:readJson(wasm.result_json())};
 }
};` }));
const results=[];
try {
 await page.goto('http://127.0.0.1:8877/colonist/');await page.waitForFunction(()=>__test.ready());
 for(const [n,seat] of [[3,0],[3,2],[4,1],[4,3]]) {
  const before=await page.evaluate(({n,seat})=>__test.setup(n,seat),{n,seat});
  await page.locator('#r-assist').click();
  await page.waitForSelector('#assist-body h3',{timeout:35000});
  const after=await page.evaluate(()=>__test.current());
  assert.equal(after.fp,before.fp);assert.equal(after.journal,before.journal);
  assert.ok(after.state.actions.some(a=>a.i===after.advice.i));
  assert.equal(await page.locator('.assist-mark').count(),1);
  if(n===4&&seat===3) await page.screenshot({path:'docs/assist-results-v14/assist.jpg',type:'jpeg',quality:65});
  await page.evaluate(()=>__test.advance());
  assert.equal(await page.locator('.assist-mark').count(),0);
  assert.match(await page.locator('#assist-body').innerText(),/局面が変わりました/);
  await page.locator('#assist-refresh').click();await page.waitForSelector('#assist-body h3',{timeout:35000});
  assert.equal((await page.evaluate(()=>__test.current())).advice.kind,'SETUP_ROAD');
  await page.locator('#assist-close').click();
  results.push({test:'advice',n,seat,passed:true});
 }
 const fixtures=JSON.parse(fs.readFileSync('docs/assist-results-v14/phase-fixtures.json','utf8'));
 for(const [phase,fixture] of Object.entries(fixtures)) {
  const online=phase==='DECIDE_TRADE';
  const before=await page.evaluate(({fixture,online})=>__test.fixture(fixture,online),{fixture,online});
  await page.locator('#r-assist').click();await page.waitForSelector('#assist-body h3',{timeout:35000});
  const after=await page.evaluate(()=>__test.current());
  assert.equal(after.fp,before.fp);assert.equal(after.journal,before.journal);
  assert.ok(after.state.actions.some(a=>a.i===after.advice.i));
  if(phase==='PLAY_TURN') {
   assert.equal(await page.evaluate(()=>__test.restore()),true);
   await page.locator('#r-assist').click();await page.waitForSelector('#assist-body h3',{timeout:35000});
   assert.equal((await page.evaluate(()=>__test.current())).fp,before.fp);
  }
  await page.locator('#assist-close').click();
  results.push({test:'phase-advice',phase,online,passed:true});
 }
 for(const n of [3,4]) {
  const game=await page.evaluate(n=>__test.finish(n),n);
  assert.notEqual(game.state.winner,null);assert.ok(game.result.timeline.length>10);
  assert.deepEqual(game.result.timeline.at(-1).points,game.state.players.map(p=>p.vp));
  assert.deepEqual(game.result.rollCounts,game.state.rollCounts);
  for(const [width,height] of [[1280,720],[800,600],[1920,1080]]) {
   await page.setViewportSize({width,height});
   assert.equal(await page.locator('.vp-line').count(),n);
   assert.equal(await page.locator('.dice-chart rect').count(),11);
   await page.locator('#vp-turn').fill('0');
   assert.match(await page.locator('#vp-readout').innerText(),/初期配置/);
   const max=await page.locator('#vp-turn').getAttribute('max');await page.locator('#vp-turn').fill(max);
   const readout=await page.locator('#vp-readout').innerText();assert.ok(readout.includes(game.state.players[0].vp+'点'));
   await page.locator('#resultagain').scrollIntoViewIfNeeded();
   const box=await page.locator('#resultagain').boundingBox();assert.ok(box.y>=0 && box.y+box.height<=height+1);
   const card=await page.locator('#result .rcard').boundingBox();assert.ok(card.x>=0 && card.x+card.width<=width+1);
   if(n===4&&width===1280) {
    await page.locator('#result .rcard').evaluate(e=>e.scrollTop=0);
    await page.screenshot({path:'docs/assist-results-v14/result.jpg',type:'jpeg',quality:65});
    await page.getByRole('button',{name:'出目分布',exact:true}).click();
    await page.locator('.dice-chart').scrollIntoViewIfNeeded();
    await page.screenshot({path:'docs/assist-results-v14/distribution.jpg',type:'jpeg',quality:65});
   }
   results.push({test:'result',n,width,height,passed:true});
  }
 }
 assert.deepEqual(errors,[]);
 fs.writeFileSync('docs/assist-results-v14/browser-results.json',JSON.stringify({errors,results},null,2));
 console.log(JSON.stringify({errors,results},null,2));
} finally {await browser.close();}
