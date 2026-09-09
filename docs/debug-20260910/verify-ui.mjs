import fs from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const {chromium} = require('C:/Users/payji/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/playwright');
const browser = await chromium.launch({headless:true,executablePath:'C:/Users/payji/AppData/Local/ms-playwright/chromium_headless_shell-1217/chrome-headless-shell-win64/chrome-headless-shell.exe'});
const page = await browser.newPage({viewport:{width:1280,height:720}});
const errors=[]; page.on('pageerror',e=>errors.push(e.message));
await page.route('**/colonist/app.js*',async route=>{
 const src=fs.readFileSync('web/colonist/app.js','utf8');
 await route.fulfill({contentType:'text/javascript',body:src+`
window.__audit = {
 ready:()=>!!wasm,
 prepare:(seat=0,n=4)=>{
  clearTimeout(botTimer); net.on=false; watching=false; mySeat=seat;
  let s, count=0;
  search: for(let attempt=0;attempt<40;attempt++){
  wasm.game_new(42+attempt,2+attempt,3,4,n,1<<seat,0,1<<seat);
  count=0; let rand=1337;
  for(;count<3000;count++){
   s=readJson(wasm.state_json());
   if(s.prompt==='MOVE_ROBBER' && s.toAct===seat && s.actions.some(a=>a.victim!==undefined && s.actions.filter(b=>b.tile===a.tile && b.victim!==undefined).length>=2)) break search;
   rand=(Math.imul(rand,1664525)+1013904223)>>>0;
   const preferred=s.actions.filter(a=>['BUILD_CITY','BUILD_SETTLEMENT','BUILD_ROAD','ROLL'].includes(a.kind));
   const list=preferred.length?preferred:s.actions;
   if(!list.length) break;
   wasm.apply_index(list[rand%list.length].i);
  }
  }
  if(!s.actions.some(a=>a.victim!==undefined)) throw Error('No fixture');
  board=readJson(wasm.board_json()); refreshState();
  document.getElementById('home').style.display='none'; document.getElementById('app').hidden=false;
  drawBoard(); render();
  return {state:s,board,count};
 }, stale:()=>{devPick={kind:0,action:"PLAY_KNIGHT"};refreshState();render();}, zoom:(scale)=>{boardView.scale=scale;boardView.dx=15;boardView.dy=-12;applyBoardView();}, select:selectTile, current:()=>readJson(wasm.state_json())
};`});
});
await page.goto('http://127.0.0.1:8877/colonist/');
await page.waitForFunction(()=>window.__audit?.ready());

const results=[];
// A stale development-card confirmation must not hide the robber picker.
await page.evaluate(()=>__audit.prepare(0,4));
for(const size of [[1280,720],[1920,1080],[1024,768],[800,600]]) {
 await page.setViewportSize({width:size[0],height:size[1]});
 for(const n of [3,4]) for(let seat=0;seat<n;seat++) {
  const fixture=await page.evaluate(({seat,n})=>__audit.prepare(seat,n),{seat,n});
  const a=fixture.state.actions.find(a=>a.victim!==undefined && fixture.state.actions.filter(b=>b.tile===a.tile && b.victim!==undefined).length>=2);
  await page.evaluate(()=>__audit.zoom(1.4));
  const unique=[...new Set(fixture.state.actions.map(a=>a.tile))];
  let status='ok';
  try {
   await page.evaluate(()=>__audit.stale());
   await page.locator('.tilehint').nth(unique.indexOf(a.tile)).click({timeout:1200});
   if(await page.locator('.tilehint.chosen').getAttribute('data-tile')!==String(a.tile)) throw Error('Selected tile highlight mismatch');
   await page.getByText('場所を選び直す',{exact:true}).click();
   if(await page.locator('.vcard').count()) throw Error('Victim selection was not cleared');
   await page.locator('.tilehint').nth(unique.indexOf(a.tile)).click({timeout:1200});
   const shown=await page.locator('.vcard:not([disabled])').count();
   if(shown<2) throw Error('Missing adjacent victim');
   const victimButton=page.locator('.vcard:not([disabled])').nth(seat%2);
   const victim=Number(await victimButton.getAttribute('data-victim'));
   if(size[0]===1280 && n===4 && seat===0) await page.screenshot({path:'docs/debug-20260910/robber-picker.jpg',type:'jpeg',quality:55});
   await victimButton.click({timeout:1200});
   const after=await page.evaluate(()=>__audit.current());
   if(after.robber!==a.tile || after.players[seat].handSize!==fixture.state.players[seat].handSize+1) throw Error('No steal');
   if(after.players[victim].handSize!==fixture.state.players[victim].handSize-1) throw Error('Wrong victim');
  } catch(e){status=e.message;}
  results.push({size,n,seat,tile:a.tile,status});
 }
}
fs.writeFileSync('docs/debug-20260910/ui-final-matrix.json',JSON.stringify({errors,results},null,2));
console.log(JSON.stringify({errors,results},null,2));
await page.locator('#r-settings').click();
const downloadPromise=page.waitForEvent('download');
await page.locator('#sb-debug').click();
const download=await downloadPromise;
await download.saveAs('docs/debug-20260910/export-test.json');
const exported=JSON.parse(fs.readFileSync('docs/debug-20260910/export-test.json','utf8'));
if(exported.schema!==1 || !exported.state || 'token' in exported || 'net' in exported) throw Error('Invalid diagnostic export');
if(results.some(r=>r.status!=='ok') || errors.length) throw Error('UI regression failed');
await browser.close();
