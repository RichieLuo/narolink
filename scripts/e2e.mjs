import assert from "node:assert/strict";
import {spawn} from "node:child_process";
import {createHash, createHmac, randomBytes, randomUUID} from "node:crypto";
import {mkdir, mkdtemp, readFile, rm, writeFile} from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import {fileURLToPath} from "node:url";

const project = path.join(path.dirname(fileURLToPath(import.meta.url)), "..");
const win = process.platform === "win32";
const exe = path.resolve(process.argv[2] ?? path.join(project,"dist",win ? "Naro.exe" : "narolink"));
const relay = path.resolve(process.argv[3] ?? path.join(project,"dist",win ? "narolink-relay.exe" : "narolink-relay-linux-amd64"));
const cli = path.join(project,"test","mock-cli.mjs");
const root = await mkdtemp(path.join(os.tmpdir(),"narolink-e2e-"));
const workspace = path.join(root,"test workspace");
const sharedWorkspace = path.join(root,"shared workspace");
const rollout = path.join(workspace,"mock-rollout.jsonl");
const docs = path.join(workspace,"docs");
const readmePath = path.join(workspace,"README.md");
const readme = "# 查看 Naro\n\n[English](docs/README%20en.md)\n\n<script>unsafe code</script>\n";
await Promise.all([mkdir(docs,{recursive:true}),mkdir(sharedWorkspace)]);
const fixture = await readFile(path.join(project,"public","icons","180.png"));
await Promise.all([
  writeFile(rollout,"original history\n"),
  writeFile(path.join(workspace,"preview file.png"),fixture),
  writeFile(path.join(root,"outside.png"),fixture),
  writeFile(readmePath,readme),
  writeFile(path.join(docs,"README en.md"),"# Naro in English\n\n[License](../LICENSE)\n"),
  writeFile(path.join(workspace,"LICENSE"),"Example license\n"),
  writeFile(path.join(root,"outside.txt"),"Outside the session directory\n"),
]);
const keyDir = path.join(root,"auth");
const proxy = "http://127.0.0.1:18080/";
await mkdir(keyDir);
await writeFile(path.join(keyDir,"cli-proxy"),proxy);
const config = path.join(root,"claude");
const id = "d637f227-050d-484d-bfa7-d43678be869a";
const newId = "d637f227-050d-484d-bfa7-d43678be869b";
const projectId = "d637f227-050d-484d-bfa7-d43678be869c";
const otherProjectId = "d637f227-050d-484d-bfa7-d43678be869d";
const sharedSessionId = "d637f227-050d-484d-bfa7-d43678be869e";
const assignedSessionId = "d637f227-050d-484d-bfa7-d43678be869f";
const untitledSessionId = "d637f227-050d-484d-bfa7-d43678be8692";
const log = (uuid,parentUuid,type,text,model) => JSON.stringify({
  uuid,parentUuid,type,cwd:workspace,sessionId:id,message:{content:text,model},
});
const projects = path.join(config,"projects","workspace");
await mkdir(projects,{recursive:true});
await writeFile(path.join(projects,id+".jsonl"),[
  log("a",null,"user",[
    {type:"text",text:"Original Claude prompt"},
    {type:"image",source:{type:"base64",media_type:"image/png",data:fixture.toString("base64")}},
  ]),
  log("discarded","a","assistant","Discarded branch","model-discarded"),
  log("b","a","assistant",[
    {type:"thinking",thinking:"Looking up the relevant files",signature:"private signature"},
    {type:"redacted_thinking",data:"private encrypted thinking"},
    {type:"text",text:"Existing Claude response"},
    {type:"tool_use",id:"history-read",name:"Read",input:{file_path:"src/main.rs"}},
    {type:"tool_use",id:"history-agent",name:"Agent",input:{subagent_type:"Explore",description:"Find affected files"}},
  ],"model-history"),
  log("c","b","user",[{type:"tool_result",tool_use_id:"history-read",content:"private file contents"}]),
  JSON.stringify({...JSON.parse(log("agent-result","c","user",[
    {type:"tool_result",tool_use_id:"history-agent",content:"private async output"},
  ])),toolUseResult:{agentId:"background-1",status:"async_launched",isAsync:true}}),
  JSON.stringify({uuid:"agent-notice",parentUuid:"agent-result",type:"attachment",attachment:{
    type:"queued_command",origin:{kind:"task-notification"},
    prompt:"<task-notification><task-id>background-1</task-id><status>failed</status><summary>private agent summary</summary></task-notification>",
  }}),
  log("d","agent-notice","assistant",[{type:"text",text:"History tool finished"}],"model-history"),
].join("\n")+"\n");

async function reservePort() {
  const server = net.createServer();
  await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}
const [pcPort, relayPort, otherPort] = await Promise.all([reservePort(),reservePort(),reservePort()]);
const origin = "http://127.0.0.1:" + pcPort;
const phoneOrigin = "http://127.0.0.1:" + relayPort;
const otherRoot = path.join(root,"other-device");
const otherWorkspace = path.join(otherRoot,"workspace");
const otherKeyDir = path.join(otherRoot,"auth");
await mkdir(otherWorkspace,{recursive:true});
await mkdir(otherKeyDir);
await writeFile(path.join(otherWorkspace,"mock-rollout.jsonl"),"Other computer history\n");
await writeFile(path.join(otherKeyDir,"cli-proxy"),proxy);
const codexHome = path.join(root,"codex");
const codexSessions = path.join(codexHome,"sessions");
await mkdir(codexSessions,{recursive:true});
const timestamp = new Date(Date.now() - 60_000).toISOString();
await Promise.all([
  writeFile(path.join(codexSessions,"rollout-usage.jsonl"),JSON.stringify({
    timestamp, type:"token_usage_record",
    payload:{response_id:"mock-response",usage:{
      input_tokens:140, output_tokens:55, cached_input_tokens:100, cache_write_input_tokens:5,
    }},
  })+"\n"),
  writeFile(path.join(projects,"usage.jsonl"),JSON.stringify({
    timestamp, type:"assistant",
    message:{id:"mock-message",usage:{
      input_tokens:30, output_tokens:10, cache_read_input_tokens:20, cache_creation_input_tokens:15,
    }},
  })+"\n"),
]);
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
const desktopEnv = {
  NAROLINK_PORT:String(pcPort),NAROLINK_STATE_DIR:keyDir,NAROLINK_PUBLIC_URL:"",
  NAROLINK_RELAY_URL:phoneOrigin,NAROLINK_RELAY_TOKEN:token,
  NAROLINK_DEVICE_NAME:"第一台电脑",
  NAROLINK_CODEX_CLI:cli,NAROLINK_CLAUDE_CLI:cli,NAROLINK_AGY_CLI:cli,
  NAROLINK_TEST_WORKSPACE:workspace,NAROLINK_TEST_PROXY:proxy,CLAUDE_CONFIG_DIR:config,
  CODEX_HOME:codexHome,ANTIGRAVITY_DATA_DIR:path.join(root,"antigravity"),
};
launch(exe,["--web-only"],desktopEnv);
launch(exe,["--web-only"],{
  ...desktopEnv,NAROLINK_PORT:String(otherPort),NAROLINK_STATE_DIR:otherKeyDir,NAROLINK_TEST_WORKSPACE:otherWorkspace,
  NAROLINK_DEVICE_NAME:"第二台电脑",CLAUDE_CONFIG_DIR:path.join(otherRoot,"claude"),
  CODEX_HOME:path.join(otherRoot,"codex"),
});

