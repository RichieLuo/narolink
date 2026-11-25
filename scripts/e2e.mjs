import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createHmac, randomBytes} from "node:crypto";
import {mkdir, mkdtemp, readFile, rm, writeFile} from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import {fileURLToPath} from "node:url";

const project = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const win = process.platform === "win32";
const exe = path.resolve(process.argv[2] ?? path.join(project,"dist",win ? "NaroLink.exe" : "narolink"));
const relay = path.resolve(process.argv[3] ?? path.join(project,"dist",win ? "narolink-relay.exe" : "narolink-relay-linux-amd64"));
const cli = path.join(project,"test","mock-cli.mjs");
const root = await mkdtemp(path.join(os.tmpdir(),"narolink-e2e-"));
const workspace = path.join(root,"test workspace");
await mkdir(workspace);
const keyDir = path.join(root,"auth");
const config = path.join(root,"claude");
const id = "d637f227-050d-484d-bfa7-d43678be869a";
const log = (uuid,parentUuid,type,text) => JSON.stringify({
  uuid,parentUuid,type,cwd:workspace,sessionId:id,message:{content:text},
});
const projects = path.join(config,"projects","workspace");
await mkdir(projects,{recursive:true});
await writeFile(path.join(projects,id+".jsonl"),[
  log("a",null,"user","Original Claude prompt"),
  log("discarded","a","assistant","Discarded branch"),
  log("b","a","assistant","Existing Claude response"),
].join("\n")+"\n");

async function reservePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const [pcPort, relayPort] = await Promise.all([reservePort(),reservePort()]);
const origin = "http://127.0.0.1:" + pcPort;
const phoneOrigin = "http://127.0.0.1:" + relayPort;
const token = randomBytes(32).toString("hex");
const children = [];
let startError;
function launch(program,args,env) {
  const child = spawn(program,args,{windowsHide:true,stdio:["ignore","ignore","pipe"],env:{...process.env,...env}});
  child.on("error",error => { startError = error; });
  child.stderr.on("data", data => process.stderr.write(data));
  children.push(child);
}
launch(relay,[],{
  NAROLINK_PUBLIC_URL:phoneOrigin,NAROLINK_RELAY_ADDR:"127.0.0.1:"+relayPort,NAROLINK_RELAY_TOKEN:token,
  NAROLINK_TLS_CERT:"",NAROLINK_TLS_KEY:"",
});
launch(exe,["--web-only"],{
  NAROLINK_PORT:String(pcPort),NAROLINK_STATE_DIR:keyDir,NAROLINK_PUBLIC_URL:"",
  NAROLINK_RELAY_URL:phoneOrigin,NAROLINK_RELAY_TOKEN:token,
  NAROLINK_CODEX_CLI:cli,NAROLINK_CLAUDE_CLI:cli,NAROLINK_AGY_CLI:cli,
  NAROLINK_TEST_WORKSPACE:workspace,CLAUDE_CONFIG_DIR:config,
});

const sleep = ms => new Promise(resolve => setTimeout(resolve,ms));
async function waitFor(read, valid, description) {
  for (let n = 0; n < 160; ++n) {
    if (startError) throw startError;
    if (children.some(child => child.exitCode !== null)) throw new Error("A local process closed before " + description);
    try { const result = await read(); if (valid(result)) return result; }
    catch (error) { if (n === 159) throw error; }
    await sleep(75);
  }
  throw new Error("Timed out: " + description);
}
function client(base) {
  let cookie;
  return {
    cookie: () => cookie,
    get: (route) => fetch(base + route,{headers:cookie ? {Cookie:cookie} : {}}),
    post: (route,body,source=base) => fetch(base+route,{
      method:"POST",headers:{Origin:source,"Content-Type":"application/json",...(cookie ? {Cookie:cookie} : {})},
      body:JSON.stringify(body),
    }),
    pair: async code => {
      const response = await ok(await fetch(base+"/api/pair",{
        method:"POST",headers:{Origin:base,"Content-Type":"application/json"},body:JSON.stringify({code}),
      }));
      const setCookie = response.headers.get("set-cookie");
      assert.match(setCookie,/HttpOnly; SameSite=Strict/);
      cookie = setCookie.split(";")[0];
    },
  };
}
async function ok(response) {
  if (!response.ok) throw new Error("HTTP "+response.status+" "+await response.text());
  return response;
}
const json = async response => (await ok(await response)).json();

