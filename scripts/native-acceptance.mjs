// Opt-in real-account acceptance. Uses fresh temporary projects, never existing user sessions.
import assert from 'node:assert/strict';
import {spawn, execFile} from 'node:child_process';
import {promisify} from 'node:util';
import {mkdtemp, mkdir, readFile, writeFile} from 'node:fs/promises';
import {createHmac} from 'node:crypto';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
const exec = promisify(execFile);
const runCli = (program,args,options) => new Promise((resolve,reject) => {
 const child=execFile(program,args,options,(error,stdout,stderr)=>error?reject(Object.assign(error,{stdout,stderr})):resolve({stdout,stderr}));
 child.stdin.end();
});
if (process.env.NARO_REAL_ACCEPTANCE !== '1') throw Error('Set NARO_REAL_ACCEPTANCE=1 to use real CLIs and account tokens');
const root = await mkdtemp(path.join(os.tmpdir(), 'naro-real-acceptance-'));
const state = path.join(root, 'state'); await mkdir(state);
const proxy = process.env.HTTPS_PROXY || 'http://127.0.0.1:7897';
await writeFile(path.join(state, 'cli-proxy'), proxy);
const env = {...process.env, CLAUDECODE:'', HTTP_PROXY:proxy, HTTPS_PROXY:proxy, ALL_PROXY:proxy, NO_PROXY:'localhost,127.0.0.1'};
const listener=net.createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
const origin=`http://127.0.0.1:${port}`;
const app=spawn(path.resolve(process.argv[2] || 'target/debug/narolink'), ['--web-only'], {env:{...env,NAROLINK_PORT:String(port),NAROLINK_STATE_DIR:state,NAROLINK_RELAY_URL:'',NAROLINK_RELAY_TOKEN:''},stdio:['ignore','ignore','pipe']});
let cookie='';const report={root,engines:{}};
async function api(route, body) {
 const response=await fetch(origin+'/api'+route,{headers:{Cookie:cookie,Origin:origin,...(body===undefined?{}:{'Content-Type':'application/json'})},method:body===undefined?'GET':'POST',body:body===undefined?undefined:JSON.stringify(body)});
 const text=await response.text(); const data=text ? JSON.parse(text) : null; if(!response.ok)throw Error(`${route}: ${response.status} ${text}`);return data;
}
const delay=ms=>new Promise(r=>setTimeout(r,ms));
try {
 for(let i=0;i<60;i++){try{await api('/access');break}catch{await delay(200)}}
 const key=await readFile(path.join(state,'pairing.key'));const digest=createHmac('sha256',key).update(`pair:${Math.floor(Date.now()/300000)}`).digest();
 const code=String(Number(digest.readBigUInt64BE(0)%100000000n)).padStart(8,'0');
 const paired=await fetch(origin+'/api/pair',{method:'POST',headers:{Origin:origin,'Content-Type':'application/json'},body:JSON.stringify({code})});assert.equal(paired.status,200);cookie=paired.headers.get('set-cookie').split(';')[0];
 await Promise.all((process.env.NARO_ACCEPTANCE_PROVIDERS || 'claude,codex,antigravity').split(',').map(async provider=>{
  const cwd=path.join(root,provider);await mkdir(cwd);await writeFile(path.join(cwd,'sample.txt'),'before\n');
  await exec('git',['init','-q',cwd]);await exec('git',['-C',cwd,'-c','user.name=Naro test','-c','user.email=naro-test@localhost','add','sample.txt']);await exec('git',['-C',cwd,'-c','user.name=Naro test','-c','user.email=naro-test@localhost','commit','-qm','Isolated acceptance fixture']);
  const result=report.engines[provider]={cwd};
  try {
   const prompt='Reply exactly NARO_NATIVE_SEED. Do not use any tools.';
   let program,args;
   if(provider==='claude'){program='/Users/richie/.local/bin/claude';args=['-p',prompt,'--output-format','json','--permission-mode','manual'];}
   else if(provider==='codex'){program='/Users/richie/.local/bin/codex';args=['exec','-C',cwd,'--json','-s','read-only',prompt];}
   else{program='/Users/richie/.local/bin/agy';args=['--print',prompt,'--output-format','stream-json','--print-timeout','90s'];}
   const seed=await runCli(program,args,{cwd,env,timeout:120000,maxBuffer:8*1024*1024});await writeFile(path.join(root,provider+'-seed.jsonl'),seed.stdout);result.seed=true;
   const inventory=await api('/sessions/'+provider+'?refresh=true');const session=inventory.sessions.find(row=>path.resolve(row.cwd).replace('/private/var/','/var/')===cwd.replace('/private/var/','/var/'));
   assert.ok(session,'Naro did not discover native CLI session');result.session_id=session.id;result.discovered=true;
   const run=await api('/runs',{provider,session_id:session.id,cwd,permission_mode:provider==='claude'?'manual':provider==='codex'?'request-approval':'default',collaboration_mode:provider==='codex' && process.env.NARO_ACCEPTANCE_PLAN==='1' ? 'plan' : null, prompt:process.env.NARO_ACCEPTANCE_PROMPT || 'Use your native file edit tool to replace sample.txt with exactly "after\\n". Do not use shell commands. Ask for any required permission. Then reply NARO_NATIVE_RESUMED.'});
   result.run_id=run.id;let finished=false;
   for(let i=0;i<240;i++){
    const snap=await api('/runs/'+run.id);
    if(snap.question){const q=snap.question;const answers={};for(const field of q.questions){const option=field.options.find(o=>/allow_once|\[yes\]|^accept$/.test(o.label))||field.options[0];answers[field.id]=[option.label];}await api('/runs/'+run.id+'/answer',{question_id:q.id,answers});result.native_question=true;}
    if(snap.approval){await api('/runs/'+run.id+'/approval',{approval_id:snap.approval.id,allow:true});result.native_approval=true;}
    if(snap.status!=='running'){result.status=snap.status;result.error=snap.error;result.output=snap.output;result.replies=snap.replies;finished=true;break;}
    await delay(500);
   }
   if(!finished){await api('/runs/'+run.id+'/stop',{});throw Error('Real resumed turn timed out');}
   result.file=await readFile(path.join(cwd,'sample.txt'),'utf8');
   const history=await api('/sessions/'+provider+'/'+session.id+'?refresh=true');result.history_messages=history.transcript?.messages.length;result.history_changes=history.transcript?.messages.flatMap(m=>m.work?.changes||[]);
  } catch(e){result.failure=e.message;}
  console.log(provider,JSON.stringify({session_id:result.session_id,status:result.status,failure:result.failure,approval:result.native_approval,question:result.native_question,file:result.file}));
 }));
} finally {app.kill();await writeFile(path.join(root,'report.json'),JSON.stringify(report,null,2));console.log('REPORT',path.join(root,'report.json'));}