const sleep = ms => new Promise(resolve => setTimeout(resolve,ms));
async function waitFor(read, valid, description, timeoutMs = 12000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (startError) throw startError;
    if (children.some(child => child.exitCode !== null)) throw new Error("A local process closed before " + description);
    try { const result = await read(); if (valid(result)) return result; }
    catch (error) { if (Date.now() >= deadline) throw error; }
    await sleep(75);
  }
  throw new Error("Timed out: " + description);
}
function client(base, scope = "") {
  let cookie;
  const url = route => base + (route.startsWith("/api/") ? scope : "") + route;
  return {
    cookie: () => cookie,
    get: (route, {headers, ...options} = {}) => fetch(url(route),{
      ...options,headers:{...(cookie ? {Cookie:cookie} : {}), ...headers},
    }),
    post: (route,body,source=base) => fetch(url(route),{
      method:"POST",headers:{Origin:source,"Content-Type":"application/json",...(cookie ? {Cookie:cookie} : {})},
      body:JSON.stringify(body),
    }),
    pair: async code => {
      const response = await ok(await fetch(url("/api/pair"),{
        method:"POST",headers:{Origin:base,"Content-Type":"application/json"},body:JSON.stringify({code}),
      }));
      const setCookie = response.headers.get("set-cookie");
      assert.match(setCookie,/HttpOnly; SameSite=Strict/);
      if (scope) assert.match(setCookie,new RegExp("Path="+scope+"/;"));
      cookie = setCookie.split(";")[0];
      return response.json();
    },
  };
}
async function ok(response) {
  if (!response.ok) throw new Error("HTTP "+response.status+" "+await response.text());
  return response;
}
const json = async response => (await ok(await response)).json();

function fileLink(html, label) {
  const match = html.match(new RegExp(`href="(/api/file-previews/[a-f0-9]{64})">${label}</a>`));
  assert.ok(match, `Expected a file preview link for ${label}`);
  return match[1];
}

async function replyEvents(target, route, signal) {
  const response = await ok(await target.get(route+"/events",{headers:{Accept:"text/event-stream"},signal}));
  assert.match(response.headers.get("content-type"),/^text\/event-stream/);
  const reader = response.body.pipeThrough(new TextDecoderStream()).getReader();
  let buffered = "";
  return async (type, matches) => {
    for (;;) {
      const end = buffered.indexOf("\n\n");
      if (end === -1) {
        const {value, done} = await reader.read();
        assert.equal(done,false,"Stream closed before "+type);
        buffered += value;
        continue;
      }
      const frame = buffered.slice(0,end);
      buffered = buffered.slice(end+2);
      const lines = frame.split("\n");
      if (!lines.includes("event: "+type)) continue;
      const data = JSON.parse(lines.filter(line => line.startsWith("data: ")).map(line => line.slice(6)).join("\n"));
      if (matches(data)) return data;
    }
  };
}

async function pairingDetails(directory) {
  const key = await waitFor(() => readFile(path.join(directory,"pairing.key")), value => value.length === 32,"pairing key");
  const deviceKey = createHmac("sha256",key).update("relay-device-key:v1").digest();
  const id = createHash("sha256").update(deviceKey).digest("hex").slice(0,32);
  const slot = Math.floor(Date.now()/300000);
  const code = String(createHmac("sha256",key).update("pair:"+slot).digest().readBigUInt64BE() % 100000000n).padStart(8,"0");
  return {scope:"/devices/"+id,code};
}

