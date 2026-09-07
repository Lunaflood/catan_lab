from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import subprocess,sys,time
sys.stdout.reconfigure(encoding='utf-8')
root=Path(__file__).resolve().parents[2]
out=Path(__file__).resolve().parent
sets=['production=600','enemy_production=-40','army=24','hand_synergy=45','hand_synergy=180','trade_cost=10','buildable_nodes=400','afford_city=1.2']
def run(setting):
 args=[str(root/'target/audit/release/catan.exe'),'trial','--set',setting,'--games','480','--seed','850000','--no-time-limit'];t=time.time();p=subprocess.run(args,cwd=root,capture_output=True,text=True,encoding='utf-8',errors='replace');s=p.stdout+p.stderr;(out/('weights-'+setting.replace('=','_')+'.txt')).write_text(' '.join(args)+'\n'+s,encoding='utf-8');print(setting,p.returncode,round(time.time()-t,1),s,flush=True)
with ThreadPoolExecutor(max_workers=8) as ex:list(ex.map(run,sets))
