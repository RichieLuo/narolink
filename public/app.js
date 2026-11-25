const engines = {
  codex: {label: "Codex", short: "Codex", mark: "C"},
  claude: {label: "Claude Code", short: "Claude", mark: "Cl"},
  antigravity: {label: "Antigravity", short: "Antigravity", mark: "A"},
};
const providers = Object.keys(engines);
const $ = id => document.getElementById(id);
const bySession = (session, view) => session.provider === view.provider && session.id === view.sessionId;
const emptyView = provider => ({
  provider, sessionId: null, data: null, messages: [], warning: "", run: null, busy: false, snapshot: null, lastFinished: null, files: [],
});

let paired = false;
let filter = "all";
let provider = "codex";
let view = emptyView(provider);
let histories = new Map();
let models = new Map();
let running = [];
let lastCompleted = null;
let syncing = false;
let pairNotice = $("pair-notice");
let composerNotice = $("composer-notice");
let poller = null;
let historyLoading = false;
let inventoryEpoch = 0;
let runEpoch = 0;
let liveKey = "";
const folderKey = "narolink.last-folder";

function notice(node, text) {
  node.textContent = text || "";
  node.hidden = !text;
}

function element(tag, text = "", className = "") {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  if (className) node.className = className;
  return node;
}

async function api(path, body) {
  let response;
  try {
    response = await fetch("/api" + path, body === undefined
      ? {cache: "no-store"}
      : {method: "POST", cache: "no-store", headers: {"Content-Type": "application/json"}, body: JSON.stringify(body)});
  } catch {
    setConnection(false);
    throw new Error("无法连接电脑。请确认 NaroLink 和远程网络仍在运行。");
  }
  setConnection(true);
  if (response.status === 204) return null;
  const json = await response.json().catch(() => ({}));
  if (!response.ok) {
    if (response.status === 401 && path !== "/pair") setPaired(false);
    throw new Error(json.error || "电脑端返回错误（" + response.status + "）");
  }
  return json;
}

function setConnection(online) {
  const label = $("network-status");
  label.textContent = online ? "已连接电脑" : "电脑未连接";
  label.classList.toggle("offline", !online);
}

function setPaired(value) {
  paired = value;
  $("gate").hidden = value;
  $("workspace").hidden = !value;
  if (value) {
    notice(pairNotice, "");
    $("pair-code").value = "";
    startNew(provider);
    refreshHistories(false);
    syncRunning();
  } else {
    clearTimeout(poller);
    ++inventoryEpoch;
    historyLoading = false;
    clearFiles(view);
    view = emptyView(provider);
    histories = new Map();
    models = new Map();
    running = [];
  }
}

async function pair(code) {
  if (!/^\d{8}$/.test(code)) return;
  $("pair-submit").disabled = true;
  try {
    await api("/pair", {code});
    setPaired(true);
  } catch (problem) {
    notice(pairNotice, problem.message);
  } finally {
    $("pair-submit").disabled = false;
  }
}

function engineMark(id) {
  const mark = element("span", engines[id]?.mark, "engine-mark " + id);
  mark.setAttribute("aria-hidden", "true");
  return mark;
}

function button(text, className, action) {
  const node = element("button", text, className);
  node.type = "button";
  if (action) node.addEventListener("click", action);
  return node;
}

function drawer(open) {
  $("rail").classList.toggle("open", open);
  $("scrim").hidden = !open;
  $("open-drawer").setAttribute("aria-expanded", String(open));
  if (open) $("close-drawer").focus();
}

function renderFilters() {
  $("filters").replaceChildren(...["all", ...providers].map(id => {
    const node = button(id === "all" ? "全部" : engines[id].short, "", () => {
      filter = id;
      renderFilters();
      renderSidebar();
    });
    node.setAttribute("aria-pressed", String(filter === id));
    return node;
  }));
}

function renderEngineChoices() {
  $("new-engines").replaceChildren(...providers.map(id => {
    const node = button("", "provider-choice", () => {
      if (view.busy || id === view.provider) return;
      provider = id;
      view.provider = id;
      view.model = null;
      renderHeader();
      renderEngineChoices();
      loadModels(id);
    });
    node.append(engineMark(id), element("span", engines[id].label));
    node.setAttribute("aria-pressed", String(id === view.provider));
    return node;
  }));
}

function listedSessions() {
  return [...histories.values()].flatMap(list => list.sessions || []).sort((a, b) => b.updated - a.updated);
}

function folderName(path) {
  return path?.split(/[\\/]/).filter(Boolean).at(-1) || "";
}