try {
  const desktop = client(origin);
  const [first, second] = await Promise.all([pairingDetails(keyDir),pairingDetails(otherKeyDir)]);
  const {code} = first;
  const phone = client(phoneOrigin,first.scope);
  const anonymous = client(phoneOrigin,first.scope);
  const other = client(phoneOrigin,second.scope);
  await Promise.all([phone,other].map(target =>
    waitFor(() => json(target.get("/api/access")), status => status.online,"Go relay handshake")));
  assert.deepEqual(await json(client(phoneOrigin).get("/api/access")),{paired:false,online:false});
  assert.match(await (await ok(await fetch(phoneOrigin+first.scope+"/"))).text(),/设备列表/);
  assert.equal((await desktop.get("/api/sessions/codex")).status,401);
  assert.equal((await phone.get("/api/sessions/codex")).status,401);
  const permissions = "/api/permissions/codex";
  const sessionPermission = sessionId => permissions + "?session_id=" + sessionId;
  assert.equal((await phone.get(permissions)).status,401);
  assert.equal((await desktop.get("/api/quotas/codex")).status,401);
  assert.equal((await phone.get("/api/quotas/codex")).status,401);
  assert.equal((await desktop.get("/api/usage")).status,401);
  assert.equal((await phone.get("/api/usage")).status,401);
  const resetPath = "/api/quotas/codex/reset";
  const resetRequest = {idempotency_key:randomUUID()};
  assert.equal((await desktop.post(resetPath,resetRequest)).status,401);
  assert.equal((await phone.post(resetPath,resetRequest)).status,401);
  assert.equal((await phone.post("/relay/poll",{})).status,401);
  assert.equal((await phone.post("/api/pair",{code:"00000000"},"https://foreign.invalid")).status,403);
  const evilHost = await new Promise((resolve,reject) => {
    const socket = net.connect(pcPort,"127.0.0.1",() => socket.end("GET / HTTP/1.1\r\nHost: attacker.invalid\r\nConnection: close\r\n\r\n"));
    socket.once("data",chunk => resolve(chunk.toString()));
    socket.once("error",reject);
  });
  assert.match(evilHost,/421/);
  await desktop.pair(code);
  assert.equal((await phone.pair(code)).name,"第一台电脑");
  assert.equal((await other.pair(second.code)).name,"第二台电脑");
  assert.equal(await json(phone.get(permissions)),null);
  await ok(await phone.post(permissions,{session_id:id,permission_mode:"full-access"}));
  await ok(await desktop.post(permissions,{session_id:sharedSessionId,permission_mode:"approve-for-me"}));
  assert.equal(await json(desktop.get(sessionPermission(id))),"full-access");
  assert.equal(await json(phone.get(sessionPermission(sharedSessionId))),"approve-for-me");
  assert.equal(await json(phone.get(permissions)),"approve-for-me");
  assert.equal(await json(phone.get("/api/permissions/claude")),null);
  assert.equal(await json(other.get(permissions)),null);
  assert.equal((await phone.post(permissions,{session_id:id,permission_mode:"acceptEdits"})).status,400);
  assert.match(phone.cookie(),/^narolink_device=/);
  const crossDevice = await fetch(phoneOrigin+second.scope+"/api/sessions/codex",{headers:{Cookie:phone.cookie()}});
  assert.equal(crossDevice.status,401);
  const usageResponse = await ok(await phone.get("/api/usage"));
  assert.equal(usageResponse.headers.get("cache-control"),"no-store");
  const usage = await usageResponse.json();
  assert.deepEqual(usage.windows.map(window => window.period),["24h","7d","30d"]);
  assert.deepEqual(usage.windows[0].total,{
    input_tokens:205, output_tokens:65, cache_read_tokens:120, cache_write_tokens:20,
  });
  assert.deepEqual(usage.windows[0].providers.map(item => [item.provider,item.counts?.input_tokens ?? null]),[
    ["codex",140], ["claude",65], ["antigravity",null],
  ]);
  assert.deepEqual(await json(desktop.get("/api/usage")),usage);
  assert.ok((await json(other.get("/api/usage"))).windows.every(window => window.total === null));
  console.log("token usage: stored counters, pairing, and device isolation verified");
  const quotaResponse = await ok(await phone.get("/api/quotas/codex"));
  assert.equal(quotaResponse.headers.get("cache-control"),"no-store");
  const quotas = await quotaResponse.json();
  assert.deepEqual(quotas,{
    limits:[
      {name:"通用额度",windows:[
        {label:"5 小时",remaining_percent:65,resets_at:null},
        {label:"7 天",remaining_percent:87,resets_at:1796154084},
      ]},
      {name:"Other model",windows:[{label:"7 天",remaining_percent:0,resets_at:1796154084}]},
    ],
    credit_balance:"10.5",unlimited_credits:false,reset_credits:2,
  });
  assert.deepEqual(await json(desktop.get("/api/quotas/codex")),quotas);
  const otherQuotas = await json(other.get("/api/quotas/codex"));
  assert.deepEqual(otherQuotas.limits,[
    {name:"通用额度",windows:[{label:"7 天",remaining_percent:25,resets_at:1796154084}]},
  ]);
  assert.equal((await phone.post(resetPath,resetRequest,"https://foreign.invalid")).status,403);
  assert.equal((await phone.post(resetPath,{idempotency_key:"00000000-0000-0000-0000-000000000000"})).status,400);
  assert.equal((await json(phone.post(resetPath,resetRequest))).outcome,"reset");
  assert.equal((await json(desktop.post(resetPath,resetRequest))).outcome,"alreadyRedeemed");
  assert.equal((await json(phone.post(resetPath,{idempotency_key:randomUUID()}))).outcome,"nothingToReset");
  assert.equal((await json(other.post(resetPath,{idempotency_key:randomUUID()}))).outcome,"noCredit");
  const afterReset = await json(phone.get("/api/quotas/codex"));
  assert.equal(afterReset.reset_credits,1);
  assert.deepEqual(afterReset.limits[0].windows.map(window => window.remaining_percent),[100,100]);
  const claudeQuota = await json(phone.get("/api/quotas/claude"));
  assert.deepEqual(claudeQuota.limits,[{name:"套餐额度",windows:[
    {label:"5 小时",remaining_percent:68,resets_at:1796154084},
    {label:"7 天",remaining_percent:90,resets_at:null},
    {label:"Fable · 7 天",remaining_percent:60,resets_at:null},
  ]}]);
  const unsupportedClaude = await other.get("/api/quotas/claude");
  assert.equal(unsupportedClaude.status,400);
  assert.match((await unsupportedClaude.json()).error,/登录方式未提供套餐额度/);
  const antigravityQuota = await json(phone.get("/api/quotas/antigravity"));
  assert.deepEqual(antigravityQuota.limits,[{name:"Gemini Models",windows:[
    {label:"7 天",remaining_percent:83,resets_at:1796154084},
    {label:"5 小时",remaining_percent:100,resets_at:null},
    {label:"Credits",remaining_percent:null,remaining_amount:12.5,resets_at:null},
  ]}]);
  const olderAntigravity = await other.get("/api/quotas/antigravity");
  assert.equal(olderAntigravity.status,400);
  assert.match((await olderAntigravity.json()).error,/1\.1\.11/);
  console.log("quotas: all three providers, paired requests and device isolation verified");
  const otherSessions = await json(other.get("/api/sessions/codex"));
  assert.equal(otherSessions.sessions.find(row => row.id === id).cwd,otherWorkspace);
  assert.equal((await phone.post("/api/runs",{},"https://foreign.invalid")).status,403);
  assert.equal((await json(phone.get("/manifest.webmanifest"))).display,"standalone");
  for (const client of [desktop, phone]) {
    const catalog = await json(client.get("/i18n.json"));
    assert.deepEqual(catalog["界面语言"],["Interface language","表示言語"]);
    assert.deepEqual(catalog["深色"],["Dark","ダーク"]);
    const theme = await client.get("/theme.js");
    assert.equal(theme.status,200);
    assert.match(theme.headers.get("content-type"),/javascript/);
    assert.match(await theme.text(),/prefers-color-scheme: dark/);
  }
  const images = [{name:"Screenshot.png",data:fixture.toString("base64")}];
  assert.equal((await phone.post("/api/runs",{
    provider:"codex",cwd:workspace,prompt:"invalid attachment",images:[{name:"bad.png",data:"invalid base64!"}],
  })).status,400);

  for (const provider of ["codex","claude","antigravity"]) {
    const listing = await json(phone.get("/api/sessions/"+provider));
    assert.equal(listing.sessions.length,provider === "codex" ? 3 : 1);
    assert.equal(listing.sessions[0].cwd,workspace);
    if (provider === "codex") {
      assert.deepEqual(listing.projects,[
        {id:projectId,name:"Native Project",roots:[workspace,sharedWorkspace]},
        {id:otherProjectId,name:"Other Project",roots:[root,sharedWorkspace]},
      ]);
      assert.equal(listing.sessions[0].project_id,projectId);
      assert.equal(listing.sessions.find(row => row.id === sharedSessionId).project_id,undefined);
      assert.equal(listing.sessions.find(row => row.id === assignedSessionId).project_id,otherProjectId);
    }
    const endpoint = "/api/sessions/"+provider+"/"+id;
    const native = await json(phone.get(endpoint));
    assert.equal(native.transcript.messages.length,2);
    assert.match(native.transcript.messages[1].text,/Existing/);
    assert.equal(native.transcript.model ?? native.session.model,"model-history");
    const agents = native.transcript.messages[1].work.agents;
    assert.deepEqual(agents.map(({name,description,status}) => [name,description,status]),{
      codex:[["search",undefined,"complete"],[undefined,"Check compatibility","failed"]],
      claude:[["Explore","Find affected files","failed"]],
      antigravity:[["reviewer","Review the changes","started"]],
    }[provider]);
    if (provider !== "codex") {
      const prompt = native.transcript.messages[0];
      assert.equal(prompt.text,provider === "claude" ? "Original Claude prompt" : "Original Antigravity prompt");
      assert.equal(prompt.attachments.length,1);
      const attachment = prompt.attachments[0];
      assert.match(attachment.url,/^\/api\/previews\//);
      assert.equal((await anonymous.get(attachment.url)).status,401);
      const image = await ok(await phone.get(attachment.url));
      assert.equal(image.headers.get("content-type"),"image/png");
      assert.deepEqual(Buffer.from(await image.arrayBuffer()).subarray(0,8),fixture.subarray(0,8));
    }
    if (provider === "claude") {
      assert.equal(native.transcript.messages[1].text,"Existing Claude response\n\nHistory tool finished");
      assert.deepEqual(native.transcript.messages[1].work.steps.map(step => [step.kind,step.status,step.text]),[
        ["thinking","complete","Looking up the relevant files"],["tool","complete","Read · src/main.rs"],
      ]);
      assert.doesNotMatch(JSON.stringify(native.transcript),/private/);
    }
    if (provider === "antigravity") {
      assert.deepEqual(native.transcript.messages[1].work.steps.map(step => [step.kind,step.status,step.text]),[
        ["thinking","complete","Reviewing the saved Antigravity turn"],
      ]);
    }
    if (provider === "codex") {
      const prompt = native.transcript.messages[0];
      assert.equal(prompt.text,"Original Codex prompt");
      assert.deepEqual(prompt.local_files,[{name:"notes.md",directory:false},{name:"assets",directory:true}]);
      assert.deepEqual(prompt.attachments.map(image => image.name),["照片 1.png","失效图片.png","图片 3"]);
      assert.equal(prompt.attachments[1].url,undefined);
      assert.doesNotMatch(JSON.stringify(prompt),/private-attachments|outside\.png|missing-screenshot|data:image/);
      for (const attachment of [prompt.attachments[0], prompt.attachments[2]]) {
        assert.match(attachment.url,/^\/api\/previews\//);
        assert.equal((await anonymous.get(attachment.url)).status,401);
        const image = await ok(await phone.get(attachment.url));
        assert.equal(image.headers.get("content-type"),"image/png");
        assert.deepEqual(Buffer.from(await image.arrayBuffer()).subarray(0,8),fixture.subarray(0,8));
      }
      const reply = native.transcript.messages[1];
      assert.equal(reply.turn_id,"original-turn");
      assert.deepEqual(reply.work.steps.map(step => step.text),[
        "First progress update","**Inspecting the repository**\n\nComparing `saved results` [Link](javascript:invalid)","Second progress update",
      ]);
      assert.doesNotMatch(JSON.stringify(reply.work),/raw reasoning/);
      const summaryHtml = reply.rendered.work_steps["stored-summary"];
      assert.match(summaryHtml,/<strong>Inspecting the repository<\/strong>/);
      assert.match(summaryHtml,/<code>saved results<\/code>/);
      assert.doesNotMatch(summaryHtml,/javascript:|<a\b/);
      assert.deepEqual(reply.work.changes.map(file => [file.path,file.added,file.removed]),[["example.js",2,1]]);
      assert.match(reply.rendered.html,/href="https:\/\/example\.com\/"/);
      assert.doesNotMatch(reply.rendered.html,/javascript:|href="[^\"]+"[^>]*>Outside/);
      assert.equal(reply.rendered.previews.length,1);
      const preview = reply.rendered.previews[0];
      assert.equal(preview.name,"preview file.png");
      assert.equal((await anonymous.get(preview.url)).status,401);
      const asset = await ok(await phone.get(preview.url));
      assert.equal(asset.headers.get("content-type"),"image/png");
      assert.deepEqual(Buffer.from(await asset.arrayBuffer()).subarray(0,8),fixture.subarray(0,8));
      const readmeLink = fileLink(reply.rendered.html,"README");
      assert.equal(fileLink(reply.rendered.html,"README relative"), readmeLink);
      assert.doesNotMatch(reply.rendered.html,/href="[^"]+"[^>]*>Outside text/);
      assert.equal((await anonymous.get(readmeLink)).status,401);
      assert.equal((await other.get(readmeLink)).status,404);
      const readmeResponse = await ok(await phone.get(readmeLink));
      assert.equal(readmeResponse.headers.get("cache-control"),"no-store");
      const document = await readmeResponse.json();
      assert.equal(document.name,"README.md");
      assert.equal(document.text,readme);
      assert.match(document.html,/<h1>查看 Naro<\/h1>/);
      assert.doesNotMatch(document.html,/<script>/);
      assert.deepEqual(await json(desktop.get(readmeLink)),document);
      const english = await json(phone.get(fileLink(document.html,"English")));
      assert.equal(english.name,"README en.md");
      const license = await json(phone.get(fileLink(english.html,"License")));
      assert.deepEqual(license,{name:"LICENSE",text:"Example license\n"});
      await writeFile(readmePath,readme+"\nUpdated file\n");
      assert.equal((await json(phone.get(readmeLink))).text,readme+"\nUpdated file\n");
      const watch = endpoint + "?since=" + encodeURIComponent(native.transcript.revision);
      assert.ok(native.transcript.revision);
      assert.equal((await json(phone.get(watch))).transcript,null);
      await writeFile(rollout,"original history\nexternal update\n");
      const changed = (await json(phone.get(watch))).transcript.messages;
      const external = changed.at(-1);
      assert.match(changed.at(-2).text,/^# Files mentioned by the user:[\s\S]*External prompt$/);
      assert.equal(external.turn_id,"external-turn");
      assert.equal(external.text,"");
      assert.deepEqual(external.work.steps.map(step => [step.kind,step.text]),[
        ["note","External progress update"],["command","git status --short"],
      ]);
      await writeFile(rollout,"original history\nsame-turn follow-up\n");
      const steeredHistory = (await json(phone.get(endpoint + "?refresh=true"))).transcript.messages;
      assert.deepEqual(steeredHistory.map(entry => entry.role),["user","assistant","user","assistant"]);
      assert.equal(steeredHistory[1].text,"Earlier answer");
      assert.equal(steeredHistory[1].work.steps.at(-1).text,"Second progress update");
      assert.equal(steeredHistory[2].text,"Follow-up prompt\n");
      assert.equal(steeredHistory[3].turn_id,steeredHistory[1].turn_id);
      assert.deepEqual(steeredHistory[3].work.steps.map(step => step.kind),["note","command"]);
      assert.deepEqual(steeredHistory[1].work.agents,agents);
      assert.equal(steeredHistory[3].work.agents,undefined);
      await writeFile(rollout,"original history\n");
    }
    const [model] = await json(phone.get("/api/models/"+provider));
    assert.equal(model.id, "model-two");
    assert.deepEqual(model.reasoning_efforts.find(effort => effort.id === "high"),
      {id: "high", label: provider === "antigravity" ? "High" : "high"});
    assert.equal(model.fast_mode, provider !== "antigravity");
    assert.equal(model.default_reasoning_effort,provider === "antigravity" ? "medium" : "high");
    assert.equal(model.default_fast_mode,{codex:true,claude:false,antigravity:undefined}[provider]);
    const catalog = await json(phone.get("/api/models/"+provider+"?settings=true"));
    assert.equal(catalog.models[0].id,model.id);
    assert.equal(catalog.default_model,"model-two");
    assert.equal(catalog.models.filter(entry => entry.id === "model-two").length,1);
    assert.ok(!catalog.models.some(entry => entry.id === "default"));
    const permissionMode = {codex:null, claude:"acceptEdits", antigravity:"accept-edits"}[provider];
    const permissionIds = catalog.permission_modes.map(mode => mode.id);
    assert.equal(catalog.approval_policies,undefined);
    if (provider === "codex") {
      assert.equal(catalog.models.find(entry => entry.id === "model-listed-default").default_reasoning_effort,"low");
      assert.deepEqual(permissionIds,["request-approval","approve-for-me","full-access","custom"]);
      assert.deepEqual(catalog.collaboration_modes,["plan","default"]);
      assert.equal(catalog.default_permission_mode,"request-approval");
      const managedPermissions = path.join(root,"managed-permissions");
      await writeFile(managedPermissions,"");
      const restricted = await json(phone.get("/api/models/codex?settings=true&refresh=true"));
      assert.deepEqual(restricted.permission_modes.map(mode => mode.id),["request-approval","custom"]);
      await rm(managedPermissions);
    } else if (provider === "claude") {
      assert.deepEqual(permissionIds,["manual","acceptEdits","plan","dontAsk","auto","bypassPermissions"]);
    } else {
      assert.deepEqual(permissionIds,["default","accept-edits","auto","plan"]);
    }
    const request = {provider,session_id:id,cwd:"/",model:"model-two",images,
      reasoning_effort:"high",fast_mode:model.fast_mode ? true : null,permission_mode:permissionMode,
      prompt:provider === "codex" ? "wait until cancelled" : "continue the original session"};
    const task = await json(phone.post("/api/runs",request));
    const runRoute = "/api/runs/"+task.id;
    if (provider === "codex") assert.equal((await other.get(runRoute)).status,404);
    const inflight = await waitFor(() => json(desktop.get(runRoute)),snapshot =>
      snapshot.session_id===id && (provider === "codex"
        ? snapshot.replies?.[0]?.text.includes("model-two")
        : snapshot.output.includes("model-two") && Boolean(snapshot.approval || snapshot.question)), provider+" streaming resume");
    assert.equal(inflight.model,"model-two");
    assert.deepEqual(inflight.images,["Screenshot.png"]);
    const image = await ok(await phone.get(runRoute+"/images/0"));
    assert.equal(image.headers.get("content-type"),"image/png");
    assert.deepEqual(Buffer.from(await image.arrayBuffer()).subarray(0,8),fixture.subarray(0,8));
    assert.equal((await phone.get(runRoute+"/images/1")).status,404);
    assert.equal((await json(phone.get("/api/activity"))).running.some(run => run.id===task.id),true);
    assert.equal((await desktop.post("/api/runs",request)).status,400);
    if (provider === "codex") {
      assert.equal(inflight.replies.length,1);
      assert.deepEqual(inflight.replies[0].work.steps.map(step => [step.kind,step.status,step.text]),[
        ["reasoning","complete","Checking the repository\n\nReviewing the results"],
        ["note","complete","A progress update"],
        ["command","running","git status --short"],
      ]);
      const eventsPath = runRoute+"/events";
      const headers = {Accept:"text/event-stream"};
      assert.equal((await anonymous.get(eventsPath,{headers})).status,401);
      assert.equal((await other.get(eventsPath,{headers})).status,404);
      const initial = snapshot => snapshot.status === "running" && snapshot.replies[0]?.text === "Model model-two";
      const controller = new AbortController();
      const signal = AbortSignal.any([AbortSignal.timeout(10000),controller.signal]);
      const localEvents = await replyEvents(desktop,runRoute,signal);
      const streams = await Promise.all(Array.from({length:4},() => replyEvents(phone,runRoute,signal)));
      await Promise.all([localEvents,...streams].map(events => events("snapshot",initial)));
      await writeFile(path.join(root,"codex-stream-next"),"");
      await Promise.all([localEvents,...streams].map(async events => {
        const delta = await events("delta",() => true);
        assert.equal(delta.after,null);
        assert.equal(delta.text," · 第二段🙂");
      }));
      const reconnectController = new AbortController();
      const reconnected = await replyEvents(phone,runRoute,AbortSignal.any([signal,reconnectController.signal]));
      await reconnected("snapshot", snapshot => snapshot.replies[0]?.text === "Model model-two · 第二段🙂");
      reconnectController.abort();
      await ok(await phone.post(runRoute+"/messages",{mode:"queue",prompt:"stop before this"}));
      await ok(await phone.post(runRoute+"/stop",{}));
      const snapshots = await Promise.all(streams.map(events => events("snapshot", snapshot => snapshot.status === "stopped")));
      assert.ok(snapshots.every(snapshot => snapshot.follow_ups[0].delivery === "unsent"));
      controller.abort();
      console.log("reply stream: UTF-8 deltas, reconnect, pairing and concurrent control requests verified");
      const stopped = await waitFor(() => json(desktop.get(runRoute)),snapshot => snapshot.status==="stopped","Codex cancellation");
      assert.equal(stopped.follow_ups[0].delivery,"unsent");
      assert.equal(stopped.replies[0].work.steps.at(-1).status,"interrupted");
      assert.equal(stopped.replies[0].work.agents[0].status,"interrupted");
      assert.equal((await json(phone.get(endpoint))).session.project_id,projectId);
    } else {
      if (provider === "claude") {
        assert.deepEqual(inflight.replies[0].work.steps.map(step => [step.kind,step.status,step.text]),[
          ["thinking","complete","Choosing a command"],["command","complete","git status --short"],
          ["tool","running","Edit · sample.txt"],
        ]);
        assert.equal(inflight.activity,"正在使用工具");
        assert.doesNotMatch(JSON.stringify(inflight.replies),/private/);
      } else {
        assert.deepEqual(inflight.replies[0].work.steps.map(step => [step.kind,step.status,step.text]),[
          ["thinking","complete","Considering the requested change"],
        ]);
      }
      if (inflight.question) {
        const question = inflight.question;
        const label = question.questions[0].options.find(option => option.label.includes("[yes]")).label;
        await ok(await phone.post(runRoute+"/answer",{question_id:question.id,answers:{decision:[label]}}));
        assert.equal((await desktop.post(runRoute+"/answer",{question_id:question.id,answers:{decision:[label]}})).status,409);
      } else {
        await ok(await desktop.post(runRoute+"/approval",{approval_id:inflight.approval.id,allow:false}));
        assert.equal((await phone.post(runRoute+"/approval",{approval_id:inflight.approval.id,allow:true})).status,409);
      }
      const completed = await waitFor(() => json(phone.get(runRoute)),snapshot => snapshot.status==="complete",provider+" completion");
      assert.match(completed.output,provider==="claude" ? /Model model-two\n\ndeclined/ : /Model model-two approved/);
      if (provider === "claude") {
        assert.deepEqual(completed.replies[0].work.steps.map(step => step.status),["complete","complete","failed"]);
      }
    }
    const recent = await json(phone.get("/api/activity"));
    assert.ok(recent.finished.some(run => run.id === task.id));
    console.log(provider + ": native history, model options and resume with image uploaded through Go relay verified");
  }

  const fileRun = await json(phone.post("/api/runs",{
    provider:"codex",session_id:id,cwd:workspace,model:"model-two",reasoning_effort:"high",
    fast_mode:true,prompt:"open the readme",images,
  }));
  const fileRunRoute = "/api/runs/"+fileRun.id;
  const liveFileEvents = await replyEvents(phone,fileRunRoute,AbortSignal.timeout(10000));
  const fileTurn = await liveFileEvents("snapshot", snap => snap.status === "complete");
  const fileFromReply = await json(phone.get(fileLink(fileTurn.replies[0].rendered.html,"README")));
  assert.equal(fileFromReply.text,readme+"\nUpdated file\n");
  console.log("file preview: history and live replies, local and paired access, Markdown links, and reload verified");

  for (const [provider, settings, answers] of [
    ["codex",{collaboration_mode:"plan",fast_mode:true},{color:["Orange"],notes:["Sample note"]}],
    ["claude",{permission_mode:"acceptEdits",fast_mode:true},{"Which color?":["Orange"],"Which snacks?":["Apple","Bread"]}],
    ["antigravity",{permission_mode:"accept-edits"},{q1:["Orange"],q2:["Apple","Cheese"]}],
  ]) {
    const task = await json(desktop.post("/api/runs",{provider,session_id:id,cwd:workspace,model:"model-two",
      reasoning_effort:"high",images,prompt:"interactive questions",...settings}));
    const route = "/api/runs/"+task.id;
    const pending = await waitFor(() => json(phone.get(route)),snapshot => snapshot.question,provider+" questions");
    const {id:questionId,questions} = pending.question;
    assert.ok(!pending.approval);
    assert.equal(questions[0].text,"Which color?");
    assert.deepEqual(questions[0].options.map(option => option.label),["Orange","Blue"]);
    assert.equal(questions[0].multiple,false);
    assert.equal(questions[1].multiple,provider !== "codex");
    assert.equal(questions[1].allow_custom,true);
    assert.equal(questions[1].secret,provider === "codex");
    assert.deepEqual((await json(desktop.get(route))).question,pending.question);
    assert.equal((await phone.post(route+"/answer",{question_id:questionId,answers:{}},"https://foreign.invalid")).status,403);
    assert.equal((await phone.post(route+"/answer",{question_id:questionId,answers:{...answers,[questions[0].id]:[""]}})).status,409);
    assert.equal((await json(phone.get(route))).question.id,questionId);
    const reply = {question_id:questionId,answers};
    const responses = await Promise.all([phone.post(route+"/answer",reply),desktop.post(route+"/answer",reply)]);
    assert.deepEqual(responses.map(response => response.status).sort(),[204,409]);
    const completed = await waitFor(() => json(phone.get(route)),snapshot => snapshot.status !== "running",provider+" continuation");
    assert.equal(completed.status,"complete",completed.error);
    assert.equal(completed.output,"Answers received");
    assert.equal(completed.question,undefined);
    console.log(provider+": structured questions and single answer across devices verified");
  }

  assert.equal((await phone.post("/api/runs",{
    provider:"claude",cwd:workspace,prompt:"invalid approval policy",approval_policy:"on-request",
  })).status,400);

  const claudePermissions = "/api/permissions/claude";
  await ok(await desktop.post(claudePermissions, {session_id:id, permission_mode:"bypassPermissions"}));
  const claudePlan = await json(phone.post("/api/runs", {
    provider:"claude", session_id:id, cwd:workspace, permission_mode:"bypassPermissions", prompt:"observe Claude plan mode",
  }));
  const claudePlanRoute = "/api/runs/" + claudePlan.id;
  const planSearch = await waitFor(() => json(phone.get(claudePlanRoute)), snap => snap.approval, "Claude plan search approval");
  assert.equal(planSearch.approval.title, "Bash");
  assert.equal(planSearch.active_permission_mode?.id, "plan");
  assert.equal(await json(phone.get(claudePermissions + "?session_id=" + id)), "bypassPermissions");
  await ok(await phone.post(claudePlanRoute + "/approval", {approval_id:planSearch.approval.id, allow:true}));
  const planReview = await waitFor(() => json(desktop.get(claudePlanRoute)), snap => snap.approval?.title === "批准计划并实施", "Claude plan approval");
  assert.equal(planReview.active_permission_mode?.id, "plan");
  await ok(await desktop.post(claudePlanRoute + "/approval", {approval_id:planReview.approval.id, allow:true}));
  const planComplete = await waitFor(() => json(phone.get(claudePlanRoute)), snap => snap.status !== "running", "Claude plan completion");
  assert.equal(planComplete.status, "complete", planComplete.error);
  assert.equal(planComplete.active_permission_mode?.id, "bypassPermissions");
  assert.equal(planComplete.output, "Plan approved");
  console.log("claude: active plan mode and native restoration of bypass permissions verified");

  for (const [mode, prompt, title, details, allow] of [
    ["request-approval", "check network permission", "Codex 请求联网", /example\.com:443/, true],
    ["request-approval", "check network permission", "Codex 请求联网", /example\.com:443/, false],
    ["read-only", "check additional permission", "Codex 申请额外权限", /"enabled": true/, true],
    ["approve-for-me", "check automatic approval"],
    ["full-access", "check full access"],
    ["custom", "check custom permission", "Codex 申请额外权限", /"enabled": true/, true],
  ]) {
    const codexSummaryConfig = path.join(root,"codex-summary-disabled");
    if (mode === "custom") await writeFile(codexSummaryConfig,"");
    const run = await json(phone.post("/api/runs",{
      provider:"codex",session_id:id,cwd:workspace,model:"model-two",images,
      reasoning_effort:"high",fast_mode:true,permission_mode:mode,
      approval_policy:mode === "read-only" ? "on-request" : undefined,
      prompt,
    }));
    const route = "/api/runs/"+run.id;
    if (title) {
      const requested = await waitFor(() => json(desktop.get(route)),snapshot => snapshot.approval || snapshot.question,
        "Codex approval in " + mode);
      if (requested.question) {
        const question = requested.question;
        assert.match(question.questions[0].text,details);
        assert.ok(question.questions[0].options.some(option => option.label === "acceptForSession"));
        await ok(await phone.post(route+"/answer",{question_id:question.id,answers:{decision:[allow ? "accept" : "decline"]}}));
      } else {
        assert.equal(requested.approval.title,title);
        assert.match(requested.approval.details,details);
        await ok(await phone.post(route+"/approval",{approval_id:requested.approval.id,allow}));
      }
    }
    const completed = await waitFor(() => json(desktop.get(route)),snapshot => snapshot.status !== "running",
      "Codex completion in " + mode);
    assert.equal(completed.status,"complete",completed.error);
    assert.equal(completed.output,title ? "Network permission " + (allow ? "accept" : "decline") : "Model model-two confirmed");
    if (mode === "custom") await rm(codexSummaryConfig);
  }
  console.log("codex: four permission options, managed restrictions and legacy permission parameters verified");

  const followUpRun = await json(desktop.post("/api/runs",{
    provider:"codex",session_id:id,cwd:workspace,model:"model-two",reasoning_effort:"high",
    fast_mode:true,prompt:"wait for steering",images,
  }));
  const followUpRoute = "/api/runs/"+followUpRun.id;
  const delegating = await waitFor(() => json(phone.get(followUpRoute)),
    run => run.replies[0]?.work.agents?.length === 2,"two running Codex agents");
  assert.deepEqual(delegating.replies[0].work.agents.map(agent => agent.status),["running","running"]);
  await ok(await phone.post(followUpRoute+"/messages",{
    mode:"queue",prompt:"queued first",images,
    options:{model:"model-next",reasoning_effort:"medium",fast_mode:false,permission_mode:"read-only"},
  }));
  await ok(await desktop.post(followUpRoute+"/messages",{mode:"queue",prompt:"withdraw this draft",
    images:images.map(image => ({...image,display_name:"original screenshot.jpg"}))}));
  await ok(await phone.post(followUpRoute+"/messages",{mode:"queue",prompt:"change direction",images}));
  const pending = await json(phone.get(followUpRoute));
  assert.deepEqual(pending.follow_ups.map(entry => entry.delivery),["pending","pending","pending"]);
  const originalDraft = pending.follow_ups[1];
  const promoted = pending.follow_ups[2];
  const withdrawRoute = followUpRoute+"/messages/"+originalDraft.id+"/withdraw";
  const draft = await json(phone.post(withdrawRoute,{}));
  assert.equal(draft.prompt,"withdraw this draft");
  assert.equal(draft.images[0].name,"original screenshot.jpg");
  assert.equal((await desktop.post(withdrawRoute,{})).status,409);
  assert.equal((await desktop.get(followUpRoute+"/images/"+originalDraft.image_start)).status,404);
  await ok(await phone.get(followUpRoute+"/images/"+promoted.image_start));
  const remaining = await json(desktop.get(followUpRoute));
  assert.deepEqual(remaining.follow_ups.map(entry => entry.prompt),["queued first","change direction"]);
  await ok(await phone.post(followUpRoute+"/messages",{mode:"queue",prompt:"queued second, edited",
    images:draft.images.map(image => ({name:"image.png",display_name:image.name,data:image.data}))}));
  await ok(await phone.post(followUpRoute+"/messages",{mode:"steer",prompt:"reject steering"}));
  await waitFor(() => json(desktop.get(followUpRoute)),run =>
    run.follow_ups.some(entry => entry.delivery === "unsent" && /rejected/.test(entry.error)),
    "Codex reports a rejected steering message");
  await ok(await phone.post(followUpRoute+"/messages",{mode:"steer",prompt:"steer while queued",images}));
  await waitFor(() => json(desktop.get(followUpRoute)),run =>
    run.follow_ups.some(entry => entry.output.includes("Steered · steer while queued")),
    "new steering message is delivered while the queue waits");
  const promoteRoute = followUpRoute+"/messages/"+promoted.id+"/steer";
  await ok(await desktop.post(promoteRoute,{}));
  await ok(await phone.post(promoteRoute,{}));
  const followed = await waitFor(() => json(phone.get(followUpRoute)),run => run.status !== "running",
    "steering and both queued messages complete");
  assert.equal(followed.status,"complete",followed.error);
  assert.equal(followed.session_id,id);
  assert.equal(followed.model,"model-next");
  assert.deepEqual(followed.replies[0].work.agents.map(agent => agent.status),["complete","interrupted"]);
  assert.ok(followed.replies.slice(1).every(reply => !reply.work.agents));
  assert.match(followed.output,/Model model-two confirmed/);
  assert.deepEqual(followed.follow_ups.map(entry => [entry.prompt,entry.delivery]),[
    ["steer while queued","sent"],["change direction","sent"],["queued first","sent"],
    ["queued second, edited","sent"],["reject steering","unsent"],
  ]);
  assert.equal(followed.follow_ups[1].id,promoted.id);
  assert.equal(followed.follow_ups[1].mode,"steer");
  assert.match(followed.follow_ups[1].output,/Steered · change direction/);
  assert.match(followed.follow_ups[2].output,/Model model-next confirmed · queued first/);
  assert.match(followed.follow_ups[3].output,/Model model-next confirmed · queued second, edited/);
  assert.deepEqual(followed.follow_ups[3].images,["original screenshot.jpg"]);
  assert.equal(followed.replies.length,5);
  assert.match(followed.replies[0].text,/Model model-two confirmed/);
  assert.equal(followed.replies[0].after,null);
  assert.equal(followed.replies[1].text,"Steered · steer while queued");
  assert.equal(followed.replies[1].after,followed.follow_ups[0].id);
  assert.equal(followed.replies[2].text,"Steered · change direction");
  assert.equal(followed.replies[2].after,followed.follow_ups[1].id);
  assert.equal((await phone.post(followUpRoute+"/messages/"+pending.follow_ups[0].id+"/steer",{})).status,409);
  assert.equal((await phone.post(followUpRoute+"/messages/"+pending.follow_ups[0].id+"/withdraw",{})).status,409);
  const followUpImage = await ok(await phone.get(followUpRoute+"/images/"+followed.follow_ups[1].image_start));
  assert.equal(followUpImage.headers.get("content-type"),"image/png");
  console.log("codex: shared FIFO queue, withdrawal and resend with images, and steering verified");

  for (const [provider, permissionMode, expectReject] of [
    ["antigravity", "accept-edits", true],
    ["claude", "acceptEdits", false],
  ]) {
    const run = await json(desktop.post("/api/runs",{
      provider,session_id:id,cwd:workspace,model:"model-two",reasoning_effort:"high",
      fast_mode:provider === "claude" ? true : null,permission_mode:permissionMode,
      prompt:"wait for steering",images,
    }));
    const route = "/api/runs/"+run.id;
    await waitFor(() => json(phone.get(route)),snap => snap.output.includes("confirmed"),provider+" ready to steer");
    await ok(await phone.post(route+"/messages",{mode:"queue",prompt:"queued after steer",images}));
    await ok(await desktop.post(route+"/messages",{mode:"queue",prompt:"change direction",images}));
    const queued = await json(phone.get(route));
    const promotedItem = queued.follow_ups[1];
    if (expectReject) {
      await ok(await phone.post(route+"/messages",{mode:"steer",prompt:"reject steering"}));
      await waitFor(() => json(desktop.get(route)),snap =>
        snap.follow_ups.some(entry => entry.delivery === "unsent" && /rejected/.test(entry.error)),
        provider+" reports a rejected steering message");
    }
    await ok(await phone.post(route+"/messages",{mode:"steer",prompt:"steer while queued",images}));
    await waitFor(() => json(desktop.get(route)),snap =>
      snap.follow_ups.some(entry => entry.delivery === "sent" && entry.output.includes("Steered · steer while queued")),
      provider+" direct steering delivered while queue waits");
    await ok(await desktop.post(route+"/messages/"+promotedItem.id+"/steer",{}));
    const finished = await waitFor(() => json(phone.get(route)),snap => snap.status !== "running",
      provider+" steering and queued follow-up complete");
    assert.equal(finished.status,"complete",finished.error);
    assert.equal(finished.session_id,id);
    assert.deepEqual(
      finished.follow_ups.filter(entry => entry.delivery === "sent").map(entry => [entry.prompt,entry.mode]),
      [["steer while queued","steer"],["change direction","steer"],["queued after steer","queue"]],
    );
    assert.equal(finished.replies.length,4);
    assert.match(finished.replies[0].text,/Model model-two confirmed/);
    assert.equal(finished.replies[0].after,null);
    assert.equal(finished.replies[1].text,"Steered · steer while queued");
    assert.equal(finished.replies[1].after,finished.follow_ups[0].id);
    assert.equal(finished.replies[2].text,"Steered · change direction");
    assert.equal(finished.replies[2].after,promotedItem.id);
    assert.match(finished.replies[3].text,/queued after steer/);
    if (provider === "antigravity") {
      assert.deepEqual(finished.replies[0].work.agents,[{
        id:"delegation",name:"tester",description:"Review the test coverage",status:"started",
      }]);
      assert.deepEqual(finished.replies[0].work.steps.filter(step => step.kind === "thinking").map(step => step.text),["Reviewing the original request"]);
      assert.deepEqual(finished.replies[3].work.steps.map(step => step.text),["Thinking about queued after steer"]);
    }
    console.log(provider+": mid-turn steering, queued promotion, and follow-up queue verified");
  }

  const newRequest = {provider:"codex",project_id:projectId,cwd:workspace,model:"model-two",
    reasoning_effort:"high",fast_mode:true,prompt:"start a conversation in the project",images,
    permission_mode:await json(phone.get(permissions))};
  const invalid = await json(phone.post("/api/runs",{...newRequest,cwd:root}));
  const rejected = await waitFor(() => json(phone.get("/api/runs/"+invalid.id)),run =>
    run.status === "failed","reject a workspace outside the selected project");
  assert.equal(rejected.session_id,null);
  assert.match(rejected.error,/重新选择项目/);
  const task = await json(phone.post("/api/runs",newRequest));
  const runPath = "/api/runs/"+task.id;
  assert.equal(await json(phone.get(permissions+"?run_id="+task.id)),"approve-for-me");
  await ok(await phone.post(permissions,{run_id:task.id,permission_mode:"custom"}));
  const result = await waitFor(() => json(phone.get(runPath)),run =>
    run.status !== "running","slow Codex project startup",30000);
  assert.equal(result.status,"complete",result.error);
  assert.equal(result.session_id,newId);
  assert.equal(await json(desktop.get(sessionPermission(newId))),"custom");
  assert.equal(result.title,newRequest.prompt);
  await writeFile(path.join(root,"codex-title-ready"),"");
  const created = await json(phone.get("/api/sessions/codex/"+newId));
  assert.equal(created.session.project_id,projectId);
  const updatedListing = await json(phone.get("/api/sessions/codex"));
  assert.equal(updatedListing.sessions.find(row => row.id === newId).project_id,
    updatedListing.sessions.find(row => row.id === id).project_id);
  assert.match(result.output,/Model model-two confirmed/);

  const titleCases = [
    ["codex", newRequest.prompt, "Start a Project Conversation"],
    ["claude", "改进图片预览的加载速度", "优化图片预览速度"],
    ["antigravity", "整理 Antigravity 的会话列表", "整理会话列表"],
  ];
  for (const [provider, prompt, title] of titleCases) {
    const run = provider === "codex" ? task :
      await json(phone.post("/api/runs",{provider,cwd:workspace,model:"model-two",prompt}));
    const completed = await waitFor(() => json(phone.get("/api/runs/"+run.id)), snapshot =>
      snapshot.status === "complete" && snapshot.title === title, provider+" generated title");
    assert.equal(completed.session_id,newId);
    assert.equal(completed.prompt,prompt);
    const listingPath = "/api/sessions/"+provider;
    const listing = await json(phone.get(listingPath));
    assert.equal(listing.sessions.find(row => row.id === newId).title,title);
    assert.equal(listing.sessions.find(row => row.id === id).title, {
      codex:"Native Codex History",claude:"Original Claude prompt [图片]",antigravity:"Native Antigravity History",
    }[provider]);
    assert.equal(listing.sessions.length,provider === "codex" ? 4 : 2);
    const history = await json(desktop.get(listingPath+"/"+newId));
    assert.equal(history.session.title,title);
    assert.ok(history.transcript.messages.every(row => !row.text.includes('"first_message"')));
  }
  const activity = await json(phone.get("/api/activity"));
  assert.equal(activity.title_revision,3);
  assert.ok(activity.finished.some(run => run.id === task.id && run.title === titleCases[0][2]));

  const fallbackPrompt = "继续整理设置页面";
  const fallback = await json(phone.post("/api/runs",{
    provider:"claude",cwd:workspace,model:"model-two",prompt:fallbackPrompt,
  }));
  const withoutTitle = await waitFor(() => json(phone.get("/api/runs/"+fallback.id)),
    snapshot => snapshot.status === "complete", "chat with an invalid model title");
  assert.equal(withoutTitle.title,fallbackPrompt);

  const reloadPort = await reservePort();
  launch(exe,["--web-only"],{
    ...desktopEnv,NAROLINK_PORT:String(reloadPort),NAROLINK_RELAY_URL:"",NAROLINK_RELAY_TOKEN:"",
  });
  const reloaded = client("http://127.0.0.1:"+reloadPort);
  await waitFor(() => reloaded.get("/api/access"),response => response.ok,"independent desktop process");
  await reloaded.pair(code);
  assert.equal(await json(reloaded.get(sessionPermission(id))),"full-access");
  assert.equal(await json(reloaded.get(sessionPermission(newId))),"custom");
  assert.equal(await json(reloaded.get(permissions)),"custom");
  for (const [provider, , title] of titleCases) {
    const history = await json(reloaded.get("/api/sessions/"+provider+"/"+newId));
    assert.equal(history.session.title,title);
  }
  assert.equal((await json(reloaded.get("/api/sessions/claude/"+untitledSessionId))).session.title,fallbackPrompt);
  await writeFile(path.join(root,"codex-created"),"Manually Named Project Session");
  await ok(await reloaded.get("/api/sessions/codex?refresh=true"));
  assert.equal((await json(reloaded.get("/api/sessions/codex/"+newId))).session.title,"Manually Named Project Session");
  const logout = await ok(await other.post("/api/logout",{}));
  assert.match(logout.headers.get("set-cookie"),new RegExp("Path="+second.scope+"/;.*Max-Age=0"));
  assert.equal((await json(phone.get("/api/access"))).paired,true);
  console.log("devices: simultaneous registration, independent pairing, session routing and logout verified");
  console.log("codex: project assignment and slow session startup through Go relay verified");
  console.log("permissions: per-session selection, engine default, device isolation, and persistence verified");
  console.log("titles: all three engines, delayed completion, shared views, persistence and fallback verified");
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
