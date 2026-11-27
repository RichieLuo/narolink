import assert from "node:assert/strict";
import {mkdir, readFile} from "node:fs/promises";
import {createServer} from "node:http";
import {fileURLToPath, pathToFileURL} from "node:url";
import path from "node:path";
const {chromium, webkit} = await import(process.env.PLAYWRIGHT_MODULE ? pathToFileURL(path.resolve(process.env.PLAYWRIGHT_MODULE)).href : "playwright");
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../public");
const screenshots = process.env.NARO_UI_SCREENSHOTS;
const dictionary = JSON.parse(await readFile(path.join(root,"i18n.json"),"utf8"));
if (screenshots) await mkdir(screenshots, {recursive:true});
const ids = {codex:"codex-native", claude:"claude-native", antigravity:"agy-native"};
const uuid = "949b40bc-2a83-4ac5-a625-36fa1f752ca2";
const sessions = Object.entries(ids).map(([provider,id]) => ({provider,id,title:provider === "codex" ? "改进会话加载体验" : provider === "claude" ? "整理文件变更" : "检查项目配置",cwd:"/workspace/naro",updated:1700000000}));
const work = {context:{used:12340,limit:200000,remaining:187660},plan:[{step:"Inspect files",status:"completed"},{step:"Implement fix",status:"inProgress"}],changes:[{path:"/workspace/naro/sample.txt",added:1,removed:1,diff:"@@ -1 +1 @@\n-before\n+after\n",truncated:false}]};
const calls = [];
let actionDelay = 40;
let names = new Map(), goal = {objective:"修复会话列表的加载问题",tokenBudget:5000,tokensUsed:120,status:"active"};
const csp = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; manifest-src 'self'; worker-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";
const server = createServer(async (req,res) => {
  const url = new URL(req.url,"http://localhost"), p = url.pathname;
  res.setHeader("Content-Security-Policy",csp);
  if (p.startsWith("/api/")) {
    let data = null;
    const provider = p.split("/")[3];
    if (p === "/api/access") data = {paired:true,online:true,name:"Naro UI acceptance"};
    else if (p === "/api/activity") data = {running:[],finished:[]};
    else if (/^\/api\/sessions\/[^/]+$/.test(p)) data = {sessions:sessions.filter(s=>s.provider===provider).map(s=>({...s,title:names.get(s.id)||s.title}))};
    else if (p.endsWith("/actions")) {
      let text=""; for await (const part of req) text+=part;
      const action=JSON.parse(text), id=p.split("/")[4];calls.push({...action,id});
      if(action.action==="rename") { if(action.name==="fail"){res.statusCode=400;data={error:"无法保存名称，请重试。"};} else {names.set(id,action.name);data=null;} }
      else if(action.action==="goal_get") data={goal};
      else if(action.action==="goal_set") {goal={...goal,objective:action.objective,tokenBudget:action.token_budget};data={goal};}
      else if(action.action==="goal_clear") {goal=null;data={cleared:true};}
      else if(action.action==="checkpoint_preview") data={canRewind:true,filesChanged:["/workspace/naro/sample.txt"],insertions:1,deletions:1};
      else if(action.action==="checkpoint_rewind") data={canRewind:true};
      else if(action.action==="context") data={totalTokens:0,maxTokens:200000};
      else if(action.action==="extensions" || action.action==="mcp_status") data={mcp:{data:[{name:"Local tools",runtimeStatus:"connected",tools:{echo:{name:"echo"}}}]},hooks:{data:[{hooks:[]}]},plugins:{marketplaces:[]},apps:null,apps_error:"Apps 暂时无法连接"};
      else if(action.action==="fork")data={session_id:id};
      else data={};
      await new Promise(resolve=>setTimeout(resolve,actionDelay));
    } else if (p.endsWith("/occurrences")) data={data:[{turnId:"turn-test",snippet:"Changes ready"}]};
    else if (/^\/api\/sessions\/[^/]+\/[^/]+$/.test(p)) {
      const session=sessions.find(s=>s.id===p.split("/")[4]);
      data={session:{...session,title:names.get(session.id)||session.title},transcript:{messages:[{role:"user",text:"检查文件修改是否正确",turn_id:provider==="claude"?uuid:"turn-test"},{role:"assistant",text:"Changes ready",turn_id:"turn-test",work}],model:"test-model",revision:"1"},active:null};
    } else if (p.startsWith("/api/search/")) data={data:[{thread:{id:ids.codex,name:"改进会话加载体验",cwd:"/workspace/naro"},snippet:"Changes ready"}]};
    else if (p.startsWith("/api/models/")) data={models:[{id:"test-model",label:"Test model"}],default_model:"test-model"};
    else if(p==="/api/runs")data=[];
    res.setHeader("Content-Type","application/json");res.end(JSON.stringify(data));return;
  }
  try {
    const file=p==="/"?"index.html":p.slice(1);if(file.includes(".."))throw Error();
    const bytes=await readFile(path.join(root,file));res.setHeader("Content-Type",({".html":"text/html",".js":"text/javascript",".json":"application/json",".css":"text/css",".webmanifest":"application/manifest+json",".png":"image/png"})[path.extname(file)]||"application/octet-stream");res.end(bytes);
  }catch{res.writeHead(404).end();}
});
await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
const origin="http://127.0.0.1:"+server.address().port;
const browsers=[["chromium",chromium,{channel:"chrome"}]];
if(process.env.WEBKIT_EXECUTABLE)browsers.push(["webkit",webkit,{executablePath:process.env.WEBKIT_EXECUTABLE}]);
async function settled(page) {
  await page.waitForFunction(() => !document.querySelector("#rail").classList.contains("open") && getComputedStyle(document.querySelector("#rail")).visibility === "hidden" || innerWidth > 740);
  await page.evaluate(async () => { await document.fonts.ready; await Promise.all(document.getAnimations().filter(a => a.effect?.getTiming().iterations !== Infinity).map(a => a.finished.catch(()=>{}))); });
}
const capture=async(page,name)=>{await settled(page);if(screenshots)await page.screenshot({path:path.join(screenshots,name+".png")});};
async function geometry(page) {
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),"document must not scroll sideways");
  const invalid=await page.locator("dialog[open]").evaluateAll(dialogs=>dialogs.flatMap(d=>{
    const r=d.getBoundingClientRect();const out=[];if(r.left<0||r.right>innerWidth+1||r.top<0||r.bottom>innerHeight+1)out.push("dialog offscreen");
    for(const n of d.querySelectorAll("input,textarea,select,footer button")){const q=n.getBoundingClientRect();if(q.width && (q.left<r.left||q.right>r.right+1))out.push(n.tagName+" clipped");}return out;
  }));assert.deepEqual(invalid,[]);
}
async function keyboardArea(page) {
  await page.evaluate(() => {
    const area=document.querySelector('.session-form-body');
    if (area) area.scrollTop=area.scrollHeight;
  });
  await page.evaluate(()=>new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve))));
  const footer=await page.locator('#session-form-footer').boundingBox();
  assert.ok(footer.y+footer.height<=480.5,'dialog actions remain reachable at reduced height');
}