function renderSidebar() {
  const query = $("session-search").value.trim().toLowerCase();
  const sessions = listedSessions().filter(item =>
    (filter === "all" || item.provider === filter) && (!query ||
      [item.title, item.cwd, engines[item.provider]?.label].some(text => text?.toLowerCase().includes(query)))
  );
  const errors = (filter === "all" ? providers : [filter]).map(id => histories.get(id)?.warning).filter(Boolean);
  notice($("inventory-notice"), errors[0] || (!sessions.length && !running.length &&
    (historyLoading ? "正在读取电脑上的会话…" : query ? "没有匹配的会话" : "暂无本机会话")));
  const items = sessions.map(session => {
    const row = button("", "session", () => openSession(session));
    row.setAttribute("aria-current", String(bySession(session, view)));
    row.title = session.title + "\n" + session.cwd;
    const copy = element("span", "", "session-copy");
    const date = session.updated ? new Date(session.updated * 1000).toLocaleDateString(undefined, {month: "short", day: "numeric"}) : "";
    copy.append(element("strong", session.title),
      element("small", [folderName(session.cwd), date].filter(Boolean).join(" · ") || engines[session.provider].label));
    row.append(engineMark(session.provider), copy);
    return row;
  });
  $("session-list").replaceChildren(...items);
  const tasks = running.filter(task => filter === "all" || task.provider === filter).map(task => {
    const row = button("", "session running", () => openRunning(task));
    row.setAttribute("aria-current", String(view.run === task.id));
    const copy = element("span", "", "session-copy");
    copy.append(element("strong", task.title), element("small", "正在运行 · " + engines[task.provider].label));
    row.append(engineMark(task.provider), copy);
    return row;
  });
  $("running-list").replaceChildren(...tasks);
  const paths = [...new Set(listedSessions().map(session => session.cwd))].slice(0, 150);
  $("recent-folders").replaceChildren(...paths.map(path => {
    const option = document.createElement("option");
    option.value = path;
    return option;
  }));
}

async function refreshHistories(force) {
  const epoch = ++inventoryEpoch;
  historyLoading = true;
  $("refresh").disabled = true;
  renderSidebar();
  await Promise.all(providers.map(async id => {
    let data;
    try {
      data = await api("/sessions/" + id + (force ? "?refresh=true" : ""));
    } catch (problem) {
      data = {sessions: histories.get(id)?.sessions || [], warning: engines[id].label + " · " + problem.message};
    }
    if (epoch === inventoryEpoch) {
      histories.set(id, data);
      renderSidebar();
    }
  }));
  if (epoch === inventoryEpoch) {
    historyLoading = false;
    $("refresh").disabled = false;
    renderSidebar();
  }
}

async function loadModels(id, refresh = false) {
  if (!refresh && models.has(id)) {
    renderModels();
    return;
  }
  if (view.provider === id) $("refresh-models").disabled = true;
  try {
    const choices = await api("/models/" + id + (refresh ? "?refresh=true" : ""));
    if (!paired) return;
    models.set(id, choices);
    if (view.provider === id) $("models").title = "给当前会话选择模型";
  } catch (problem) {
    if (view.provider === id) $("models").title = problem.message;
    if (refresh && view.provider === id) notice(composerNotice, problem.message);
  } finally {
    if (view.provider === id) {
      $("refresh-models").disabled = false;
      renderModels();
    }
  }
}

function renderModels() {
  const select = $("models");
  const selected = view.model || "";
  const choices = models.get(view.provider) || [];
  const options = [new Option(view.sessionId ? "会话当前模型" : "默认模型", "")];
  for (const model of choices) {
    if (model.id && !options.some(option => option.value === model.id)) {
      options.push(new Option(model.label, model.id));
    }
  }
  select.replaceChildren(...options);
  select.value = selected;
  if (!select.value) select.selectedIndex = 0;
  select.disabled = view.busy;
}

function startNew(engine) {
  clearTimeout(poller);
  provider = engine;
  clearFiles(view);
  view = emptyView(engine);
  $("directory").value = localStorage.getItem(folderKey) || "";
  $("prompt").value = "";
  notice(composerNotice, "");
  drawer(false);
  paint();
  loadModels(engine);
}

async function openSession(session, task = null) {
  clearTimeout(poller);
  provider = session.provider;
  clearFiles(view);
  view = {...emptyView(provider), sessionId: session.id, data: session, warning: "正在读取原会话…", run: task?.id || null, busy: Boolean(task)};
  const target = view;
  $("prompt").value = "";
  notice(composerNotice, "");
  drawer(false);
  paint();
  loadModels(provider);
  if (target.run) poll(target);
  await openHistory(target);
}