try {
  const desktop = client(origin);
  const phone = client(phoneOrigin);
  await waitFor(() => json(phone.get("/api/access")), status => status.online,"Go relay handshake");
  assert.equal((await desktop.get("/api/sessions/codex")).status,401);
  assert.equal((await phone.get("/api/sessions/codex")).status,401);
  assert.equal((await phone.post("/relay/poll",{})).status,401);
  assert.equal((await phone.post("/api/pair",{code:"00000000"},"https://foreign.invalid")).status,403);
  const evilHost = await new Promise((resolve,reject) => {
    const socket = net.connect(pcPort,"127.0.0.1",() => socket.end("GET / HTTP/1.1\r\nHost: attacker.invalid\r\nConnection: close\r\n\r\n"));
    socket.once("data",chunk => resolve(chunk.toString()));
    socket.once("error",reject);
  });
  assert.match(evilHost,/421/);
  const key = await readFile(path.join(keyDir,"pairing.key"));
  const slot = Math.floor(Date.now()/300000);
  const code = String(createHmac("sha256",key).update("pair:"+slot).digest().readBigUInt64BE() % 100000000n).padStart(8,"0");
  await desktop.pair(code);
  await phone.pair(code);
  assert.match(phone.cookie(),/^narolink_remote=/);
  assert.equal((await phone.post("/api/runs",{},"https://foreign.invalid")).status,403);
  assert.equal((await json(phone.get("/manifest.webmanifest"))).display,"standalone");
  const fixture = await readFile(path.join(project,"public","icons","180.png"));
  const images = [{name:"Screenshot.png",data:fixture.toString("base64")}];
  assert.equal((await phone.post("/api/runs",{
    provider:"codex",cwd:workspace,prompt:"invalid attachment",images:[{name:"bad.png",data:"invalid base64!"}],
  })).status,400);

  for (const provider of ["codex","claude","antigravity"]) {
    const listing = await json(phone.get("/api/sessions/"+provider));
    assert.equal(listing.sessions.length,1);
    assert.equal(listing.sessions[0].cwd,workspace);
    const endpoint = "/api/sessions/"+provider+"/"+id;
    const native = await json(phone.get(endpoint));
    assert.equal(native.transcript.messages.length,2);
    assert.match(native.transcript.messages[1].text,/Existing/);
    assert.deepEqual((await json(phone.get("/api/models/"+provider))).map(model => model.id),["model-two"]);
    const request = {provider,session_id:id,cwd:"/",model:"model-two",images,
      prompt:provider === "codex" ? "wait until cancelled" : "continue the original session"};
    const task = await json(phone.post("/api/runs",request));
    const runRoute = "/api/runs/"+task.id;
    const inflight = await waitFor(() => json(desktop.get(runRoute)),snapshot =>
      snapshot.session_id===id && snapshot.output.includes("model-two")
        && (provider==="codex" || Boolean(snapshot.approval)), provider+" streaming resume");
    assert.deepEqual(inflight.images,["Screenshot.png"]);
    const image = await ok(await phone.get(runRoute+"/images/0"));
    assert.equal(image.headers.get("content-type"),"image/png");
    assert.deepEqual(Buffer.from(await image.arrayBuffer()).subarray(0,8),fixture.subarray(0,8));
    assert.equal((await phone.get(runRoute+"/images/1")).status,404);
    assert.equal((await json(phone.get("/api/activity"))).running.some(run => run.id===task.id),true);
    assert.equal((await desktop.post("/api/runs",request)).status,400);
    if (provider === "codex") {
      await ok(await phone.post(runRoute+"/stop",{}));
      await waitFor(() => json(desktop.get(runRoute)),snapshot => snapshot.status==="stopped","Codex cancellation");
    } else {
      await ok(await desktop.post(runRoute+"/approval",{approval_id:inflight.approval.id,allow:provider==="antigravity"}));
      assert.equal((await phone.post(runRoute+"/approval",{approval_id:inflight.approval.id,allow:true})).status,409);
      const completed = await waitFor(() => json(phone.get(runRoute)),snapshot => snapshot.status==="complete",provider+" completion");
      assert.match(completed.output,provider==="claude" ? /Model model-two\n\ndeclined/ : /Model model-two approved/);
    }
    const recent = await json(phone.get("/api/activity"));
    assert.ok(recent.finished.some(run => run.id === task.id));
    console.log(provider + ": native history, model and resume with image uploaded through Go relay verified");
  }
} finally {
  await Promise.all(children.map(async child => {
    if (child.exitCode !== null) return;
    const exit = new Promise(resolve => child.once("exit",resolve));
    child.kill();
    await Promise.race([exit,sleep(1500)]);
  }));
  if (path.dirname(path.resolve(root)) === path.resolve(os.tmpdir()) && path.basename(root).startsWith("narolink-e2e-")) {
    await rm(root,{recursive:true,force:true,maxRetries:3,retryDelay:100}).catch(() => {});
  }
}