try {
  for(const [engineName,engine,options] of browsers) {
    const browser=await engine.launch({headless:true,...options});
    try {
      for(const width of [402,1280]) {
        names=new Map();goal={objective:"修复会话列表的加载问题",tokenBudget:5000,tokensUsed:120,status:"active"};
        const page=await browser.newPage({viewport:{width,height:874},serviceWorkers:"block",colorScheme:"light"});
        const errors=[];page.on("pageerror",e=>errors.push(e.message));
        await page.goto(origin);await page.locator("#workspace").waitFor({state:"visible"});
        assert.equal(await page.locator("#session-more").isVisible(),false,"new chat has no session manager");
        assert.ok((await page.locator("#chat-page > .topbar").boundingBox()).height<80,"header stays one compact row");
        await geometry(page);await capture(page,`${engineName}-${width}-new`);
        const composerHeight=(await page.locator(".composer-wrap").boundingBox()).height;
        const welcomeTitle=page.locator("#new-view h1");
        const titleTop=(await welcomeTitle.boundingBox()).y;
        await page.locator("#draft-settings summary").click();
        await settled(page);
        assert.equal((await welcomeTitle.boundingBox()).y,titleTop,"expanding options keeps the new chat heading in place");
        assert.equal(await page.locator("#draft-settings-fields input").count(),0,"Codex has no agent or worktree field");
        await page.locator("#draft-settings-fields textarea").first().fill("/private/extra-project");
        await page.getByRole("button",{name:"Claude Code",exact:true}).click();
        assert.equal(await page.locator("#draft-settings-fields textarea").first().inputValue(),"","engine options do not leak");
        await page.locator("#draft-settings summary").click();await capture(page,`${engineName}-${width}-setup`);
        if(width<740)await page.locator("#open-drawer").click();
        await page.locator(".project-sessions .session").filter({hasText:"改进会话加载体验"}).click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();
        assert.equal((await page.locator(".composer-wrap").boundingBox()).height,composerHeight,"context does not add a composer row");
        const plan=page.locator(".turn-plan");
        assert.equal(await plan.locator("li").first().isVisible(),false,"plans are collapsed initially");
        await plan.locator("summary").click();await plan.getByText("已完成 · Inspect files",{exact:true}).waitFor();
        await plan.locator("summary").click();
        await capture(page,`${engineName}-${width}-chat`);
        await page.locator("#session-more").click();await geometry(page);await capture(page,`${engineName}-${width}-menu`);
        await page.locator("#session-menu-list").getByRole("button",{name:"重命名",exact:true}).click();
        await geometry(page);assert.equal(await page.locator("#session-dialog").isVisible(),true);await capture(page,`${engineName}-${width}-rename`);
        const name=page.getByLabel("会话名称",{exact:true});assert.equal(await name.inputValue(),"改进会话加载体验");
        await name.fill("改进手机会话体验");await page.locator("#session-form-footer").getByRole("button",{name:"保存",exact:true}).click();
        await page.waitForFunction(()=>!document.querySelector("#session-dialog").open);assert.equal(await page.locator("#chat-title").textContent(),"改进手机会话体验");
        await page.locator("#session-more").click();await page.locator("#session-menu-list").getByRole("button",{name:"目标",exact:true}).click();
        await page.getByLabel("目标",{exact:true}).waitFor();await geometry(page);await capture(page,`${engineName}-${width}-goal`);
        assert.equal(await page.getByLabel("Token 预算",{exact:true}).inputValue(),"5000");await page.locator("#session-dialog-close").click();
        await page.locator("#session-more").click();
        await page.locator("#session-menu-list").getByRole("button",{name:"上下文用量",exact:true}).click();
        await capture(page,`${engineName}-${width}-context`);
        assert.deepEqual(await page.locator(".session-metric dd").allTextContents(),["12,340","200,000","187,660"]);
        await page.locator("#session-form-footer").getByRole("button",{name:"压缩上下文",exact:true}).click();
        assert.equal(await page.locator("#prompt").inputValue(),"/compact");
        if(width<740)await page.locator("#open-drawer").click();await page.locator(".project-sessions .session").filter({hasText:"整理文件变更"}).click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();await page.locator("#session-more").click();await page.locator("#session-menu-list").getByRole("button",{name:"回退文件",exact:true}).click();
        await page.getByLabel("回退到",{exact:true}).selectOption(uuid);await page.locator("#session-form-footer").getByRole("button",{name:"预览文件回退",exact:true}).click();
        await page.locator(".session-rewind-preview li").getByText("/workspace/naro/sample.txt",{exact:true}).waitFor();await page.getByRole("button",{name:"确认文件回退",exact:true}).waitFor();await page.waitForFunction(()=>!document.querySelector("#session-dialog-close").disabled);await geometry(page);await capture(page,`${engineName}-${width}-rewind`);assert.ok(!(await page.locator("#session-dialog").innerText()).includes(uuid));
        await page.locator("#session-dialog-close").click();
        await page.locator("#session-more").click();
        await page.locator("#session-menu-list").getByRole("button",{name:"上下文用量",exact:true}).click();
        const refreshContext=page.getByRole("button",{name:"刷新上下文",exact:true});
        await refreshContext.click();await page.locator(".session-metric dd").getByText("0",{exact:true}).waitFor();
        assert.equal(await refreshContext.isEnabled(),true);
        await page.locator("#session-dialog-close").click();
        assert.equal(await page.locator("#session-more").evaluate(node=>node===document.activeElement),true);
        // The sidebar owns full-text search; results open the originating engine session.
        if(width<740)await page.locator("#open-drawer").click();
        await page.locator("#session-search").fill("Changes");await page.locator("#search-native").click();
        await page.locator("#search-results .search-hit").first().waitFor();await page.locator("#search-results .search-hit").first().click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();await settled(page);
        // Error is local to the focused rename form; cancel makes no mutation.
        await page.locator("#session-more").click();await page.locator("#session-menu-list").getByRole("button",{name:"重命名",exact:true}).click();
        await page.getByLabel("会话名称",{exact:true}).fill("fail");await page.locator("#session-form-footer").getByRole("button",{name:"保存",exact:true}).click();
        await page.locator("#session-error").waitFor({state:"visible"});assert.equal(await page.getByLabel("会话名称",{exact:true}).inputValue(),"fail");
        await page.locator("#session-dialog-close").click();assert.equal(await page.locator("#chat-title").textContent(),"改进手机会话体验");
        // Unsupported Antigravity management controls do not appear as dead buttons.
        if(width<740)await page.locator("#open-drawer").click();await page.locator("#session-search").fill("");await page.locator(".project-sessions .session").filter({hasText:"检查项目配置"}).click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();await page.locator("#session-more").click();
        assert.equal(await page.locator("#session-menu-list").getByRole("button",{name:"重命名",exact:true}).count(),0);
        await page.locator("#session-menu-list").getByRole("button",{name:"上下文用量",exact:true}).click();
        assert.equal(await page.locator("#session-form-footer").getByRole("button",{name:"压缩上下文",exact:true}).count(),0);
        await page.locator("#session-dialog-close").click();
        // A late mutation stays attached to its source session and cannot steal navigation.
        if(width<740)await page.locator("#open-drawer").click();await page.locator(".project-sessions .session").filter({hasText:"改进手机会话体验"}).click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();await page.locator("#session-more").click();await page.locator("#session-menu-list").getByRole("button",{name:"重命名",exact:true}).click();
        actionDelay=700;const before=calls.length;await page.getByLabel("会话名称",{exact:true}).fill("迟到的重命名");await page.locator("#session-form-footer").getByRole("button",{name:"保存",exact:true}).click();
        await page.waitForFunction(()=>document.querySelector("#session-dialog-close").disabled);
        assert.equal(await page.locator("#session-form-footer .primary").isDisabled(),true);
        await page.locator("#quick-new").evaluate(node=>node.click());
        await page.waitForFunction(()=>!document.querySelector("#session-dialog").open);await new Promise(resolve=>setTimeout(resolve,800));actionDelay=40;
        assert.equal(await page.locator("#chat-title").textContent(),"Naro");assert.equal(calls.length,before+1);assert.equal(await page.locator("#session-more").isVisible(),false);
        // Reset only this deterministic fixture's name for the next viewport.
        names.set(ids.codex,"改进手机会话体验");
        assert.deepEqual(errors,[]);await page.close();
        console.log(`${engineName} ${width}px: new chat, menu, forms, scoped draft, context and checkpoint preview passed`);
      }
      // Visual matrix: long translations, dark surfaces, narrow and keyboard-height bounds.
      for(const locale of ["en","ja"]) for(const theme of ["light","dark"]) {
        const width=locale==="en"?390:320;
        const page=await browser.newPage({viewport:{width,height:874},serviceWorkers:"block",colorScheme:theme});
        await page.addInitScript(({locale,theme})=>{localStorage.setItem("narolink.language",locale);localStorage.setItem("narolink.appearance",theme);},{locale,theme});
        await page.goto(origin);await page.locator("#workspace").waitFor({state:"visible"});await page.waitForFunction(code=>document.documentElement.lang===code,locale);
        const tr=key=>dictionary[key]?.[locale==="en"?0:1]||key;
        assert.equal(await page.locator("#session-more").isVisible(),false);await geometry(page);await capture(page,`${engineName}-${width}-${locale}-${theme}-new`);
        await page.locator("#open-drawer").click();await page.locator(".project-sessions .session").filter({hasText:"改进手机会话体验"}).click();
        await page.locator("#messages").getByText("Changes ready",{exact:true}).waitFor();await page.locator("#session-more").click();await geometry(page);await capture(page,`${engineName}-${width}-${locale}-${theme}-menu`);
        await page.locator("#session-menu-list").getByRole("button",{name:tr("上下文用量"),exact:true}).click();
        await geometry(page);await capture(page,`${engineName}-${width}-${locale}-${theme}-context`);
        await page.locator("#session-dialog-close").click();await page.locator("#session-more").click();
        await page.locator("#session-menu-list").getByRole("button",{name:tr("目标"),exact:true}).click();await page.getByLabel(tr("Token 预算"),{exact:true}).waitFor();
        await geometry(page);await capture(page,`${engineName}-${width}-${locale}-${theme}-goal`);
        await page.setViewportSize({width,height:480});await geometry(page);await keyboardArea(page);await capture(page,`${engineName}-${width}-${locale}-${theme}-reduced-height`);
        await page.close();
      }
    }finally{await browser.close();}
  }
  assert.ok(calls.some(c=>c.action==="rename"));assert.ok(calls.some(c=>c.action==="checkpoint_preview"));
}finally{await new Promise(resolve=>server.close(resolve));}