async function openHistory(target, refresh = false) {
  if (!target.sessionId) return;
  try {
    const result = await api("/sessions/" + target.provider + "/" + encodeURIComponent(target.sessionId) + (refresh ? "?refresh=true" : ""));
    if (view !== target) return;
    target.data = result.session;
    target.warning = result.transcript.warning;
    const last = result.transcript.messages;
    const canReplace = !refresh || !target.snapshot ||
      (last.length > target.messages.length && last.some(item => item.role === "user" && item.text.trim() === target.snapshot.prompt.trim()));
    if (canReplace) {
      target.messages = last;
      if (refresh && !target.busy) target.snapshot = null;
    }
    if (result.active && !target.run) {
      target.run = result.active.id;
      target.busy = true;
      poll(target);
    }
  } catch (problem) {
    if (view === target) target.warning = problem.message;
  }
  if (view === target) paint();
}

function openRunning(task) {
  const known = listedSessions().find(session => session.provider === task.provider && session.id === task.session_id);
  if (known) {
    openSession(known, task);
    return;
  }
  clearTimeout(poller);
  clearFiles(view);
  view = {...emptyView(task.provider), sessionId: task.session_id, run: task.id, busy: true, title: task.title};
  const target = view;
  drawer(false);
  paint();
  loadModels(view.provider);
  poll(target);
  if (task.session_id) openHistory(target);
}

function renderHeader() {
  $("chat-title").textContent = view.data?.title || view.title || "新建会话";
  $("chat-path").textContent = view.data?.cwd || "";
  $("chat-path").hidden = !view.data?.cwd;
  $("active-provider").textContent = engines[view.provider].label;
  $("new-view").hidden = Boolean(view.sessionId || view.run);
}

function markup(text) {
  const wrapper = element("div", "", "message-text");
  const marker = String.fromCharCode(96).repeat(3);
  const blocks = text.split(marker);
  for (const [index, block] of blocks.entries()) {
    if (index % 2 === 0) {
      if (block) wrapper.append(document.createTextNode(block.trimStart()));
      continue;
    }
    const code = block.replace(/^[a-zA-Z0-9_+.#-]{0,32}\r?\n/, "").replace(/\r?\n$/, "");
    wrapper.append(element("pre", code));
  }
  return wrapper;
}

function message(role, text, engine = view.provider) {
  const row = element("article", "", "message " + role);
  const title = element("header", role === "user" ? "你" : engines[engine].label, "message-title");
  if (role === "assistant") {
    title.append(button("复制", "", async event => {
      try {
        await navigator.clipboard.writeText(text);
        event.target.textContent = "已复制";
      } catch {
        notice(composerNotice, "可选中回复文字后复制。");
      }
    }));
  }
  row.append(title, role === "assistant" ? markup(text) : element("div", text, "message-text"));
  return row;
}

function renderMessages() {
  const area = $("messages");
  area.replaceChildren(...view.messages.map(entry => message(entry.role, entry.text)));
  const preview = !view.messages.length && !view.busy && !view.snapshot ? view.data?.preview : "";
  notice($("history-notice"), view.warning || (preview && "原会话预览") || "");
  if (preview) area.append(message("user", preview));
}

function renderRun() {
  const area = $("live");
  const snap = view.snapshot;
  const key = snap ? [snap.id, snap.output.length, snap.activity, snap.status, snap.approval?.id, snap.error].join(":") : "";
  if (key === liveKey && area.hidden === !snap) return;
  liveKey = key;
  const scroll = $("conversation");
  const follow = scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 140;
  area.hidden = !snap;
  area.replaceChildren();
  if (!snap) return;
  const user = message("user", snap.prompt);
  if (snap.images?.length) {
    const files = element("div", "", "sent-images");
    snap.images.forEach((name, index) => {
      const img = new Image();
      img.src = "/api/runs/" + encodeURIComponent(snap.id) + "/images/" + index;
      img.alt = name;
      img.title = name;
      img.loading = "lazy";
      files.append(img);
    });
    user.append(files);
  }
  area.append(user);
  if (snap.output) area.append(message("assistant", snap.output, snap.provider));
  if (snap.status === "running" && snap.activity) area.append(element("div", snap.activity, "activity"));
  if (snap.status === "stopped") area.append(element("div", "已停止", "activity"));
  if (snap.error) area.append(element("div", snap.error, "notice"));
  if (snap.approval) {
    const box = element("div", "", "approval");
    box.append(element("strong", "确认操作 · " + snap.approval.title),
      element("pre", snap.approval.details),
      button("允许一次", "primary", () => decide(snap, true)),
      button("拒绝", "secondary", () => decide(snap, false)));
    area.append(box);
  }
  if (follow) scroll.scrollTop = scroll.scrollHeight;
}

function clearFiles(target) {
  for (const entry of target.files) URL.revokeObjectURL(entry.preview);
  target.files = [];
}

function addImages(files) {
  if (view.busy) return;
  notice(composerNotice, "");
  for (const file of files) {
    if (!/\.(png|jpe?g|gif|webp)$/i.test(file.name) && !/^image\/(png|jpeg|gif|webp)$/.test(file.type)) {
      notice(composerNotice, "请选择 PNG、JPEG、GIF 或 WebP 图片。");
      continue;
    }
    if (view.files.length >= 4) {
      notice(composerNotice, "最多可以附加四张图片。");
      break;
    }
    if (view.files.reduce((size, entry) => size + entry.file.size, 0) + file.size > 12 * 1024 * 1024) {
      notice(composerNotice, "图片总大小不能超过 12 MiB。");
      continue;
    }
    view.files.push({file, preview: URL.createObjectURL(file)});
  }
  renderDraftFiles();
  renderControls();
}

function renderDraftFiles() {
  const area = $("draft-files");
  area.hidden = !view.files.length;
  area.replaceChildren(...view.files.map(entry => {
    const node = element("div", "", "draft-file");
    const img = new Image();
    img.src = entry.preview;
    img.alt = "";
    const remove = button("×","icon",() => {
      if (view.busy) return;
      view.files = view.files.filter(item => item !== entry);
      URL.revokeObjectURL(entry.preview);
      renderDraftFiles();
      renderControls();
    });
    remove.setAttribute("aria-label","移除 " + entry.file.name);
    node.append(img, element("span",entry.file.name),remove);
    return node;
  }));
}

function upload(entry) {
  return new Promise((resolve,reject) => {
    const reader = new FileReader();
    reader.addEventListener("load",() => resolve({name:entry.file.name,data:String(reader.result).split(",")[1]}),{once:true});
    reader.addEventListener("error",() => reject(new Error("无法读取图片 " + entry.file.name)),{once:true});
    reader.readAsDataURL(entry.file);
  });
}
function renderControls() {
  const hasProject = view.sessionId || $("directory").value.trim();
  $("send").disabled = view.busy || !hasProject || (!$("prompt").value.trim() && !view.files.length);
  $("stop").hidden = !view.busy || !view.run;
  $("prompt").disabled = view.busy;
  $("compose-editor").dataset.text = $("prompt").value;
  $("add-images").disabled = view.busy || view.files.length >= 4;
  $("directory").disabled = view.busy;
  for (const option of $("new-engines").children) option.disabled = view.busy;
  renderModels();
}

function paint() {
  liveKey = "";
  renderFilters();
  renderEngineChoices();
  renderHeader();
  renderSidebar();
  renderMessages();
  renderDraftFiles();
  renderRun();
  renderControls();
}

async function send() {
  const target = view;
  if (target.busy || $("send").disabled) return;
  const prompt = $("prompt").value.trim();
  const directory = (target.data?.cwd || $("directory").value).trim();
  target.busy = true;
  target.snapshot = null;
  renderControls();
  renderRun();
  notice(composerNotice, "");
  try {
    const images = await Promise.all(target.files.map(upload));
    const run = await api("/runs", {provider: target.provider, session_id: target.sessionId, model: target.model || null, cwd: directory, prompt, images});
    if (view !== target) return;
    if (!target.sessionId) localStorage.setItem(folderKey, directory);
    $("prompt").value = "";
    clearFiles(target);
    renderDraftFiles();
    target.run = run.id;
    target.title = run.title;
    running = [...running.filter(entry => entry.id !== run.id), run];
    renderSidebar();
    renderHeader();
    poll(target);
  } catch (problem) {
    if (view === target) {
      target.busy = false;
      notice(composerNotice, problem.message);
      renderControls();
    }
  }
}

async function poll(target) {
  if (!paired || view !== target || !target.run) return;
  const runId = target.run;
  const epoch = ++runEpoch;
  try {
    const snap = await api("/runs/" + runId);
    if (view !== target || target.run !== runId || runEpoch !== epoch) return;
    target.snapshot = snap;
    target.sessionId ||= snap.session_id;
    target.busy = snap.status === "running";
    renderHeader();
    renderRun();
    renderControls();
    if (!target.busy) {
      target.run = null;
      target.lastFinished = runId;
      running = running.filter(run => run.id !== runId);
      renderSidebar();
      if (target.sessionId) openHistory(target, true);
      if (!historyLoading) refreshHistories(false);
      return;
    }
  } catch (problem) {
    if (view !== target || !paired) return;
    notice(composerNotice, problem.message);
  }
  if (view === target) poller = setTimeout(() => poll(target), 450);
}

async function syncRunning() {
  if (!paired || syncing) return;
  syncing = true;
  try {
    const activity = await api("/activity");
    if (!paired) return;
    const list = activity.running;
    const changed = JSON.stringify(list) !== JSON.stringify(running);
    running = list;
    if (changed) renderSidebar();
    const latest = activity.finished[0]?.id;
    if (latest && lastCompleted !== latest && !historyLoading) {
      lastCompleted = latest;
      refreshHistories(false);
    }
    if (!view.run && !view.busy && view.sessionId) {
      const sameSession = run => run.provider === view.provider && run.session_id === view.sessionId;
      const task = running.find(sameSession);
      const finished = activity.finished.find(sameSession);
      if (task) {
        view.run = task.id;
        view.busy = true;
        poll(view);
      } else if (finished && view.lastFinished !== finished.id) {
        view.lastFinished = finished.id;
        openHistory(view,true);
      }
    }
  } catch {} finally {
    syncing = false;
  }
}

async function decide(snap, allow) {
  try {
    await api("/runs/" + snap.id + "/approval", {approval_id: snap.approval.id, allow});
    if (view.run === snap.id) {
      view.snapshot = {...snap, approval: null};
      renderRun();
    }
  } catch (problem) {
    notice(composerNotice, problem.message);
  }
}

$("pair-form").addEventListener("submit", event => {
  event.preventDefault();
  pair($("pair-code").value.trim());
});
$("composer").addEventListener("submit", event => {
  event.preventDefault();
  send();
});
$("add-images").addEventListener("click", () => $("image-files").click());
$("image-files").addEventListener("change", () => {
  addImages($("image-files").files);
  $("image-files").value = "";
});
$("prompt").addEventListener("paste", event => {
  const files = Array.from(event.clipboardData?.files || []).filter(file => file.type.startsWith("image/"));
  if (!files.length) return;
  event.preventDefault();
  addImages(files);
});
$("prompt").addEventListener("input", renderControls);
$("prompt").addEventListener("keydown", event => {
  if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !event.isComposing) {
    event.preventDefault();
    send();
  }
});
$("directory").addEventListener("input", renderControls);
$("models").addEventListener("change", () => { view.model = $("models").value || null; });
$("refresh-models").addEventListener("click", () => loadModels(view.provider, true));
$("session-search").addEventListener("input", renderSidebar);
$("new-chat").addEventListener("click", () => startNew(provider));
$("quick-new").addEventListener("click", () => startNew(provider));
$("refresh").addEventListener("click", () => refreshHistories(true));
$("open-drawer").addEventListener("click", () => drawer(true));
$("close-drawer").addEventListener("click", () => { drawer(false); $("open-drawer").focus(); });
$("scrim").addEventListener("click", () => { drawer(false); $("open-drawer").focus(); });
document.addEventListener("keydown", event => {
  if (event.key === "Escape" && !$("scrim").hidden) {
    drawer(false);
    $("open-drawer").focus();
  }
});
$("disconnect").addEventListener("click", async () => {
  try { await api("/logout", {}); }
  catch (problem) { notice(composerNotice, problem.message); return; }
  setPaired(false);
});
$("stop").addEventListener("click", async () => {
  if (!view.run) return;
  try { await api("/runs/" + view.run + "/stop", {}); }
  catch (problem) { notice(composerNotice, problem.message); }
});
setInterval(syncRunning, 2000);
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") syncRunning();
});
window.addEventListener("online", syncRunning);

if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});
const code = new URLSearchParams(location.hash.slice(1)).get("pair");
if (code) {
  history.replaceState(null, "", location.pathname + location.search);
  if (/^\d{8}$/.test(code)) pair(code);
}
if (!code || !/^\d{8}$/.test(code)) {
  api("/access").then(access => { if (access.paired) setPaired(true); }).catch(problem => notice(pairNotice, problem.message));
}
