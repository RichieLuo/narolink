const engines = {
  codex: {label: "Codex", short: "Codex", steering: true},
  claude: {label: "Claude Code", short: "Claude", steering: true},
  antigravity: {label: "Antigravity", short: "Antigravity", steering: true},
};
const providers = Object.keys(engines);
const $ = id => document.getElementById(id);
const devicePath = /^\/devices\/[\da-f]{32}\/$/;
const deviceScope = devicePath.test(location.pathname) ? location.pathname : "/";
const devicePrefix = deviceScope.slice(0, -1);
const devicesKey = "narolink.devices";
const bySession = (session, view) => session.provider === view.provider && session.id === view.sessionId;
const isNewView = view => !view.sessionId && !view.run && !view.snapshot;
const emptyView = provider => ({
  provider, sessionId: null, data: null, messages: [], warning: "", run: null, busy: false, snapshot: null, lastFinished: null, files: [],
  model: null, runModel: null, historyModel: null, effort: null, fast: null,
  permissionMode: undefined, permissionLoad: null, permissionSaving: false, permissionPersistence: true,
  collaborationMode: null, stopping: false,
  replyReveal: null,
  location: "", cwd: "", sending: false, startupDraft: null,
  withdrawing: null, editingWithdrawn: false, otherDrafts: [],
  historyRevision: null, historyEpoch: 0, historyPending: false, followLatest: true,
});

let paired = false;
let filter = "all";
let provider = "codex";
let view = emptyView(provider);
let histories = new Map();
let models = new Map();
let modelLoading = new Set();
const modelErrors = new Map();
const projectsOpen = new Map();
const recoveredWithdrawals = new Map();
let modelPanelKey = "";
let permissionPanelKey = "";
let permissionWrite = Promise.resolve();
let running = [];
let lastCompleted = null;
let lastTitleRevision = 0;
let syncing = false;
let pairNotice = $("pair-notice");
let composerNotice = $("composer-notice");
let runWatcher = null;
let historyLoading = false;
let inventoryEpoch = 0;
let liveKey = "";
let pendingKey = "";
const folderKey = "narolink.last-folder" + devicePrefix;
const projectKey = "narolink.last-project" + devicePrefix;
const modelPreferenceKey = id => "narolink.model-preference" + devicePrefix + ":" + id;
const sendModeKey = "narolink.send-mode";
let sendMode = localStorage.getItem(sendModeKey) === "steer" ? "steer" : "queue";
const languageKey = "narolink.language";
const languageTrigger = $("interface-language");
const defaultLanguage = "zh-CN";
const translationIndex = new Map([["en", 0], ["ja", 1]]);
let language = defaultLanguage;
let translations = {};
const staticText = [...document.querySelectorAll("[data-i18n]")].map(node => [node, node.textContent]);
const staticAttributes = [...document.querySelectorAll("[data-i18n-attrs]")].flatMap(node =>
  node.dataset.i18nAttrs.split(" ").map(name => [node, name, node.getAttribute(name)]));
const notices = new Map();
const narrowLayout = matchMedia("(max-width: 740px)");
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");
const composerSettings = [[$("models"), $("model-panel")], [$("permissions"), $("permission-panel")]];
const picker = $("destination-picker");
const questionDialog = $("question-dialog");
const deviceDialog = $("device-dialog");
const scanDialog = $("scan-dialog");
const scanVideo = $("scan-video");
const scanStatus = $("scan-status");
let scanSession = null;
const settingsDialog = $("settings-dialog");
const filePreviewDialog = $("file-preview");
let filePreview = null;
const quotaPage = $("quota-page");
let usageSection = null;
let tokenPeriod = "24h";
let tokenRequest = 0;
let tokenReport = null;
let quotaDate;
let quotaAmount;
let quotaProvider = "codex";
let quotaRequest = 0;
let chatScroll = null;
let quotaResetCount = null;
let quotaResetAttempt = null;
let quotaResetNotice = "";
const quotaResetKey = "narolink.quota-reset" + devicePrefix;
let deviceName = savedDevices().get(deviceScope) || "";
let questionForm = null;
let promptComposing = false;
const conversation = $("conversation");
const runningList = $("running-list");
const runningSpinnerPeriod = 900;
let runningSpinnerFrame = null;
const destinationOptions = new Map();
const renderedLists = new WeakMap();
let pickerTrigger = null;

function t(source, values) {
  const pattern = Object.hasOwn(translations, source) ? translations[source][translationIndex.get(language)] : undefined;
  const text = pattern ?? source;
  return values ? text.replace(/\{(\w+)\}/g, (match, name) => Object.hasOwn(values, name) ? values[name] : match) : text;
}

function applyLanguage(selected) {
  language = translationIndex.has(selected) ? selected : defaultLanguage;
  document.documentElement.lang = language;
  quotaDate = new Intl.DateTimeFormat(language, {
    month: "long", day: "numeric", hour: "2-digit", minute: "2-digit", hourCycle: "h23",
  });
  quotaAmount = new Intl.NumberFormat(language, {maximumFractionDigits: 2});
  for (const [node, source] of staticText) setText(node, t(source));
  for (const [node, name, source] of staticAttributes) node.setAttribute(name, t(source));
  renderLanguageChoice(language);
  renderSendModeChoice(sendMode);
  renderAppearanceChoice(window.naroAppearance.preference);
  for (const [node, source] of notices) setText(node, t(source));
  renderPairInstructions();
  if (paired) {
    setConnection(!$("network-status").classList.contains("offline"));
    for (const field of questionForm?.fields ?? []) field.translate();
    paint();
    if (typeof renderGit === "function") renderGit();
    if (usageSection) labelUsageRefresh();
    if (usageSection === "tokens") renderTokenUsage();
    else if (usageSection === "quota") loadQuota();
  }
}

function renderPairInstructions() {
  const source = $("pair-submit").hidden
    ? "扫描电脑「设备连接」中的二维码，或从设备列表选择已配对的电脑。"
    : "扫描电脑「设备连接」中的二维码，或输入配对码。";
  setText($("pair-instructions"), t(source));
}

function setText(node, text) {
  if (node.textContent !== text) node.textContent = text;
}

function scrollToLatest() {
  if (isNewView(view) || !view.followLatest || !quotaPage.hidden) return;
  const bottom = Math.max(0, conversation.scrollHeight - conversation.clientHeight);
  if (Math.abs(conversation.scrollTop - bottom) > 0.5) conversation.scrollTop = bottom;
}

function notice(node, text) {
  if (node === composerNotice) node.classList.remove("is-feedback");
  if (text) notices.set(node, text);
  else notices.delete(node);
  setText(node, text ? t(text) : "");
  node.hidden = !text;
}

function feedback(text) {
  notice(composerNotice,text);
  composerNotice.classList.add("is-feedback");
}

function element(tag, text = "", className = "") {
  const node = document.createElement(tag);
  if (text) node.textContent = text;
  if (className) node.className = className;
  return node;
}

function renderList(parent, items) {
  const previous = renderedLists.get(parent) || new Map();
  const next = new Map();
  for (const [key, value, render] of items) {
    const signature = JSON.stringify([language, value]);
    const cached = previous.get(key);
    const node = cached && cached.signature === signature ? cached.node : render(cached?.node);
    next.set(key, {signature, node});
  }
  for (const [key, {node}] of previous) {
    if (next.get(key)?.node !== node) node.remove();
  }
  let cursor = parent.firstElementChild;
  for (const {node} of next.values()) {
    if (cursor !== node) parent.insertBefore(node, cursor);
    cursor = node.nextElementSibling;
  }
  renderedLists.set(parent, next);
}

function savedDevices() {
  try {
    return new Map(Object.entries(JSON.parse(localStorage.getItem(devicesKey) || "{}"))
      .filter(([path, name]) => (path === "/" || devicePath.test(path)) && typeof name === "string"));
  } catch {
    return new Map();
  }
}

function rememberDevice(name) {
  const devices = savedDevices();
  if (name === null) devices.delete(deviceScope);
  else devices.set(deviceScope, name);
  localStorage.setItem(devicesKey, JSON.stringify(Object.fromEntries(devices)));
}

function showDevices() {
  const devices = [...savedDevices()].map(([path, name]) => {
    const link = element("a", displayDeviceName(name, path), "saved-device");
    link.href = path;
    if (paired && path === deviceScope) {
      link.setAttribute("aria-current", "page");
      link.append(element("small", t("当前设备")));
    }
    return link;
  });
  $("saved-devices").replaceChildren(...devices);
  $("saved-devices").hidden = devices.length === 0;
  $("disconnect").hidden = !paired;
  notice($("device-notice"), "");
  deviceDialog.showModal();
}

function connectionAddress(value) {
  try {
    const url = new URL(value.trim());
    const allowed = url.protocol === "https:" ||
      (url.protocol === "http:" && ["127.0.0.1", "[::1]"].includes(url.hostname));
    if (allowed && (url.pathname === "/" || devicePath.test(url.pathname)) &&
        !url.username && !url.password && !url.search) return url;
  } catch {}
  return null;
}

function stopScanner() {
  const session = scanSession;
  if (!session) return;
  scanSession = null;
  clearTimeout(session.timer);
  session.worker?.terminate();
  scanVideo.srcObject?.getTracks().forEach(track => track.stop());
  scanVideo.srcObject = null;
}

function closeScanner() {
  stopScanner();
  scanDialog.close();
}

async function startScanner() {
  if (scanDialog.open) return;
  scanDialog.showModal();
  if (!navigator.mediaDevices?.getUserMedia) {
    notice(scanStatus, "此浏览器无法使用相机。可以使用手机系统相机扫码，或手动连接。");
    return;
  }

  const session = {worker: null, timer: null};
  scanSession = session;
  const fail = message => {
    if (scanSession !== session) return;
    stopScanner();
    notice(scanStatus, message);
  };
  notice(scanStatus, "正在打开相机…");
  try {
    const stream = await navigator.mediaDevices.getUserMedia({video: {facingMode: {ideal: "environment"}}, audio: false});
    if (scanSession !== session) {
      stream.getTracks().forEach(track => track.stop());
      return;
    }
    scanVideo.srcObject = stream;
    await scanVideo.play();
    if (scanSession !== session) return;
    for (const track of stream.getVideoTracks()) {
      track.addEventListener("ended", () => fail("相机已关闭，请关闭扫码窗口后重试。"));
    }

    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", {willReadFrequently: true});
    if (!context) throw new Error("Camera frames are unavailable");
    const worker = new Worker("/qr-worker.js");
    session.worker = worker;
    const nextFrame = () => { session.timer = setTimeout(readFrame, 120); };
    function readFrame() {
      if (scanSession !== session) return;
      const {videoWidth, videoHeight, readyState} = scanVideo;
      if (readyState < 2 || !videoWidth || !videoHeight) return nextFrame();
      const scale = Math.min(1, 720 / Math.max(videoWidth, videoHeight));
      const width = Math.max(1, Math.round(videoWidth * scale));
      const height = Math.max(1, Math.round(videoHeight * scale));
      if (canvas.width !== width || canvas.height !== height) {
        canvas.width = width;
        canvas.height = height;
      }
      context.drawImage(scanVideo, 0, 0, width, height);
      const {data} = context.getImageData(0, 0, width, height);
      worker.postMessage({width, height, data}, [data.buffer]);
    }
    worker.addEventListener("message", ({data}) => {
      if (scanSession !== session) return;
      if (data) {
        const url = connectionAddress(data);
        if (url && /^#pair=\d{8}$/.test(url.hash)) {
          closeScanner();
          if (deviceDialog.open) deviceDialog.close();
          if (!(paired && url.origin === location.origin && url.pathname === location.pathname)) {
            location.assign(url.href);
          }
          return;
        }
        notice(scanStatus, "请扫描 Naro 电脑端「设备连接」里的配对二维码。");
      }
      nextFrame();
    });
    worker.addEventListener("error", () => fail("扫码功能未能加载，请关闭扫码窗口后重试。"));
    notice(scanStatus, "将电脑「设备连接」里的二维码对准取景框。");
    readFrame();
  } catch (problem) {
    fail(problem.name === "NotAllowedError"
      ? "请允许浏览器使用相机，或关闭扫码窗口后手动连接。"
      : "无法打开相机。可以使用手机系统相机扫码，或手动连接。");
  }
}

function scopedAsset(path) {
  return path?.startsWith("/api/") ? devicePrefix + path : path;
}

async function api(path, body) {
  let response;
  try {
    response = await fetch(devicePrefix + "/api" + path, body === undefined
      ? {cache: "no-store"}
      : {method: "POST", cache: "no-store", headers: {"Content-Type": "application/json"}, body: JSON.stringify(body)});
  } catch {
    setConnection(false);
    throw new Error("无法连接电脑。请确认 Naro 和远程网络仍在运行。");
  }
  const json = response.status === 204 ? null : await response.json().catch(() => ({}));
  setConnection(json?.online !== false);
  if (!response.ok) {
    if (response.status === 401 && path !== "/pair") setPaired(false);
    const problem = new Error(json.error || t("电脑端返回错误（{status}）", {status: response.status}));
    problem.status = response.status;
    throw problem;
  }
  return json;
}

function defaultDeviceName(path) {
  return path === "/" ? "当前电脑" : "设备 " + path.slice(9, 15);
}

function displayDeviceName(name, path) {
  if (name === defaultDeviceName(path)) {
    return path === "/" ? t("当前电脑") : t("设备 {id}", {id: path.slice(9, 15)});
  }
  return name === "Naro 电脑" ? t(name) : name;
}

function setConnection(online) {
  const label = $("network-status");
  const status = t(online ? "已连接" : "未连接");
  const name = displayDeviceName(deviceName, deviceScope);
  setText(label, name || t(online ? "已连接电脑" : "电脑未连接"));
  label.title = name ? name + " · " + status : "";
  label.setAttribute("aria-label", label.title || label.textContent);
  label.classList.toggle("offline", !online);
}

function setPaired(value, name) {
  paired = value;
  $("gate").hidden = value;
  $("workspace").hidden = !value;
  if (value) {
    deviceName = name || deviceName || defaultDeviceName(deviceScope);
    rememberDevice(deviceName);
    setConnection(!$("network-status").classList.contains("offline"));
    notice(pairNotice, "");
    $("pair-code").value = "";
    startNew(provider);
    const retryId = savedQuotaReset();
    if (retryId) quotaResetAttempt = {id: retryId, error: "上一次重置结果尚未确认。"};
    syncQuotaPage();
    refreshHistories(false);
    syncRunning();
  } else {
    stopWatchingRun();
    drawer(false);
    closePicker();
    if (settingsDialog.open) settingsDialog.close();
    if (filePreviewDialog.open) filePreviewDialog.close();
    syncQuotaPage();
    tokenReport = null;
    renderTokenUsage();
    quotaResetAttempt = null;
    quotaResetNotice = "";
    ++inventoryEpoch;
    historyLoading = false;
    clearFiles(view);
    for (const {draft} of recoveredWithdrawals.values()) releaseFiles(draft.files);
    recoveredWithdrawals.clear();
    view = emptyView(provider);
    histories = new Map();
    models = new Map();
    modelLoading = new Set();
    modelErrors.clear();
    closeComposerSettings();
    closeSessionSurfaces(false,true);
    running = [];
    renderQuestion();
  }
}

function syncQuotaPage() {
  const section = paired ? {"#usage": "tokens", "#quota": "quota"}[location.hash] ?? null : null;
  if (section === usageSection) return;
  const open = section !== null;
  const entering = open && !usageSection;
  usageSection = section;
  ++quotaRequest;
  ++tokenRequest;
  if (entering) {
    chatScroll = {view, top: conversation.scrollTop};
    quotaProvider = view.provider;
    drawer(false);
    closePicker();
    closeComposerSettings();
    if (deviceDialog.open) deviceDialog.close();
    if (settingsDialog.open) settingsDialog.close();
    if (questionDialog.open) {
      questionDialog.close();
      if (questionForm) questionForm.deferred = true;
    }
  }
  quotaPage.hidden = !open;
  $("rail").hidden = open;
  $("chat-page").hidden = open;
  syncRunningSpinners();
  if (open) {
    $("token-usage").hidden = section !== "tokens";
    $("account-quota").hidden = section !== "quota";
    for (const choice of $("usage-tabs").children) {
      choice.setAttribute("aria-pressed", String(choice.dataset.usageSection === section));
    }
    labelUsageRefresh();
    $("quota-scroll").scrollTop = 0;
    if (entering) $("quota-heading").focus({preventScroll: true});
    if (section === "tokens") loadTokenUsage();
    else loadQuota();
    return;
  }
  $("quota-details").replaceChildren();
  quotaResetCount = null;
  if (quotaResetAttempt && !quotaResetAttempt.pending && !quotaResetAttempt.error) {
    quotaResetAttempt = null;
  }
  if (paired) {
    if (view.followLatest) scrollToLatest();
    else if (view === chatScroll?.view) conversation.scrollTop = chatScroll.top;
    $(narrowLayout.matches ? "open-drawer" : "open-quota").focus({preventScroll: true});
    if (questionForm?.deferred) {
      questionForm.deferred = false;
      openQuestion();
    }
  }
  chatScroll = null;
}

function labelUsageRefresh() {
  const label = t(usageSection === "tokens" ? "刷新用量" : "刷新额度");
  const refresh = $("refresh-quota");
  refresh.setAttribute("aria-label", label);
  refresh.title = label;
}

async function loadTokenUsage() {
  const request = ++tokenRequest;
  const refresh = $("refresh-quota");
  const status = $("token-status");
  refresh.disabled = true;
  refresh.classList.add("is-spinning");
  notice(status, "正在读取 Token 用量…");
  try {
    const report = await api("/usage");
    if (request !== tokenRequest) return;
    tokenReport = report;
    renderTokenUsage();
    notice(status, "");
  } catch (problem) {
    if (request === tokenRequest) {
      notice(status, problem.status === 404 ? "请更新目标电脑上的 Naro 以查看 Token 用量。" : problem.message);
    }
  } finally {
    if (request === tokenRequest) {
      refresh.disabled = false;
      refresh.classList.remove("is-spinning");
    }
  }
}

function renderTokenUsage() {
  const details = $("token-details");
  const period = tokenReport?.windows?.find(window => window.period === tokenPeriod);
  if (!period) {
    details.replaceChildren();
    return;
  }
  const grandTotal = period.total ? period.total.input_tokens + period.total.output_tokens : 0;
  const anyPartial = period.providers.some(provider => provider.partial);

  const hero = element("section", "", "token-hero");
  const heroHead = element("div", "", "token-hero-head");
  heroHead.append(element("h2", t("总消耗")));
  if (anyPartial) {
    heroHead.append(element("p", t("部分 Token 记录暂不可用。"), "token-note"));
  }
  hero.append(heroHead);
  if (period.total) {
    const total = element("strong", formatTokenCount(grandTotal), "token-total");
    total.title = quotaAmount.format(grandTotal) + " Tokens";
    total.append(element("small", "Tokens"));
    hero.append(total);
    if (grandTotal > 0) {
      const bar = element("div", "", "token-share-bar");
      bar.setAttribute("aria-hidden", "true");
      for (const provider of period.providers) {
        const count = provider.counts ? provider.counts.input_tokens + provider.counts.output_tokens : 0;
        if (count <= 0) continue;
        const seg = element("span", "", "token-share-seg");
        seg.dataset.provider = provider.provider;
        seg.style.flex = String(count);
        const share = formatTokenShare(count, grandTotal);
        seg.title = `${engines[provider.provider].label} · ${quotaAmount.format(count)} Tokens (${share})`;
        bar.append(seg);
      }
      hero.append(bar);
    }
    hero.append(element("div", "", "token-hero-divider"), tokenMetricsList(period.total));
  } else {
    hero.append(element("p", t("暂无 Token 记录。"), "token-note"));
  }

  const sectionTitle = element("h2", t("各引擎明细"), "token-section-title");
  const breakdown = element("section", "", "token-breakdown");
  for (const provider of period.providers) {
    breakdown.append(tokenProviderRow(provider, grandTotal));
  }
  details.replaceChildren(hero, sectionTitle, breakdown);
}

function formatTokenCount(count) {
  if (count < 1_000_000) return quotaAmount.format(count);
  // Use B when a value would round to 1,000M.
  const billions = count >= 999_995_000;
  const step = billions ? 10_000_000 : 10_000;
  const hundredths = Math.round(count / step);
  return quotaAmount.format(hundredths / 100) + (billions ? "B" : "M");
}

function formatTokenShare(part, total) {
  if (!total || !part) return "";
  const tenths = Math.round((part * 1000) / total);
  if (tenths === 0) return "<0.1%";
  return tenths % 10 === 0 ? `${tenths / 10}%` : `${(tenths / 10).toFixed(1)}%`;
}

function tokenMetricsList(counts) {
  const metrics = element("dl", "", "token-metrics");
  const fields = [
    ["输入 Token", counts.input_tokens], ["输出 Token", counts.output_tokens],
    ["缓存读取", counts.cache_read_tokens], ["缓存写入", counts.cache_write_tokens],
  ];
  for (const [name, count] of fields) {
    const item = element("div");
    const value = element("dd", formatTokenCount(count), count === 0 ? "is-zero" : "");
    value.title = quotaAmount.format(count) + " Tokens";
    item.append(element("dt", t(name)), value);
    metrics.append(item);
  }
  return metrics;
}

function tokenProviderRow(provider, grandTotal) {
  const row = element("div", "", "token-provider-row");
  const counts = provider.counts;
  const count = counts ? counts.input_tokens + counts.output_tokens : null;
  const head = element("div", "", "token-provider-head");
  const name = element("span", "", "token-provider-name");
  const dot = element("span", "", count && count > 0 ? "token-dot" : "token-dot is-empty");
  dot.dataset.provider = provider.provider;
  dot.setAttribute("aria-hidden", "true");
  name.append(dot, engines[provider.provider].label);
  head.append(name);

  if (count !== null) {
    const right = element("span", "", "token-provider-total");
    const share = formatTokenShare(count, grandTotal);
    if (share) right.append(element("span", share, "token-share-pill"));
    const value = element("strong", formatTokenCount(count), count === 0 ? "is-zero" : "");
    value.title = quotaAmount.format(count) + " Tokens";
    right.append(value, element("small", "Tokens"));
    head.append(right);
  } else {
    head.append(element("span", t("暂无 Token 记录。"), "token-note"));
  }
  row.append(head);

  if (counts) {
    if (grandTotal > 0) {
      const bar = element("div", "", "token-row-bar");
      bar.setAttribute("aria-hidden", "true");
      if (count > 0) {
        const fill = element("div", "", "token-row-bar-fill");
        fill.dataset.provider = provider.provider;
        fill.style.width = `${Math.max(1.2, (count / grandTotal) * 100)}%`;
        bar.append(fill);
      }
      row.append(bar);
    }
    row.append(tokenMetricsList(counts));
  }
  if (provider.partial) {
    row.append(element("p", t("部分 Token 记录暂不可用。"), "token-note"));
  }
  return row;
}

async function loadQuota() {
  const request = ++quotaRequest;
  const selected = quotaProvider;
  const refresh = $("refresh-quota");
  const status = $("quota-status");
  const details = $("quota-details");
  refresh.disabled = true;
  refresh.classList.add("is-spinning");
  for (const choice of $("quota-engines").children) {
    choice.setAttribute("aria-pressed", String(choice.value === selected));
  }
  if (details.dataset.provider !== selected) {
    details.replaceChildren();
    quotaResetCount = null;
  }
  renderQuotaReset();
  notice(status, "正在读取额度…");
  try {
    const quota = await api("/quotas/" + selected);
    if (request !== quotaRequest || quotaPage.hidden) return;
    notice(status, quota.limits.length ? "" : "暂未返回用量窗口。");
    const rows = document.createDocumentFragment();
    const animatedMeters = [];
    for (const limit of quota.limits) {
      const group = element("section", "", "quota-group");
      group.append(element("h2", t(limit.name)));
      for (const window of limit.windows) {
        const row = element("div", "", "quota-window");
        const heading = element("div", "", "quota-row");
        const percent = window.remaining_percent;
        const remaining = percent === null ? quotaAmount.format(window.remaining_amount) : percent + "%";
        heading.append(element("span", quotaWindow(window.label)), element("strong", t("剩余 {amount}", {amount: remaining})));
        row.append(heading);
        if (percent !== null) {
          const meter = element("progress");
          meter.max = 100;
          meter.value = reducedMotion.matches ? percent : 0;
          meter.setAttribute("aria-hidden", "true");
          row.append(meter);
          if (!reducedMotion.matches && percent > 0) animatedMeters.push([meter, percent]);
        }
        if (window.resets_at !== null) {
          const reset = new Date(window.resets_at * 1000);
          if (!Number.isNaN(reset.getTime())) {
            const time = element("time", t("{time} 重置", {time: quotaDate.format(reset)}));
            time.dateTime = reset.toISOString();
            row.append(time);
          }
        }
        group.append(row);
      }
      rows.append(group);
    }
    const credits = quota.unlimited_credits ? t("不限") : quota.credit_balance;
    if (credits !== null) rows.append(quotaSummary(t("积分余额"), credits));
    details.replaceChildren(rows);
    if (animatedMeters.length) {
      const start = performance.now();
      const duration = 340;
      const step = now => {
        const t = Math.min(1, (now - start) / duration);
        const eased = 1 - (1 - t) ** 3;
        for (const [meter, target] of animatedMeters) meter.value = target * eased;
        if (t < 1 && request === quotaRequest && !quotaPage.hidden) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }
    details.dataset.provider = selected;
    quotaResetCount = quota.reset_credits;
  } catch (problem) {
    if (request === quotaRequest && !quotaPage.hidden) {
      quotaResetCount = null;
      notice(status, problem.message);
    }
  } finally {
    if (request === quotaRequest) {
      refresh.disabled = false;
      refresh.classList.remove("is-spinning");
      renderQuotaReset();
    }
  }
}

function quotaWindow(label) {
  // Older hosts send display labels with the duration already interpolated.
  const parts = label.match(/^(?:(.+) · )?(\d+) (分钟|小时|天)$/);
  if (!parts) return t(label);
  const [, name, count, unit] = parts;
  const duration = t("{count} " + unit, {count});
  return name ? t(name) + " · " + duration : duration;
}

function renderQuotaReset() {
  const attempt = quotaResetAttempt;
  $("quota-reset").hidden = quotaProvider !== "codex" ||
    (quotaResetCount === null && !attempt && !quotaResetNotice);
  notice($("quota-reset-count"), quotaResetCount === null ? "" : t("可用重置次数 {count}", {count: quotaResetCount}));
  const start = $("start-quota-reset");
  start.disabled = !(quotaResetCount > 0) || $("refresh-quota").disabled;
  start.hidden = Boolean(attempt) || quotaResetCount === null;
  start.setAttribute("aria-expanded", String(Boolean(attempt)));
  $("quota-reset-confirm").hidden = !attempt;
  $("cancel-quota-reset").hidden = Boolean(attempt?.pending || attempt?.error);
  const confirm = $("confirm-quota-reset");
  confirm.disabled = Boolean(attempt?.pending);
  setText(confirm, t(attempt?.pending ? "正在重置…" : attempt?.error ? "重试重置" : "确认重置"));
  notice($("quota-reset-status"), attempt?.error || quotaResetNotice);
}

function savedQuotaReset() {
  try { return sessionStorage.getItem(quotaResetKey); }
  catch { return null; }
}

function rememberQuotaReset(id) {
  try {
    if (id) sessionStorage.setItem(quotaResetKey, id);
    else sessionStorage.removeItem(quotaResetKey);
  } catch {}
}

async function submitQuotaReset() {
  const attempt = quotaResetAttempt;
  if (!attempt || attempt.pending) return;
  rememberQuotaReset(attempt.id);
  attempt.pending = true;
  attempt.error = "";
  renderQuotaReset();
  try {
    const result = await api("/quotas/codex/reset", {idempotency_key: attempt.id});
    if (savedQuotaReset() === attempt.id) rememberQuotaReset(null);
    if (quotaResetAttempt !== attempt) return;
    quotaResetAttempt = null;
    quotaResetNotice = result.message;
    quotaResetCount = null;
    if (usageSection === "quota" && quotaProvider === "codex") loadQuota();
  } catch (problem) {
    attempt.error = problem.message;
  } finally {
    attempt.pending = false;
    renderQuotaReset();
  }
}

function quotaSummary(label, value) {
  const row = element("div", "", "quota-row quota-summary");
  row.append(element("span", label), element("strong", value));
  return row;
}

async function pair(code) {
  const submit = $("pair-submit");
  if (!/^\d{8}$/.test(code) || submit.disabled) return false;
  submit.disabled = true;
  notice(pairNotice, "");
  try {
    const access = await api("/pair", {code});
    setPaired(true, access.name);
    return true;
  } catch (problem) {
    notice(pairNotice, problem.message);
    return false;
  } finally {
    submit.disabled = false;
  }
}

function button(text, className, action) {
  const node = element("button", text, className);
  node.type = "button";
  if (action) node.addEventListener("click", action);
  return node;
}

function rotateRunningSpinners(now) {
  // Use the page clock so a newly attached row starts at the current angle.
  const angle = (now % runningSpinnerPeriod) / runningSpinnerPeriod;
  runningList.style.setProperty("--running-angle", `${angle}turn`);
  runningSpinnerFrame = requestAnimationFrame(rotateRunningSpinners);
}

function syncRunningSpinners() {
  const visible = paired && !usageSection && !document.hidden && runningList.firstElementChild &&
    (!narrowLayout.matches || $("rail").classList.contains("open"));
  if (visible && !reducedMotion.matches) {
    if (runningSpinnerFrame === null) rotateRunningSpinners(performance.now());
  } else if (runningSpinnerFrame !== null) {
    cancelAnimationFrame(runningSpinnerFrame);
    runningSpinnerFrame = null;
  }
}

function drawer(open) {
  open = open && narrowLayout.matches;
  if (!open && narrowLayout.matches && $("rail").classList.contains("open")) {
    $("rail").classList.add("closing");
    setTimeout(() => $("rail").classList.remove("closing"),450);
  }
  $("rail").classList.toggle("open", open);
  $("scrim").hidden = !open;
  $("open-drawer").setAttribute("aria-expanded", String(open));
  document.querySelector(".main").inert = open;
  syncRunningSpinners();
  if (open) $("close-drawer").focus({preventScroll: true});
}

function setupRailResize() {
  const workspace = $("workspace");
  const rail = $("rail");
  const handle = $("rail-resizer");
  const storageKey = "narolink.rail-width";
  const style = getComputedStyle(workspace);
  const minimum = parseFloat(style.getPropertyValue("--rail-min-width"));
  const maximum = parseFloat(style.getPropertyValue("--rail-max-width"));
  const mainMinimum = parseFloat(style.getPropertyValue("--main-min-width"));
  let preferredWidth = 0;
  let drag = null;

  const maxWidth = () => Math.max(minimum, Math.min(maximum, workspace.clientWidth - mainMinimum));
  const clampWidth = width => Math.round(Math.max(minimum, Math.min(maxWidth(), width)));
  const drawWidth = width => workspace.style.setProperty("--rail-width", width ? `${width}px` : "");
  try {
    const saved = Number(localStorage.getItem(storageKey));
    if (Number.isFinite(saved) && saved > 0) preferredWidth = Math.round(Math.max(minimum, Math.min(maximum, saved)));
  } catch {}
  drawWidth(preferredWidth);

  function rememberWidth(width) {
    preferredWidth = width;
    drawWidth(width);
    try { localStorage.setItem(storageKey, String(width)); } catch {}
  }

  function endDrag() {
    if (!drag) return;
    const {pointerId} = drag;
    drag = null;
    drawWidth(preferredWidth);
    workspace.classList.remove("is-resizing-rail");
    if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
  }

  handle.addEventListener("pointerdown", event => {
    if (event.button !== 0 || drag) return;
    drag = {pointerId: event.pointerId, startX: event.clientX, width: rail.offsetWidth};
    handle.setPointerCapture(event.pointerId);
    workspace.classList.add("is-resizing-rail");
  });
  handle.addEventListener("pointermove", event => {
    if (event.pointerId === drag?.pointerId) drawWidth(clampWidth(drag.width + event.clientX - drag.startX));
  });
  handle.addEventListener("pointerup", event => {
    if (event.pointerId !== drag?.pointerId) return;
    const width = clampWidth(drag.width + event.clientX - drag.startX);
    if (width !== drag.width) rememberWidth(width);
    endDrag();
  });
  handle.addEventListener("lostpointercapture", event => {
    if (event.pointerId === drag?.pointerId) endDrag();
  });
  narrowLayout.addEventListener("change", endDrag);

  handle.addEventListener("keydown", event => {
    const adjustments = {ArrowLeft: -16, ArrowRight: 16, Home: -Infinity, End: Infinity};
    if (!Object.hasOwn(adjustments, event.key)) return;
    event.preventDefault();
    const current = rail.offsetWidth;
    const width = clampWidth(current + adjustments[event.key]);
    if (width !== current) rememberWidth(width);
  });

  const updateValues = () => {
    if (narrowLayout.matches || rail.offsetWidth === 0) return;
    handle.setAttribute("aria-valuemax", String(maxWidth()));
    handle.setAttribute("aria-valuenow", String(rail.offsetWidth));
  };
  handle.setAttribute("aria-valuemin", String(minimum));
  window.addEventListener("resize", updateValues);
  if ("ResizeObserver" in window) new ResizeObserver(updateValues).observe(rail);
}

function enableConversationSwipe() {
  let start = null;
  const canOpenDrawer = () => narrowLayout.matches && paired && quotaPage.hidden && $("scrim").hidden &&
    !document.querySelector("dialog[open]") && (window.visualViewport?.scale ?? 1) < 1.05 &&
    window.getSelection()?.isCollapsed !== false;

  document.addEventListener("touchstart", event => {
    start = null;
    if (event.touches.length !== 1 || !conversation.contains(event.target) || !canOpenDrawer() ||
        event.target.closest("button, a, input, textarea, select, label, summary, [contenteditable], pre, table")) return;
    const {identifier, clientX, clientY} = event.touches[0];
    start = {identifier, x: clientX, y: clientY};
  }, {passive: true});
  conversation.addEventListener("touchmove", event => {
    if (!start) return;
    const touch = event.touches[0];
    if (event.touches.length !== 1 || touch.identifier !== start.identifier) {
      start = null;
      return;
    }
    const dx = touch.clientX - start.x;
    const dy = Math.abs(touch.clientY - start.y);
    // Once the drag goes vertically, keep it a scroll even if it turns to the right.
    if (dx <= -12 || (dy >= 12 && dy >= dx)) start = null;
  }, {passive: true});
  conversation.addEventListener("touchend", event => {
    const origin = start;
    start = null;
    if (!origin || event.touches.length || !canOpenDrawer()) return;
    const {identifier, clientX, clientY} = event.changedTouches[0];
    const dx = clientX - origin.x;
    const dy = Math.abs(clientY - origin.y);
    if (identifier === origin.identifier && dx >= 64 && dx >= 1.5 * dy) drawer(true);
  }, {passive: true});
  conversation.addEventListener("touchcancel", () => { start = null; }, {passive: true});
}

function renderFilters() {
  $("filters").replaceChildren(...["all", ...providers].map(id => {
    const node = button(id === "all" ? t("全部") : engines[id].short, "", () => {
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
    const node = button(engines[id].label, "provider-choice", () => {
      if (view.busy || id === view.provider) return;
      closeComposerSettings();
      provider = id;
      view.provider = id;
      if (id !== "codex" && view.location.startsWith("project:")) {
        view.location = view.cwd ? "folder:" + view.cwd : "";
      }
      restoreModelPreference(view);
      restorePermissionMode(view);
      view.collaborationMode = null;
      renderHeader();
      renderEngineChoices();
      renderDestinations();
      renderControls();
      loadModels(id);
    });
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

function nativeProjects() {
  return histories.get("codex")?.projects || [];
}

function projectsByRoot(catalog) {
  const byRoot = new Map();
  for (const project of catalog) {
    for (const root of new Set(project.roots)) {
      byRoot.set(root, byRoot.has(root) ? null : project);
    }
  }
  return byRoot;
}

function setPicker(trigger, options, selected) {
  const [value, label] = options.find(([value]) => value === selected) || options[0] || ["", ""];
  const key = JSON.stringify([options, value]);
  if (destinationOptions.get(trigger)?.key === key) return;
  destinationOptions.set(trigger, {options, key});
  trigger.value = value;
  trigger.querySelector("[id]").textContent = label;
  if (pickerTrigger === trigger) renderPickerOptions();
}

function renderPickerOptions() {
  const trigger = pickerTrigger;
  const options = destinationOptions.get(trigger).options;
  const search = $("picker-search");
  const searchable = options.length > 8;
  $("picker-search-row").hidden = !searchable;
  const query = searchable ? search.value.trim().toLowerCase() : "";
  const list = $("picker-options");
  const focused = list.contains(document.activeElement) ? document.activeElement.value : null;
  const rows = options.flatMap(([value, label, path = ""]) => {
    if (!value && (!trigger.value || query)) return [];
    const title = value ? label : t("清空选择");
    if (query && ![title, path].some(text => text.toLowerCase().includes(query))) return [];
    const row = button("", "picker-option", () => chooseDestination(trigger, value));
    row.value = value;
    row.setAttribute("role", "option");
    row.setAttribute("aria-selected", String(value === trigger.value));
    row.tabIndex = -1;
    const text = element("span", "", "picker-option-copy");
    text.append(element("strong", title));
    if (path) {
      text.append(element("small", path));
      row.title = path;
    }
    row.append(text);
    if (value === trigger.value) {
      const check = element("span", "", "picker-check");
      check.setAttribute("aria-hidden", "true");
      row.append(check);
    }
    return [row];
  });
  list.replaceChildren(...rows);
  $("picker-empty").hidden = rows.length > 0;
  const active = rows.find(row => row.value === focused) ||
    rows.find(row => row.value === trigger.value) || rows[0];
  if (active) {
    active.tabIndex = 0;
    if (focused !== null) active.focus();
  }
}

function positionPicker() {
  if (!pickerTrigger || narrowLayout.matches) return;
  const rect = pickerTrigger.getBoundingClientRect();
  const below = window.innerHeight - rect.bottom - 14;
  const above = rect.top - 14;
  const openAbove = below < 230 && above > below;
  picker.classList.toggle("above", openAbove);
  picker.style.setProperty("--picker-left", rect.left + "px");
  picker.style.setProperty("--picker-edge", (openAbove ? window.innerHeight - rect.top + 6 : rect.bottom + 6) + "px");
  picker.style.setProperty("--picker-width", rect.width + "px");
  picker.style.setProperty("--picker-room", (openAbove ? above : below) + "px");
}

function openPicker(trigger) {
  if (trigger.disabled || pickerTrigger) return;
  closeComposerSettings();
  pickerTrigger = trigger;
  $("picker-heading").textContent = $(trigger.dataset.label).textContent;
  $("picker-search").value = "";
  renderPickerOptions();
  positionPicker();
  trigger.setAttribute("aria-expanded", "true");
  picker.showModal();
  $("picker-options").querySelector('[tabindex="0"]')?.focus();
}

function closePicker(restoreFocus = false) {
  if (!pickerTrigger) return;
  const trigger = pickerTrigger;
  pickerTrigger = null;
  trigger.setAttribute("aria-expanded", "false");
  picker.close();
  if (restoreFocus) trigger.focus({preventScroll: true});
}

function chooseDestination(trigger, value) {
  const custom = trigger.id === "project-select" && value === "custom";
  closePicker(!custom);
  if (trigger.id === "project-select") {
    if (value !== trigger.value) {
      view.location = value;
      if (value.startsWith("folder:")) view.cwd = value.slice(7);
      else if (!value.startsWith("project:")) view.cwd = "";
      renderDestinations();
      renderHeader();
    }
    if (custom) $("directory").focus();
  } else {
    view.cwd = value;
    renderDestinations();
  }
  renderControls();
}

function renderDestinations() {
  const projects = view.provider === "codex" ? nativeProjects().filter(project => project.roots.length) : [];
  const roots = new Set(projects.flatMap(project => project.roots));
  if (view.location.startsWith("project:") && histories.has("codex") &&
      !projects.some(project => "project:" + project.id === view.location)) {
    view.location = "";
  }
  if (!view.location && view.cwd) {
    const matches = projects.filter(project => project.roots.includes(view.cwd));
    if (matches.length === 1 || !roots.has(view.cwd)) {
      view.location = matches.length ? "project:" + matches[0].id : "folder:" + view.cwd;
    }
  }
  const folders = new Set(listedSessions().map(session => session.cwd).filter(Boolean).slice(0, 150));
  if (view.location.startsWith("folder:")) folders.add(view.cwd);
  const options = [["", t("选择项目或文件夹")], ...projects.map(project => ["project:" + project.id, project.name, project.roots[0]]),
    ...[...folders].filter(path => !roots.has(path) || view.location === "folder:" + path)
      .map(path => ["folder:" + path, folderName(path) || path, path]), ["custom", t("输入其他路径…")]];
  setPicker($("project-select"), options, view.location);
  const selected = projects.find(project => "project:" + project.id === view.location);
  if (selected && !selected.roots.includes(view.cwd)) view.cwd = selected.roots[0];
  $("project-root-row").hidden = !selected || selected.roots.length <= 1;
  setPicker($("project-root"), (selected?.roots || []).map(path => [path, folderName(path) || path, path]), view.cwd);
  $("custom-project").hidden = view.location !== "custom";
  if (view.location === "custom" && $("directory").value !== view.cwd) $("directory").value = view.cwd;
}

function renderSidebar() {
  renderNativeSearch();
  const query = $("session-search").value.trim().toLowerCase();
  const catalog = nativeProjects();
  const native = new Map(catalog.map(project => [project.id, project]));
  const byRoot = projectsByRoot(catalog);
  const errors = (filter === "all" ? providers : [filter]).map(id => {
    const data = histories.get(id);
    return data?.error ? engines[id].label + " · " + t(data.error) : data?.warning;
  }).filter(Boolean);
  const projects = new Map();
  for (const session of listedSessions()) {
    if (filter !== "all" && session.provider !== filter) continue;
    const useRoot = !session.project_id && (session.provider === "codex" || filter === "all");
    const project = useRoot ? byRoot.get(session.cwd) : native.get(session.project_id);
    if (query && ![session.title, session.cwd, project?.name, engines[session.provider]?.label]
      .some(text => text?.toLowerCase().includes(query))) continue;
    const path = project?.roots[0] || session.cwd || "";
    const key = project ? "project:" + project.id : "folder:" + path;
    if (!projects.has(key)) projects.set(key, {project, path, rows: []});
    projects.get(key).rows.push(session);
  }
  if (filter === "all" || filter === "codex") {
    for (const project of catalog) {
      const key = "project:" + project.id;
      if (!projects.has(key) && project.roots.length && (!query || project.name.toLowerCase().includes(query))) {
        projects.set(key, {project, path: project.roots[0], rows: []});
      }
    }
  }
  notice($("inventory-notice"), errors[0] || (!projects.size && !running.length &&
    (historyLoading ? "正在读取电脑上的会话…" : query ? "没有匹配的项目或会话" : "暂无本机项目或会话")));
  const names = new Map();
  for (const {project, path} of projects.values()) {
    const name = project?.name || folderName(path);
    if (!names.has(name)) names.set(name, new Map());
    const paths = names.get(name);
    paths.set(path, (paths.get(path) || 0) + 1);
  }
  const groups = [...projects].map(([key, {project, path, rows}], index) => {
    const name = project?.name || folderName(path);
    const paths = names.get(name);
    const kind = paths.get(path) > 1 ? t(project ? "Codex 项目" : "目录会话") : "";
    const description = [kind, path].filter(Boolean).join(" · ");
    const active = rows.some(session => bySession(session, view));
    const expanded = projectsOpen.get(key) ?? (index === 0 || active || Boolean(query));
    const group = element("section", "", "project");
    const list = element("div", "", "project-sessions");
    list.id = "project-sessions-" + index;
    list.hidden = !expanded;
    const header = button("", "project-heading", () => {
      const open = list.hidden;
      projectsOpen.set(key, open);
      list.hidden = !open;
      header.setAttribute("aria-expanded", String(open));
    });
    header.title = path;
    header.setAttribute("aria-expanded", String(expanded));
    header.setAttribute("aria-controls", list.id);
    const title = element("span", "", "project-title");
    title.append(element("strong", name || path || t("会话")));
    if (name && (paths.size > 1 || kind)) {
      title.append(element("small", description, "project-path"));
    }
    header.append(element("span", "", "chevron"), element("span", "", "folder-mark"),
      title, element("span", String(rows.length), "project-count"));
    list.append(...rows.map(session => {
      const row = button("", "session", () => {
        projectsOpen.set(key, true);
        openSession(session);
      });
      row.setAttribute("aria-current", String(bySession(session, view)));
      row.title = session.title + "\n" + session.cwd;
      const copy = element("span", "", "session-copy");
      const date = session.updated ? new Date(session.updated * 1000).toLocaleDateString(language, {month: "short", day: "numeric"}) : "";
      copy.append(element("strong", session.title),
        element("small", [engines[session.provider].label, date].filter(Boolean).join(" · ")));
      row.append(copy);
      return row;
    }));
    const heading = element("div", "", "project-bar");
    heading.append(header);
    if (path) {
      const newChat = button("", "project-new", () => {
        projectsOpen.set(key, true);
        startNew(project ? "codex" : filter === "all" ? view.provider : filter, project?.roots[0] || path, project?.id);
        $("prompt").focus({preventScroll: true});
      });
      const destination = paths.size > 1 || kind ? path : name || path;
      const label = kind ? t("在 {project} 新建会话（{kind}）", {project: destination, kind})
        : t("在 {project} 新建会话", {project: destination});
      newChat.setAttribute("aria-label", label);
      newChat.title = label;
      newChat.append(element("span", "", "plus-mark"));
      heading.append(newChat);
    }
    group.append(heading, list);
    return group;
  });
  $("session-list").replaceChildren(...groups);
  const tasks = running.filter(task => filter === "all" || task.provider === filter).map(task => {
    const current = view.run === task.id;
    return [task.id, [task, current], row => {
      if (!row) {
        row = button("", "session running");
        const copy = element("span", "", "session-copy");
        copy.append(element("strong"), element("small"));
        const spinner = element("span", "", "running-spinner");
        spinner.setAttribute("aria-hidden", "true");
        row.append(copy, spinner);
      }
      const engine = engines[task.provider].label;
      row.onclick = () => openRunning(task);
      row.setAttribute("aria-current", String(current));
      row.setAttribute("aria-label", t("{title}，{engine}，正在运行", {title: task.title, engine}));
      const copy = row.firstElementChild;
      setText(copy.firstElementChild, task.title);
      setText(copy.lastElementChild, engine);
      return row;
    }];
  });
  renderList(runningList, tasks);
  syncRunningSpinners();
  if (!view.sessionId && !view.run) {
    renderDestinations();
    renderHeader();
    renderControls();
  }
}

async function refreshHistories(force) {
  const epoch = ++inventoryEpoch;
  historyLoading = true;
  $("refresh").disabled = true;
  $("refresh").classList.add("is-spinning");
  renderSidebar();
  await Promise.all(providers.map(async id => {
    let data;
    try {
      data = await api("/sessions/" + id + (force ? "?refresh=true" : ""));
    } catch (problem) {
      data = {sessions: histories.get(id)?.sessions || [], error: problem.message};
    }
    if (epoch === inventoryEpoch) {
      histories.set(id, data);
      if (view.provider === id && view.sessionId && !view.busy) {
        const session = data.sessions.find(session => session.id === view.sessionId);
        if (session && session.title !== view.data?.title) {
          view.data = {...(view.data ?? session), title: session.title};
          renderHeader();
        }
      }
      renderSidebar();
    }
  }));
  if (epoch === inventoryEpoch) {
    historyLoading = false;
    $("refresh").disabled = false;
    $("refresh").classList.remove("is-spinning");
    renderSidebar();
  }
}

async function loadModels(id, refresh = false) {
  if (!paired) return;
  const pending = modelLoading;
  if (pending.has(id)) {
    renderModels();
    return;
  }
  pending.add(id);
  const previous = models.get(id);
  const active = selectedModel(view, previous);
  const missingModel = id === "codex" && active && previous?.models?.some(model => model.id === active) === false;
  modelErrors.delete(id);
  renderModels();
  try {
    const catalog = await api("/models/" + id + "?settings=true" + (refresh || missingModel ? "&refresh=true" : ""));
    if (pending === modelLoading) {
      const listing = Array.isArray(catalog) ? {models: catalog} : catalog;
      models.set(id, listing);
      if (view.provider === id && isNewView(view)) retainSupportedModelSelection(view, listing);
    }
  } catch (problem) {
    if (pending === modelLoading) modelErrors.set(id, problem.message);
  } finally {
    pending.delete(id);
    if (pending === modelLoading && view.provider === id) renderModels();
  }
}

function selectedModel(target, catalog = models.get(target.provider)) {
  const initial = !target.sessionId && !target.run ? catalog?.default_model : null;
  return [target.model, target.runModel, target.historyModel, target.data?.model, initial]
    .find(id => id && id.toLowerCase() !== "default") ?? null;
}

function setModel(id) {
  const changed = id !== selectedModel(view);
  view.model = id;
  if (changed) {
    view.effort = null;
    view.fast = null;
  }
  rememberModelPreference();
}

function rememberModelPreference() {
  const {provider, model, effort, fast} = view;
  try {
    localStorage.setItem(modelPreferenceKey(provider), JSON.stringify({model, effort, fast}));
  } catch {}
}

function restoreModelPreference(target) {
  let saved;
  try { saved = JSON.parse(localStorage.getItem(modelPreferenceKey(target.provider))); } catch {}
  target.model = typeof saved?.model === "string" && saved.model ? saved.model : null;
  target.effort = target.model && typeof saved.effort === "string" ? saved.effort : null;
  target.fast = target.model && typeof saved.fast === "boolean" ? saved.fast : null;
  const catalog = models.get(target.provider);
  if (catalog) retainSupportedModelSelection(target, catalog);
}

function permissionTarget(target) {
  if (target.sessionId) return {session_id: target.sessionId};
  return target.run ? {run_id: target.run} : {};
}

function restorePermissionMode(target) {
  const query = new URLSearchParams(permissionTarget(target));
  const route = "/permissions/" + target.provider + (query.size ? "?" + query : "");
  target.permissionMode = undefined;
  const request = permissionWrite.then(() => api(route)).catch(problem => {
    // A newer relay can still serve a device with an older desktop app.
    if (problem.status !== 404) throw problem;
    return undefined;
  }).then(mode => {
    if (target.permissionLoad === request) {
      target.permissionPersistence = mode !== undefined;
      target.permissionMode = mode ?? null;
    }
    return true;
  }).catch(problem => {
    if (view === target && target.permissionLoad === request) notice(composerNotice, problem.message);
    return false;
  }).finally(() => {
    if (target.permissionLoad === request) {
      target.permissionLoad = null;
      if (view === target) renderControls();
    }
  });
  target.permissionLoad = request;
  return request;
}

function savePermissionMode(target, mode) {
  if (!target.permissionPersistence) {
    target.permissionMode = mode;
    return;
  }
  const provider = target.provider;
  const selection = {...permissionTarget(target), permission_mode: mode};
  target.permissionSaving = true;
  permissionWrite = permissionWrite.then(() => api("/permissions/" + provider, selection)).then(() => {
    if (target.provider === provider) target.permissionMode = mode;
  }).catch(problem => {
    if (view === target) notice(composerNotice, problem.message);
  }).finally(() => {
    target.permissionSaving = false;
    if (view === target) renderControls();
  });
  renderControls();
}

function selectedPermission(target, catalog) {
  const mode = target.permissionMode;
  return !catalog || catalog.permission_modes?.some(choice => choice.id === mode) ? mode : null;
}

function retainSupportedModelSelection(target, catalog) {
  if (!target.model) return;
  const model = catalog.models?.find(item => item.id === target.model);
  if (!model) target.model = null;
  if (!model?.reasoning_efforts?.some(item => item.id === target.effort)) target.effort = null;
  if (!model?.fast_mode) target.fast = null;
}

function closeComposerSettings(restoreFocus = false) {
  const open = composerSettings.find(([, panel]) => !panel.hidden);
  if (!open) return false;
  const [trigger, panel] = open;
  panel.hidden = true;
  trigger.setAttribute("aria-expanded", "false");
  if (restoreFocus) trigger.focus();
  return true;
}

function modelChoices(label, choices, selected, choose, className = "") {
  const section = element("div", "", "model-section " + className);
  if (label) section.append(element("h3", label));
  const options = element("div", "", "model-choice-list");
  for (const [value, name] of choices) {
    const choice = button(name, "model-choice", () => {
      choose(value);
      renderModels();
    });
    choice.dataset.choice = (label ?? "") + ":" + String(value);
    choice.setAttribute("aria-pressed", String(value === selected));
    if (value === selected) {
      const check = element("span", "✓", "choice-check");
      check.setAttribute("aria-hidden", "true");
      choice.append(check);
    }
    options.append(choice);
  }
  section.append(options);
  return section;
}

const effortPalettes = [
  ["#0d645d", "#21a78d", "#83e6c3"],
  ["#164b91", "#3484e5", "#91cafa"],
  ["#322173", "#6450d6", "#ab9cf4"],
  ["#4c1687", "#8e2be4", "#c290f9"],
  ["#681580", "#af2deb", "#e0a0fb"],
];

function effortPaletteIndex(id, index, count) {
  const known = {none: 0, minimal: 0, low: 0, medium: 1, high: 2, xhigh: 3, max: 4};
  return Object.hasOwn(known, id) ? known[id] : Math.round(Math.max(index, 0) / Math.max(count - 1, 1) * 4);
}

function updateEffortSelection(section, selected, preview = false) {
  if (!section) return;
  if (!preview) section.dataset.selected = selected ?? "";
  const choices = [...section.querySelectorAll(".model-choice")];
  const index = choices.findIndex(choice => choice.dataset.effort === selected);
  choices.forEach((choice, position) => choice.setAttribute("aria-pressed", String(position === index)));
  const position = choices.length > 1 ? Math.max(index, 0) / (choices.length - 1) * 100 : 100;
  const palette = effortPaletteIndex(selected, index, choices.length);
  section.style.setProperty("--effort-position", `${position}%`);
  ["--effort-dark", "--effort-color", "--effort-light"].forEach((name, part) => {
    section.style.setProperty(name, effortPalettes[palette][part]);
  });
  section.style.setProperty("--effort-flow-time", `${5.6 - palette * .6}s`);
  section.classList.toggle("has-selection", index >= 0);
  const caption = section.querySelector(".effort-value-label");
  if (caption) setText(caption, index >= 0 ? choices[index].textContent.replace("✓", "").trim() : "—");
}

function enableEffortControl(section, efforts, selected) {
  const labels = section.querySelector(".model-choice-list");
  const choices = [...labels.children];
  const lastIndex = choices.length - 1;
  choices.forEach((choice, index) => { choice.dataset.effort = efforts[index].id; });
  section.style.setProperty("--effort-count", choices.length);
  const heading = section.querySelector("h3");
  const header = element("div", "", "effort-heading");
  const value = element("span", "", "effort-value");
  value.setAttribute("aria-hidden", "true");
  value.append(element("span", "", "effort-value-label"));
  heading.replaceWith(header);
  header.append(heading, value);
  const control = element("div", "", "effort-control");
  control.setAttribute("role", "group");
  control.setAttribute("aria-label", t("思考强度"));
  const rail = element("div", "", "effort-rail");
  rail.setAttribute("aria-hidden", "true");
  const fill = element("span", "", "effort-fill");
  const particles = element("span", "", "effort-particles");
  for (let index = 0; index < 22; index++) {
    const particle = element("span", "", "effort-particle");
    particle.style.setProperty("--particle-y", `${18 + (index * 37 % 65)}%`);
    particle.style.setProperty("--particle-size", `${index % 5 === 0 ? 2.8 : 1.5}px`);
    particle.style.setProperty("--particle-phase", -index / 22);
    particles.append(particle);
  }
  fill.append(particles);
  rail.append(fill);
  control.append(element("span", "", "effort-thumb"));
  labels.replaceWith(control);
  control.prepend(rail);
  control.append(labels);
  updateEffortSelection(section, selected);
  let pointer = null;
  let draggingRail = false;
  const selectAt = x => {
    const {left, width} = control.getBoundingClientRect();
    const index = draggingRail
      ? Math.round((x - left) / Math.max(width, 1) * lastIndex)
      : Math.floor((x - left) / width * choices.length);
    return Math.min(Math.max(index, 0), lastIndex);
  };
  const highlight = index => updateEffortSelection(section, efforts[index]?.id, true);
  control.addEventListener("pointerdown", event => {
    if (pointer !== null || !event.isPrimary || event.button !== 0) return;
    event.preventDefault();
    control.classList.add("using-pointer", "is-dragging");
    draggingRail = !event.target.closest(".model-choice");
    pointer = event.pointerId;
    control.setPointerCapture(pointer);
    highlight(selectAt(event.clientX));
  });
  control.addEventListener("pointermove", event => {
    if (event.pointerId === pointer) highlight(selectAt(event.clientX));
  });
  control.addEventListener("pointerup", event => {
    if (event.pointerId !== pointer) return;
    pointer = null;
    control.classList.remove("is-dragging");
    control.releasePointerCapture(event.pointerId);
    const choice = choices[selectAt(event.clientX)];
    choice.focus({preventScroll: true});
    choice.click();
  });
  const cancel = event => {
    if (event.pointerId !== pointer) return;
    pointer = null;
    control.classList.remove("is-dragging");
    updateEffortSelection(section, section.dataset.selected);
  };
  control.addEventListener("pointercancel", cancel);
  control.addEventListener("lostpointercapture", cancel);
  control.addEventListener("keydown", event => {
    control.classList.remove("using-pointer");
    const index = choices.indexOf(event.target);
    if (index < 0) return;
    let next;
    if (event.key === "ArrowLeft" || event.key === "ArrowDown") next = Math.max(index - 1, 0);
    else if (event.key === "ArrowRight" || event.key === "ArrowUp") next = Math.min(index + 1, lastIndex);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = lastIndex;
    else return;
    event.preventDefault();
    choices[next].focus({preventScroll: true});
    choices[next].click();
  });
}

function permissionIcon(mode) {
  switch (mode) {
    case "request-approval": case "manual": case "default": return "lock";
    case "full-access": case "bypassPermissions": return "unlock";
    case "approve-for-me": case "auto": return "shield";
    case "acceptEdits": case "accept-edits": return "edit";
    case "plan": return "plan";
    case "dontAsk": return "blocked";
    default: return "settings";
  }
}

function renderPermissions(catalog) {
  const modes = catalog?.permission_modes ?? [];
  const trigger = $("permissions");
  const panel = $("permission-panel");
  trigger.hidden = !modes.length || view.permissionMode === undefined;
  if (trigger.hidden) {
    if (!panel.hidden) closeComposerSettings();
    return;
  }
  const category = t(view.provider === "antigravity" ? "CLI 执行模式" : "权限");
  const defaultMode = catalog.default_permission_mode ?? null;
  const selectedMode = selectedPermission(view, catalog) ?? defaultMode;
  const permission = modes.find(mode => mode.id === selectedMode);
  const running = view.snapshot?.status === "running" ? view.snapshot.active_permission_mode : null;
  const runningLabel = running ? t("当前回复使用{mode}。", {mode: t(running.label)}) : null;
  const followsSession = defaultMode === null && view.provider !== "codex";
  const label = permission ? t(permission.label) : (followsSession && selectedMode === null ? t("跟随会话") : null);
  const description = runningLabel ?? (label ? t("{category}：{mode}", {category, mode: label}) : category);
  if (trigger.title !== description) {
    trigger.title = description;
    trigger.setAttribute("aria-label", description);
  }
  const mode = running ?? permission;
  const icon = mode ? permissionIcon(mode.id) : "lock";
  if ($("permission-icon").dataset.icon !== icon) $("permission-icon").dataset.icon = icon;
  if (panel.hidden) return;

  const disabled = view.permissionSaving || view.sending;
  const key = JSON.stringify([language, category, modes, followsSession, selectedMode, runningLabel, view.busy, disabled]);
  if (permissionPanelKey === key) return;
  permissionPanelKey = key;
  setText($("permission-heading"), category);
  const options = $("permission-options");
  const restore = options.contains(document.activeElement);
  const choices = modes.map(mode => [mode.id, t(mode.label)]);
  if (followsSession) choices.unshift([null, t("跟随会话")]);
  const section = modelChoices(null, choices, selectedMode, value => savePermissionMode(view, value));
  if (disabled) for (const choice of section.querySelectorAll("button")) choice.disabled = true;
  if (permission?.description) section.append(element("p", t(permission.description), "model-hint"));
  options.replaceChildren();
  if (runningLabel) options.append(element("p", runningLabel, "model-hint"));
  if (view.busy) options.append(element("p", t("下条开启新回复的消息使用此设置。"), "model-hint"));
  options.append(section);
  if (restore) section.querySelector('[aria-pressed="true"]')?.focus({preventScroll: true});
}

function renderModels() {
  const catalog = models.get(view.provider);
  renderPermissions(catalog);
  const choices = (catalog?.models || []).filter(model => model.id && model.id.toLowerCase() !== "default");
  const id = selectedModel(view, catalog);
  const selected = choices.find(model => model.id === id);
  const efforts = selected?.reasoning_efforts || [];
  const effort = efforts.find(item => item.id === (view.effort ?? selected?.default_reasoning_effort));
  const loading = modelLoading.has(view.provider);
  const label = selected?.label || id || t(loading || view.historyPending ? "正在读取模型…" : "选择模型");
  const summary = [label, effort && t(effort.label), view.collaborationMode === "plan" ? t("计划模式") : null]
    .filter(Boolean).join(" · ");
  setText($("model-summary"), summary);
  const speedToggle = $("model-speed");
  speedToggle.hidden = !selected?.fast_mode;
  if (!speedToggle.hidden) {
    const fast = view.fast ?? selected.default_fast_mode ?? false;
    speedToggle.setAttribute("aria-pressed", String(fast));
    speedToggle.title = t(fast ? "快速模式" : "标准速度");
    speedToggle.setAttribute("aria-label", t("快速模式"));
  }
  const trigger = $("models");
  const description = t("模型设置 · {summary}", {summary});
  if (trigger.title !== description) {
    trigger.title = description;
    trigger.setAttribute("aria-label", description);
  }
  if ($("model-panel").hidden) return;
  const refresh = $("refresh-models");
  refresh.disabled = loading;
  refresh.classList.toggle("is-spinning", loading);
  const problem = modelErrors.get(view.provider);
  const options = $("model-options");
  const key = JSON.stringify([language, view.provider, catalog, id,
    view.collaborationMode, problem, view.busy]);
  if (modelPanelKey === key) {
    updateEffortSelection(options.querySelector(".model-effort"), effort?.id);
    return;
  }
  modelPanelKey = key;
  const restore = options.contains(document.activeElement);
  const focusedChoice = restore ? document.activeElement.dataset.choice : null;
  const unique = new Map(choices.map(model => [model.id, model.label || model.id]));
  if (id && !unique.has(id)) unique.set(id, id);
  options.replaceChildren();
  if (unique.size) options.append(modelChoices(t("模型"), [...unique], id, setModel, "model-list-section"));
  if (problem) options.append(element("p", t(problem), "model-hint"));
  if (catalog?.collaboration_modes?.length) {
    options.append(modelChoices(t("协作模式"), catalog.collaboration_modes.map(mode => [mode, t(mode === "plan" ? "计划模式" : "执行")]),
      view.collaborationMode ?? "default", value => { view.collaborationMode = value; }, "model-pills"));
  }
  if (efforts.length) {
    const section = modelChoices(t("思考强度"), efforts.map(item => [item.id, t(item.label)]),
      effort?.id, value => {
        view.model = id;
        view.effort = value;
        rememberModelPreference();
      }, "model-effort");
    enableEffortControl(section, efforts, effort?.id);
    options.append(section);
  }
  if (selected && !efforts.length && !selected.fast_mode) {
    options.append(element("p", t("此模型由引擎管理思考强度和速度。"), "model-hint"));
  }
  if (restore) {
    const buttons = [...options.querySelectorAll("button")];
    (buttons.find(button => button.dataset.choice === focusedChoice) || buttons[0])?.focus();
  }
}

function startNew(engine, path = null, projectId = null) {
  closeComposerSettings();
  closePicker();
  stopWatchingRun();
  provider = engine;
  clearFiles(view);
  view = emptyView(engine);
  restoreModelPreference(view);
  restorePermissionMode(view);
  view.cwd = path ?? (localStorage.getItem(folderKey) || "");
  const lastProject = path === null && engine === "codex" ? localStorage.getItem(projectKey) : null;
  if (projectId || lastProject) view.location = "project:" + (projectId || lastProject);
  $("directory").value = "";
  $("prompt").value = "";
  notice(composerNotice, "");
  drawer(false);
  paint();
  conversation.scrollTop = 0;
  loadModels(engine);
}

function startNewFromCurrent() {
  if (view.data) startNew(view.provider, view.data.cwd, view.data.project_id);
  else startNew(view.provider);
}

async function openSession(session, task = null) {
  closeComposerSettings();
  stopWatchingRun();
  provider = session.provider;
  clearFiles(view);
  view = {...emptyView(provider), sessionId: session.id, data: session, warning: "正在读取原会话…",
    run: task?.id || null, runModel: task?.model ?? null, busy: Boolean(task)};
  const target = view;
  restorePermissionMode(target);
  $("prompt").value = "";
  notice(composerNotice, "");
  drawer(false);
  paint();
  loadModels(provider);
  if (target.run) watchRun(target);
  await openHistory(target);
}

async function openHistory(target, refresh = false, watch = false) {
  if (!target.sessionId || (target.historyPending && !refresh)) return;
  const epoch = ++target.historyEpoch;
  target.historyPending = true;
  const query = refresh ? "?refresh=true" : watch ? "?since=" + encodeURIComponent(target.historyRevision) : "";
  let changed = false;
  try {
    const result = await api("/sessions/" + target.provider + "/" + encodeURIComponent(target.sessionId) + query);
    if (view !== target || epoch !== target.historyEpoch) return;
    changed = result.session.model !== target.data?.model || result.session.title !== target.data?.title;
    target.data = {...result.session,archived:target.data?.archived || false};
    if (result.transcript) {
      const {messages, warning, revision, model} = result.transcript;
      changed = true;
      target.warning = warning;
      const afterRun = !target.busy && target.snapshot;
      if (!afterRun || appearsInHistory(afterRun, messages.slice(target.messages.length))) {
        target.messages = messages;
        target.historyRevision = revision;
        if (model) target.historyModel = model;
        if (afterRun) {
          target.snapshot = null;
          target.runModel = null;
        }
      }
    }
    if (result.active && !target.run && result.active.id !== target.lastFinished) {
      target.run = result.active.id;
      target.runModel = result.active.model ?? null;
      target.busy = true;
      changed = true;
      watchRun(target);
    }
  } catch (problem) {
    if (view === target && epoch === target.historyEpoch) {
      changed = true;
      target.warning = problem.message;
    }
  } finally {
    if (epoch === target.historyEpoch) target.historyPending = false;
  }
  if (view === target && changed) {
    renderHeader();
    renderMessages();
    renderRun();
    renderControls();
    scrollToLatest();
  }
}

function appearsInHistory(snapshot, messages) {
  const followUps = snapshot.follow_ups || [];
  if (followUps.some(entry => entry.delivery !== "sent")) return false;
  const prompts = [snapshot, ...followUps];
  let next = 0;
  for (const entry of messages) {
    const prompt = prompts[next];
    if (entry.role !== "user" || !prompt) continue;
    const text = entry.text.trim();
    if (text === prompt.prompt || (prompt.images?.length && text.replace(/(?:\n?\[图片\])+$/, "").trim() === prompt.prompt)) next++;
  }
  if (next !== prompts.length) return false;
  const visible = messages.filter(entry => entry.role === "assistant");
  const agents = visible.flatMap(entry => entry.work?.agents || []);
  if (snapshot.replies?.some(reply => reply.work?.agents?.some(agent =>
    !agents.some(recorded => recorded.id === agent.id &&
      (!agent.status || snapshot.status === "stopped" || agent.status === recorded.status))))) return false;
  if (snapshot.provider !== "codex" || !snapshot.replies?.length) return true;
  const recordedReplies = new Map();
  for (const [index, entry] of visible.entries()) {
    if (!entry.text) continue;
    const turn = entry.turn_id ?? index;
    const previous = recordedReplies.get(turn);
    recordedReplies.set(turn, previous ? previous + "\n\n" + entry.text : entry.text);
  }
  const replies = [...recordedReplies.values()];
  const notes = new Set(visible.flatMap(entry => (entry.work?.steps || []).filter(step => step.kind === "note").map(step => step.text)));
  return snapshot.replies.every(reply =>
    (!reply.text || replies.some(text => text.includes(reply.text))) &&
    (reply.work?.steps || []).every(step => step.kind !== "note" || notes.has(step.text)));
}

function openRunning(task) {
  const known = listedSessions().find(session => session.provider === task.provider && session.id === task.session_id);
  if (known) {
    openSession(known, task);
    return;
  }
  closeComposerSettings();
  stopWatchingRun();
  clearFiles(view);
  view = {...emptyView(task.provider), sessionId: task.session_id, run: task.id, runModel: task.model ?? null,
    busy: true, title: task.title};
  const target = view;
  restorePermissionMode(target);
  drawer(false);
  paint();
  loadModels(view.provider);
  watchRun(target);
  if (task.session_id) openHistory(target);
}

// Session controls deliberately separate actions from per-draft configuration.
let sessionSurface = null;
const sessionPending = new Set();
const sessionMenu = $("session-menu");
const sessionDialog = $("session-dialog");
const sessionKey = target => `${target.provider}:${target.sessionId}`;
const sameSession = target => target === view && target.sessionId === view.sessionId;
const sessionUnavailable = target => target.busy || target.sending || sessionPending.has(sessionKey(target)) ||
  histories.get(target.provider)?.native_sessions?.some(row => row.sessionId === target.sessionId && row.status === "busy");

function closeSessionSurfaces(restore = false, force = false) {
  if (sessionSurface?.pending && !force) return false;
  const trigger = sessionSurface?.trigger || $("session-more");
  sessionSurface = null;
  sessionMenu.close(); sessionDialog.close();
  $("session-more").setAttribute("aria-expanded", "false");
  if (restore) (trigger.isConnected && !trigger.hidden ? trigger : $("prompt")).focus({preventScroll:true});
  return true;
}

function openSessionMenu() {
  if (!view.sessionId || sessionSurface?.pending) return;
  closeComposerSettings(); closePicker(); drawer(false);
  const target = view;
  const list = $("session-menu-list"); list.replaceChildren();
  const add = (label, kind, unavailable = false, description = "") => {
    const row = button("", "picker-option session-menu-option", () => openSessionForm(kind, target));
    const copy = element("span", "", "picker-option-copy");
    copy.append(element("strong", t(label)));
    if (description) copy.append(element("small", t(description)));
    row.append(copy); row.disabled = unavailable; list.append(row);
  };
  const busy = sessionUnavailable(target);
  add("上下文用量", "context");
  if (target.provider !== "antigravity") {
    add("重命名", "rename", busy); add("创建分支", "fork", busy);
    if (target.provider === "codex") {
      add("目标", "goal", busy); add("审查改动", "review", busy);
      add(target.data?.archived ? "恢复归档" : "归档", target.data?.archived ? "unarchive" : "archive", busy);
    }
    if (target.provider === "claude") add("回退文件", "rewind", busy);
  }
  add("本轮设置", "setup", busy);
  if (target.provider !== "antigravity") {
    list.append(element("div", t("高级操作"), "session-menu-section"));
    add("MCP 与扩展", "extensions", busy);
  }
  if (busy) list.append(element("p", t("运行结束后可用"), "session-hint"));
  sessionSurface = {target, kind:"menu", trigger:$("session-more")};
  $("session-more").setAttribute("aria-expanded", "true");
  if (!narrowLayout.matches) {
    const rect = $("session-more").getBoundingClientRect();
    sessionMenu.style.setProperty("--menu-top", `${rect.bottom + 6}px`);
    sessionMenu.style.setProperty("--menu-right", `${Math.max(12, innerWidth - rect.right)}px`);
  }
  sessionMenu.showModal(); list.querySelector("button:not(:disabled)")?.focus();
}

function sessionField(body, label, input, help = "") {
  const group = element("div", "", "settings-field");
  input.id ||= `${body.id || "session"}-field-${body.querySelectorAll("input, textarea, select").length}`;
  const title = element("label", t(label)); title.htmlFor = input.id;
  group.append(title, input);
  if (help) group.append(element("p", t(help)));
  body.append(group); return input;
}

function sessionInput(value = "", type = "text") {
  const input = element("input"); input.type = type; input.value = value; return input;
}

function sessionFooter(state, label, submit, extra) {
  const footer = $("session-form-footer"); footer.replaceChildren();
  if (extra) footer.append(extra);
  if (state.kind !== "context") {
    footer.append(button(t("取消"), "secondary", () => closeSessionSurfaces(true)));
  }
  if (label) {
    const confirm = button(t(label), "primary"); confirm.type = "submit";
    footer.append(confirm); state.confirm = confirm;
  }
  state.submit = submit;
  state.confirmLabel = label;
}

function sessionStatus(text, tone = "") {
  const node = element("p", t(text), `session-hint ${tone}`); node.setAttribute("role", "status"); return node;
}

function activeSurface(state) { return sessionSurface === state && sameSession(state.target); }

async function mutateSession(state, action, after) {
  if (!activeSurface(state) || state.pending || sessionUnavailable(state.target)) return;
  const target = state.target;
  state.pending = true; sessionPending.add(sessionKey(target));
  renderControls();
  $("session-error").hidden = true;
  for (const control of sessionDialog.querySelectorAll("button,input,textarea,select")) control.disabled = true;
  if (state.confirm) state.confirm.textContent = t("正在处理…");
  try {
    const result = await api(`/sessions/${target.provider}/${target.sessionId}/actions`, action);
    await after(result, activeSurface(state));
  } catch (error) {
    if (activeSurface(state)) notice($("session-error"), error.message);
  } finally {
    state.pending = false; sessionPending.delete(sessionKey(target));
    if (target === view) renderControls();
    if (activeSurface(state)) {
      for (const control of sessionDialog.querySelectorAll("button,input,textarea,select")) control.disabled = false;
      if (state.confirm) state.confirm.textContent = t(state.confirmLabel);
    }
  }
}

function openSessionForm(kind, target = view) {
  if (target !== view || (!target.sessionId && kind !== "setup")) return;
  if (!closeSessionSurfaces(false)) return;
  closeComposerSettings(); closePicker(); drawer(false);
  const state = {target, kind, pending:false, trigger:$("session-more")};
  sessionSurface = state;
  $("session-dialog-close").disabled = false;
  const title = {rename:"重命名",fork:"创建分支",goal:"目标",review:"审查改动",archive:"归档",unarchive:"恢复归档",rewind:"回退文件",context:"上下文用量",extensions:"MCP 与扩展",setup:"本轮设置"}[kind];
  const heading = $("session-dialog-title");
  setText(heading, t(title));
  const body = $("session-form-body"); body.replaceChildren(); notice($("session-error"), "");
  if (kind === "rename") {
    const input = sessionField(body, "会话名称", sessionInput(target.data?.title || target.title || ""));
    input.required = true; input.maxLength = 150; input.autocomplete = "off";
    state.confirmLabel = "保存";
    sessionFooter(state, "保存", () => {
      const name = input.value.trim(); if (!name) { input.focus(); return; }
      return mutateSession(state, {action:"rename",name}, async (_result, current) => {
        if (target.data) target.data = {...target.data, title:name}; target.title = name;
        if (current) { closeSessionSurfaces(false, true); renderHeader(); feedback("会话名称已更新"); }
        await refreshHistories(true);
      });
    });
  } else {
    renderSessionTask(state, body);
  }
  sessionDialog.showModal();
  const focus = kind === "context" ? heading : body.querySelector("input,textarea,select");
  focus?.focus({preventScroll:true});
}

function draftConfiguration(target) {
  target.turnSettings ||= new Map();
  const key = `${target.provider}:${target.data?.cwd || target.cwd}`;
  if (!target.turnSettings.has(key)) target.turnSettings.set(key, {directories:"",files:"",agent:"",worktree:""});
  return target.turnSettings.get(key);
}

function buildSetupFields(body, target, configuration, update = () => {}) {
  const lines = (label, key, help) => {
    const input = element("textarea"); input.rows = 1; input.value = configuration[key]; input.spellcheck = false;
    sessionField(body, label, input, help);
    input.addEventListener("input", () => { configuration[key] = input.value; update(); });
  };
  lines("附加目录", "directories", "电脑上的绝对路径，每行一个目录");
  lines("文件上下文", "files", "电脑上的绝对路径，每行一个文件");
  if (target.provider !== "codex") {
    const agent = sessionField(body, "Agent 名称", sessionInput(configuration.agent), "可选，使用 CLI 中已配置的名称");
    agent.addEventListener("input", () => { configuration.agent = agent.value; update(); });
  }
  if (target.provider === "claude" && isNewView(target)) {
    const worktree = sessionField(body, "独立 Worktree", sessionInput(configuration.worktree), "可选，输入新 Worktree 名称");
    worktree.addEventListener("input", () => { configuration.worktree = worktree.value; update(); });
  }
}

let draftSettingsKey = "";
let draftSettingsTarget = null;
function renderDraftSettings() {
  const key = `${language}:${view.provider}:${view.data?.cwd || view.cwd}:${isNewView(view)}`;
  if (draftSettingsKey === key && draftSettingsTarget === view) return;
  draftSettingsTarget = view;
  draftSettingsKey = key; $("draft-settings").open = false;
  const body = $("draft-settings-fields"); body.replaceChildren();
  buildSetupFields(body, view, draftConfiguration(view));
}

$("session-more").addEventListener("click", openSessionMenu);
$("session-menu-close").addEventListener("click", () => closeSessionSurfaces(true));
$("session-dialog-close").addEventListener("click", () => closeSessionSurfaces(true));
$("session-form").addEventListener("submit", event => { event.preventDefault(); if (!sessionSurface?.pending) sessionSurface?.submit?.(); });
sessionMenu.addEventListener("keydown", event => {
  if (!["ArrowDown","ArrowUp","Home","End"].includes(event.key)) return;
  const choices = [...sessionMenu.querySelectorAll(".session-menu-option:not(:disabled)")];
  const index = choices.indexOf(document.activeElement);
  const next = event.key === "Home" ? 0 : event.key === "End" ? choices.length-1 : (index + (event.key === "ArrowUp" ? -1 : 1) + choices.length) % choices.length;
  if (choices[next]) {event.preventDefault();choices[next].focus();}
});
for (const surface of [sessionMenu, sessionDialog]) {
  surface.addEventListener("cancel", event => { event.preventDefault(); closeSessionSurfaces(true); });
  surface.addEventListener("click", event => { if (event.target === surface) closeSessionSurfaces(true); });
}

const nativeSearch = {query:"",archived:false,active:false,pending:false,hits:[],error:"",revision:0};
function renderNativeSearch() {
  const query = $("session-search").value.trim();
  const allowed = filter === "all" || filter === "codex";
  if (nativeSearch.query !== query || !allowed) {
    nativeSearch.revision++; nativeSearch.query = query; nativeSearch.active = false;
    nativeSearch.pending = false; nativeSearch.hits = []; nativeSearch.error = "";
  }
  $("search-tools").hidden = !query || !allowed;
  $("search-archived-row").hidden = !nativeSearch.active;
  const area = $("search-results"); area.hidden = !nativeSearch.active || !allowed || !query;
  if (area.hidden) return;
  area.replaceChildren(element("h3",t("Codex 原生全文搜索")));
  if (nativeSearch.pending) { area.append(sessionStatus("正在搜索…")); return; }
  if (nativeSearch.error) { area.append(sessionStatus(nativeSearch.error)); return; }
  if (!nativeSearch.hits.length) { area.append(sessionStatus("没有匹配的消息")); return; }
  for (const hit of nativeSearch.hits) {
    const row = button("","search-hit",async () => {
      const thread = hit.thread, term = nativeSearch.query, archived = nativeSearch.archived;
      await openSession({provider:"codex",id:thread.id,title:thread.name || thread.preview || t("未命名会话"),cwd:thread.cwd,project_id:thread.projectId,archived});
      const target = view;
      if (target.sessionId !== thread.id) return;
      try {
        const occurrences = await api(`/sessions/codex/${thread.id}/occurrences?q=${encodeURIComponent(term)}`);
        if (target !== view) return;
        const turn = occurrences.data?.[0]?.turnId;
        const index = target.messages.findIndex(message => message.turn_id === turn && message.text.toLowerCase().includes(term.toLowerCase()));
        const node = $("messages").children[index];
        if (node) { node.scrollIntoView({block:"center"}); node.tabIndex = -1; node.focus({preventScroll:true}); }
        else feedback("匹配消息尚未载入，可在原会话中继续查看。");
      } catch (error) { if (target === view) notice(composerNotice,error.message); }
    });
    row.append(element("strong",hit.thread.name || hit.thread.preview || t("未命名会话")),element("small",hit.snippet || "")); area.append(row);
  }
}
async function searchNativeMessages() {
  const query = $("session-search").value.trim(); if (!query) return;
  const revision = ++nativeSearch.revision;
  Object.assign(nativeSearch,{query,active:true,pending:true,error:"",archived:$("search-archived").checked});
  renderNativeSearch();
  try {
    const result = await api(`/search/codex?q=${encodeURIComponent(query)}&archived=${nativeSearch.archived}`);
    if (nativeSearch.revision !== revision) return;
    nativeSearch.hits = result.data || [];
  } catch (error) { if (nativeSearch.revision === revision) nativeSearch.error = error.message; }
  finally { if (nativeSearch.revision === revision) { nativeSearch.pending = false; renderNativeSearch(); } }
}
$("search-native").addEventListener("click",searchNativeMessages);
$("search-archived").addEventListener("change",searchNativeMessages);
$("session-search").addEventListener("keydown",event => { if (event.key === "Enter" && (filter === "all" || filter === "codex")) {event.preventDefault();searchNativeMessages();} });

function latestContext(target) {
  const live = target.snapshot?.replies?.map(reply => reply.work?.context).filter(Boolean).at(-1);
  return target.contextOverride || live || target.messages?.map(message => message.work?.context).filter(Boolean).at(-1) || null;
}

function nativeAction(target, action) {
  return api(`/sessions/${target.provider}/${target.sessionId}/actions`, action);
}

async function loadSessionSurface(state, load, render) {
  const body = $("session-form-body");
  body.replaceChildren(sessionStatus("正在读取…"));
  try {
    const data = await load();
    if (!activeSurface(state)) return;
    body.replaceChildren(); render(data, body);
    if ([sessionDialog,$("session-dialog-close")].includes(document.activeElement)) body.querySelector("input,textarea,select")?.focus({preventScroll:true});
  } catch (error) {
    if (!activeSurface(state)) return;
    body.replaceChildren(sessionStatus("暂时无法读取，请重试。"));
    notice($("session-error"), error.message);
    body.append(button(t("重试"), "secondary", () => loadSessionSurface(state,load,render)));
  }
}

function contextMetrics(usage) {
  const metrics = element("dl", "", "session-metrics");
  for (const [label,key] of [["已用","used"],["上限","limit"],["剩余","remaining"]]) {
    const row = element("div", "", "session-metric");
    row.append(element("dt",t(label)),element("dd",usage?.[key] == null ? t("引擎未报告") : usage[key].toLocaleString(language)));
    metrics.append(row);
  }
  return metrics;
}

function stageNativeCommand(state, command) {
  if (!activeSurface(state) || sessionUnavailable(state.target)) return;
  closeSessionSurfaces(false,true);
  const existing = $("prompt").value;
  if (existing.trim() || view.files.length) {
    view.otherDrafts.push({prompt:existing,files:view.files,withdrawn:view.editingWithdrawn,turnSettings:new Map([...view.turnSettings || []].map(([key,value]) => [key,{...value}]))});
    view.files = []; view.editingWithdrawn = false;
  }
  $("prompt").value = command;
  renderDraftFiles();
  $("prompt").dispatchEvent(new Event("input",{bubbles:true}));
  feedback("已填入输入框，请确认后发送。");
  $("prompt").focus();
}

function renderSessionTask(state, body) {
  const target = state.target;
  if (state.kind === "context") {
    let metrics = contextMetrics(latestContext(target));
    body.append(metrics,sessionStatus("最近一次引擎报告"));
    if (target.provider === "claude" && !sessionUnavailable(target)) {
      const refresh = button(t("刷新上下文"),"secondary", async () => {
        if (sessionUnavailable(target)) return;
        refresh.disabled = true;
        notice($("session-error"), "");
        try {
          const data = await nativeAction(target,{action:"context"});
          if (!activeSurface(state)) return;
          const used = data.totalTokens ?? null, limit = data.maxTokens ?? null;
          const context = {used,limit,remaining:used != null && limit != null ? Math.max(0,limit-used) : null};
          target.contextOverride = context;
          const updated = contextMetrics(context);
          metrics.replaceWith(updated);
          metrics = updated;
        } catch (error) { if (activeSurface(state)) notice($("session-error"),error.message); }
        finally { refresh.disabled = false; }
      }); body.append(refresh);
    }
    sessionFooter(state,target.provider !== "antigravity" && !sessionUnavailable(target) ? "压缩上下文" : null,
      () => stageNativeCommand(state,"/compact"));
  } else if (state.kind === "setup") {
    const configuration = {...draftConfiguration(target)};
    body.append(sessionStatus("此设置仅用于下一次启动，不会改变正在运行的回复。"));
    buildSetupFields(body,target,configuration);
    sessionFooter(state,"保存",() => {
      Object.assign(draftConfiguration(target),configuration); closeSessionSurfaces(true);
      feedback("本轮设置已保存");
    });
  } else if (["fork","archive","unarchive"].includes(state.kind)) {
    body.append(sessionStatus(({fork:"从当前记录创建独立分支，原会话保持不变。",archive:"归档后可从侧栏的 Codex 搜索中找到并恢复。",unarchive:"恢复后，此会话将重新出现在历史列表中。"})[state.kind]));
    state.confirmLabel = ({fork:"创建分支",archive:"归档",unarchive:"恢复归档"})[state.kind];
    sessionFooter(state,state.confirmLabel,() => mutateSession(state,{action:state.kind},async (result,current) => {
      if (state.kind === "fork") {
        if (!result?.session_id) throw new Error(t("引擎未返回新会话，请刷新历史确认。"));
        await refreshHistories(true);
        if (!activeSurface(state)) return;
        const session = listedSessions().find(row => row.provider === target.provider && row.id === result.session_id) ||
          {...target.data,provider:target.provider,id:result.session_id,title:t("会话分支"),cwd:target.data?.cwd || target.cwd};
        closeSessionSurfaces(false,true); await openSession(session);
      } else {
        if (current) closeSessionSurfaces(false,true);
        await refreshHistories(true);
        if (view === target && state.kind === "archive") startNew(target.provider,target.data?.cwd || target.cwd);
        else if (view === target) { target.data = {...target.data,archived:false}; renderHeader(); }
      }
    }));
  } else if (state.kind === "goal") {
    sessionFooter(state,null);
    loadSessionSurface(state,() => nativeAction(target,{action:"goal_get"}),(data,area) => {
      const goal = data?.goal;
      const objective = element("textarea"); objective.rows = 4; objective.value = goal?.objective || ""; objective.required = true;
      sessionField(area,"目标",objective,"说明希望达成的结果");
      const budget = sessionInput(goal?.tokenBudget ?? "","number"); budget.min = "1"; budget.step = "1";
      sessionField(area,"Token 预算",budget,"可选，留空不设预算");
      if (goal) {
        const status = {active:"进行中",paused:"已暂停",blocked:"等待处理",usageLimited:"用量受限",budgetLimited:"预算已用完",complete:"已完成"}[goal.status] || "引擎未报告";
        area.append(sessionStatus(t("目标状态：{status}",{status:t(status)})));
        if (goal.tokensUsed != null) area.append(sessionStatus(t("已使用 {count} Token",{count:goal.tokensUsed.toLocaleString(language)})));
      }
      const clear = goal ? button(t("清除目标"),"secondary session-clear",() => mutateSession(state,{action:"goal_clear"},async (_result,current) => {
        if (current) { closeSessionSurfaces(false,true); feedback("目标已清除"); }
      })) : null;
      state.confirmLabel = "保存目标";
      sessionFooter(state,"保存目标",() => {
        if (!objective.value.trim()) { objective.focus(); return; }
        return mutateSession(state,{action:"goal_set",objective:objective.value.trim(),token_budget:budget.value ? Number(budget.value) : null},async (_result,current) => {
          if (current) { closeSessionSurfaces(false,true); feedback("目标已保存"); }
        });
      },clear);
    });
  } else if (state.kind === "review") {
    const choice = element("select");
    choice.append(new Option(t("未提交改动"),"working"),new Option(t("指定分支"),"branch"));
    sessionField(body,"审查范围",choice);
    const branch = sessionInput(); const group = element("div"); body.append(group);
    sessionField(group,"分支名称",branch,"与当前分支比较的基准分支"); group.hidden = true;
    choice.addEventListener("change",() => { group.hidden = choice.value !== "branch"; branch.required = !group.hidden; if (!group.hidden) branch.focus(); });
    body.append(sessionStatus("返回输入框确认发送"));
    sessionFooter(state,"准备审查",() => {
      if (choice.value === "branch" && !branch.value.trim()) { branch.focus(); return; }
      stageNativeCommand(state,choice.value === "branch" ? `/review ${branch.value.trim()}` : "/review");
    });
  } else if (state.kind === "rewind") {
    body.append(sessionStatus("仅回退文件，不回退对话"));
    const messages = target.messages.filter(message => message.role === "user" && /^[\da-f]{8}(?:-[\da-f]{4}){3}-[\da-f]{12}$/i.test(message.turn_id || ""));
    if (!messages.length) { body.append(sessionStatus("暂无可回退的用户消息")); sessionFooter(state,null); return; }
    const choice = element("select"); choice.append(new Option(t("选择一条用户消息"),"")); choice.required = true;
    for (const [index,message] of messages.entries()) choice.append(new Option(`${index+1}. ${message.text.replace(/\s+/g," ").slice(0,90) || t("图片消息")}`,message.turn_id));
    sessionField(body,"回退到",choice);
    const preview = element("section","","session-rewind-preview"); body.append(preview);
    let previewId = null, previewRevision = 0;
    const showPreview = () => {
      if (!choice.value) { choice.focus(); return; }
      const id = choice.value, revision = ++previewRevision;
      state.confirmLabel = "预览文件回退";
      return mutateSession(state,{action:"checkpoint_preview",message_id:id},async (result,current) => {
        if (!current || revision !== previewRevision || choice.value !== id) return;
        if (!result?.canRewind) throw new Error(t("这条消息没有可用的文件检查点。"));
        previewId = id; preview.replaceChildren();
        preview.append(sessionStatus("文件列表与增删统计，不是完整差异。"));
        const files = element("ul","","session-list");
        for (const file of result.filesChanged || []) files.append(element("li",file));
        preview.append(files,sessionStatus(t("新增 {added} 行，删除 {removed} 行",{added:result.insertions ?? 0,removed:result.deletions ?? 0})));
        state.confirmLabel = "确认文件回退";
        sessionFooter(state,"确认文件回退",() => {
          if (!previewId || previewId !== choice.value) return;
          return mutateSession(state,{action:"checkpoint_rewind",message_id:previewId},async (_result,current) => {
            if (current) { closeSessionSurfaces(false,true); feedback("文件已回退，对话记录保留。"); await openHistory(target,true); }
          });
        });
      });
    };
    choice.addEventListener("change",() => {
      previewId = null; previewRevision++; preview.replaceChildren(); state.confirmLabel = "预览文件回退";
      sessionFooter(state,"预览文件回退",showPreview);
    });
    state.confirmLabel = "预览文件回退"; sessionFooter(state,"预览文件回退",showPreview);
  } else if (state.kind === "extensions") {
    sessionFooter(state,null);
    const method = target.provider === "codex" ? "extensions" : "mcp_status";
    loadSessionSurface(state,() => nativeAction(target,{action:method}),(data,area) => renderExtensionStatus(state,data,area));
  }
}

function renderExtensionStatus(state, data, area) {
  const servers = data?.mcp?.data || data?.data || data?.mcpServers || [];
  if (!servers.length) area.append(sessionStatus("当前引擎未报告可读条目"));
  for (const server of servers) {
    const card = element("section","","session-status-card");
    card.append(element("h3",server.name || server.serverName || t("MCP 服务器")));
    const status = server.runtimeStatus || server.status;
    const translated = {connected:"已连接",connecting:"连接中",failed:"连接失败",disconnected:"未连接",pending:"等待处理"}[status];
    card.append(sessionStatus(translated || "引擎未报告"));
    const tools = Array.isArray(server.tools) ? server.tools : Object.values(server.tools || {});
    if (tools.length) {
      const details = element("details"); details.append(element("summary",t("已加载 {count} 个工具",{count:tools.length})));
      const list = element("ul","","session-list");
      for (const tool of tools) list.append(element("li",typeof tool === "string" ? tool : tool.name || tool.title || ""));
      details.append(list); card.append(details);
    }
    if (server.toolsError) card.append(sessionStatus(server.toolsError));
    if (state.target.provider === "codex" && ["notLoggedIn","unauthenticated"].includes(server.authStatus)) {
      card.append(button(t("MCP 授权"),"secondary",() => {
        state.confirmLabel = "MCP 授权";
        mutateSession(state,{action:"mcp_authorize",name:server.name},async (result,current) => {
          if (!current) return;
          const url = result?.authorizationUrl;
          if (!url || !/^https?:\/\//.test(url)) throw new Error(t("引擎未返回授权地址"));
          const link = element("a",t("打开授权页面"),"session-authorize-link"); link.href = url; openInNewTab(link); card.append(link,sessionStatus("授权后刷新状态"));
        });
      }));
    }
    area.append(card);
  }
  for (const [key,label] of [["hooks","Hooks"],["plugins","Plugins"],["apps","Apps"]]) {
    if (!(key in (data || {})) && !data?.[`${key}_error`]) continue;
    const card = element("section","","session-status-card"); card.append(element("h3",label));
    if (data[`${key}_error`]) card.append(sessionStatus(data[`${key}_error`]));
    else {
      const value = data[key];
      const entries = key === "hooks" ? (value?.data || []).flatMap(row => row.hooks || []) : key === "plugins" ? (value?.marketplaces || []).flatMap(row => row.plugins || []) : value?.data || value?.apps || [];
      if (!entries.length) card.append(sessionStatus("未报告已加载条目"));
      for (const entry of entries) card.append(element("p",entry.name || entry.displayName || entry.eventName || t("已配置"),"session-hint"));
    }
    area.append(card);
  }
  if (state.target.provider === "codex") {
    state.confirmLabel = "重新加载 MCP";
    sessionFooter(state,"重新加载 MCP",() => mutateSession(state,{action:"mcp_reconnect"},async (_result,current) => {
      if (current) { closeSessionSurfaces(false,true); feedback("MCP 已重新加载"); }
    }));
  }
}

function renderHeader() {
  const isNew = isNewView(view);
  if (sessionSurface && sessionSurface.target !== view) closeSessionSurfaces(false, true);
  if (sessionSurface?.pending) $("session-more").disabled = true;
  else $("session-more").disabled = false;
  $("session-more").hidden = !view.sessionId;
  renderDraftSettings();
  const title = view.run ? view.title : view.data?.title || view.title;
  setText($("chat-title"), isNew ? "Naro" : title || view.data?.title || t("未命名会话"));
  const projectId = view.data?.project_id || (view.location.startsWith("project:") ? view.location.slice(8) : null);
  const path = view.data?.cwd || view.cwd;
  const catalog = nativeProjects();
  const project = projectId ? catalog.find(project => project.id === projectId) :
    view.provider === "codex" ? projectsByRoot(catalog).get(path) : null;
  const projectName = project?.name || folderName(path);
  setText($("chat-path"), projectName);
  $("chat-path").hidden = !projectName;
  setText($("active-provider"), engines[view.provider].label);
  $("chat-details").hidden = isNew;
  $("new-view").hidden = !isNew;
  if (!isNew) closePicker();
  if (typeof syncGitCwd === "function") syncGitCwd();
}

function appendInline(node, text) {
  for (const part of text.split(/(`[^`\r\n]+`|\*\*[^*\r\n]+\*\*)/)) {
    if (part.startsWith("`") && part.endsWith("`") && part.length > 2) {
      node.append(element("code", part.slice(1, -1)));
    } else if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
      node.append(element("strong", part.slice(2, -2)));
    } else {
      node.append(part);
    }
  }
}

function markup(text) {
  const wrapper = element("div", "", "message-text");
  for (const [index, block] of text.split(/^ {0,3}```[^\r\n]*\r?$/m).entries()) {
    if (index % 2) {
      const pre = element("pre");
      pre.append(element("code", block.replace(/^\r?\n/, "")));
      wrapper.append(pre);
      continue;
    }
    let paragraph = null;
    let list = null;
    for (const line of block.split(/\r?\n/)) {
      const heading = line.match(/^ {0,3}(#{1,6})[ \t]+(.+)$/);
      const item = line.match(/^ {0,3}([-+*]|\d{1,9}[.)])[ \t]+(.+)$/);
      if (heading || !line.trim()) {
        paragraph = list = null;
        if (heading) {
          const title = element("h" + Math.min(heading[1].length + 1, 5));
          appendInline(title, heading[2]);
          wrapper.append(title);
        }
      } else if (item) {
        paragraph = null;
        const type = item[1].length > 1 ? "ol" : "ul";
        if (list?.localName !== type) {
          list = element(type);
          if (type === "ol") list.start = parseInt(item[1], 10);
          wrapper.append(list);
        }
        const row = element("li");
        appendInline(row, item[2]);
        list.append(row);
      } else {
        list = null;
        if (!paragraph) {
          paragraph = element("p");
          wrapper.append(paragraph);
        } else {
          paragraph.append("\n");
        }
        appendInline(paragraph, line);
      }
    }
  }
  return addCodeCopyButtons(wrapper);
}

function renderedMarkup(text, html) {
  if (typeof html !== "string") return markup(text);
  const body = element("div", "", "message-text rendered-message");
  body.innerHTML = html;
  for (const link of body.querySelectorAll("a")) {
    const href = link.getAttribute("href");
    if (href?.startsWith("/api/")) link.href = scopedAsset(href);
    const filePath = href?.match(/^\/api(\/file-previews\/[a-f0-9]{64})$/)?.[1];
    if (filePath) {
      link.setAttribute("aria-haspopup", "dialog");
      link.addEventListener("click", event => {
        event.preventDefault();
        // A touch tap can leave the conversation focused. Avoid restoring it on close.
        if (!filePreviewDialog.open && event.detail > 0 && document.activeElement === conversation) {
          conversation.blur();
        }
        loadFilePreview(filePath);
      });
    } else {
      openInNewTab(link);
    }
  }
  return addCodeCopyButtons(body);
}

async function loadFilePreview(path, source = false) {
  const preview = {path, source, file: null};
  filePreview = preview;
  const status = $("file-preview-notice");
  const refresh = $("file-preview-refresh");
  setText($("file-preview-title"), t("文件预览"));
  $("file-preview-content").replaceChildren();
  $("file-preview-copy").replaceChildren();
  $("file-preview-modes").hidden = true;
  refresh.disabled = true;
  notice(status, "正在读取文件…");
  if (!filePreviewDialog.open) filePreviewDialog.showModal();
  try {
    const file = await api(path);
    if (filePreview !== preview || !filePreviewDialog.open) return;
    preview.file = file;
    setText($("file-preview-title"), file.name);
    $("file-preview-copy").append(copyTextButton(file.text, "复制文件内容", () =>
      notice(status, "选中文件内容即可复制。")));
    notice(status, "");
    showFilePreview(source);
  } catch (error) {
    if (filePreview === preview && filePreviewDialog.open) notice(status, error.message);
  } finally {
    if (filePreview === preview) refresh.disabled = false;
  }
}

function showFilePreview(source) {
  const {file} = filePreview;
  if (!file) return;
  filePreview.source = source;
  const canRender = typeof file.html === "string";
  $("file-preview-modes").hidden = !canRender;
  $("file-preview-rendered").setAttribute("aria-pressed", String(!source));
  $("file-preview-source").setAttribute("aria-pressed", String(source));
  const content = $("file-preview-content");
  if (canRender && !source) {
    content.replaceChildren(renderedMarkup(file.text, file.html));
  } else {
    content.replaceChildren(element("pre", file.text, "file-preview-raw"));
  }
  $("file-preview-scroll").scrollTo(0, 0);
}

$("file-preview-close").addEventListener("click", () => filePreviewDialog.close());
$("file-preview-refresh").addEventListener("click", () => loadFilePreview(filePreview.path, filePreview.source));
$("file-preview-rendered").addEventListener("click", () => showFilePreview(false));
$("file-preview-source").addEventListener("click", () => showFilePreview(true));
filePreviewDialog.addEventListener("close", () => {
  if (filePreviewDialog.open) return;
  filePreview = null;
  $("file-preview-content").replaceChildren();
  $("file-preview-copy").replaceChildren();
  notice($("file-preview-notice"), "");
});

function addCodeCopyButtons(body) {
  for (const code of body.querySelectorAll("pre > code")) {
    const pre = code.parentElement;
    const block = element("div", "", "code-block");
    const toolbar = element("div", "", "code-block-toolbar");
    toolbar.append(copyTextButton(code.textContent.replace(/\r?\n$/, ""), "复制代码"));
    pre.replaceWith(block);
    block.append(toolbar, pre);
  }
  return body;
}

function message(role, text, {rendered, work, key, running = false, attachments, local_files: localFiles, includeNotes = true, streamingText = false} = {},
  row = element("article", "", "message " + role)) {
  const parts = [];
  if (role !== "assistant") {
    const display = text.trimEnd();
    if (attachments?.length) parts.push(["images", attachments, () => sentImages(attachments)]);
    if (localFiles?.length) parts.push(["files", localFiles, () => sentFiles(localFiles)]);
    if (display) {
      parts.push(["bubble", display, () => {
        const bubble = element("div", "", "message-bubble");
        bubble.append(element("div", display, "message-text"));
        return bubble;
      }]);
      parts.push(["copy", text, () => copyTextButton(text)]);
    }
    renderList(row, parts);
    return row;
  }
  if (work?.plan?.length) parts.push(["plan", work.plan, previous => {
    const label = t("计划");
    const panel = previous || disclosure("turn-plan", key + ":plan", label, false);
    if (!previous) panel.append(element("ul", "", "plan-steps"));
    setText(panel.firstElementChild, label);
    const statuses = {pending:"待办", inProgress:"进行中", completed:"已完成"};
    panel.lastElementChild.replaceChildren(...work.plan.map(step =>
      element("li", `${t(statuses[step.status] || step.status)} · ${step.step}`)));
    return panel;
  }]);
  const steps = work?.steps || [];
  if (includeNotes) {
    for (const step of steps) {
      if (step.kind === "note" && step.text) parts.push(["note:" + step.id, step.text, () => markup(step.text)]);
    }
  }
  if (work?.agents?.length) {
    parts.push(["agents", work.agents, panel => agentTasks(work.agents, panel)]);
  }
  const activity = steps.filter(visibleWorkStep);
  const characters = [...text];
  let offset = 0;
  const appendBody = end => {
    const start = offset;
    const content = characters.slice(start, end).join("");
    offset = end;
    if (!content.trim()) return;
    const tail = end === characters.length;
    const section = rendered?.sections?.find(section => section.offset === start)?.rendered;
    const html = streamingText && tail ? undefined : section?.html ?? (start === 0 && tail ? rendered?.html : undefined);
    parts.push(["body:" + start, [content, html], () => renderedMarkup(content, html)]);
  };
  for (let index = 0; index < activity.length;) {
    const end = Math.min(Math.max(activity[index].text_offset ?? 0, offset), characters.length);
    appendBody(end);
    const first = index++;
    while (index < activity.length && (activity[index].text_offset ?? 0) <= end) index++;
    const group = activity.slice(first, index);
    const groupKey = key + ":work:" + group[0].id;
    const html = group.map(step => rendered?.work_steps?.[step.id]);
    parts.push(["work:" + group[0].id, [group, html, running], panel => workSteps(group, html, groupKey, running, panel)]);
  }
  appendBody(characters.length);
  if (work?.changes?.length) parts.push(["files", work.changes, panel => fileChanges(work.changes, key, panel)]);
  if (rendered?.previews?.length) {
    parts.push(["previews", rendered.previews, gallery => previewImages(rendered.previews, gallery)]);
  }
  if (text.trim()) parts.push(["copy", text, () => copyTextButton(text)]);
  renderList(row, parts);
  return row;
}

function copyTextButton(text, label = "复制", onFailure = () => notice(composerNotice, "可选中消息文字后复制。")) {
  const control = button("", "message-copy", async () => {
    try {
      await navigator.clipboard.writeText(text);
      showCopied(true);
      clearTimeout(timer);
      timer = setTimeout(() => showCopied(false), 1800);
    } catch {
      onFailure();
    }
  });
  let timer;
  function showCopied(copied) {
    const title = t(copied ? "已复制" : label);
    control.classList.toggle("copied", copied);
    control.title = title;
    control.setAttribute("aria-label", title);
  }
  showCopied(false);
  return control;
}

function openInNewTab(link) {
  link.target = "_blank";
  link.rel = "noopener noreferrer";
}

function disclosure(className, key, label, open) {
  const details = element("details", "", className);
  details.dataset.panel = key;
  details.open = open;
  const summary = element("summary", label);
  summary.addEventListener("click", () => { details.dataset.userOpen = String(!details.open); });
  details.append(summary);
  return details;
}

function restorePanels(area, previous) {
  const saved = new Map([previous, ...previous.querySelectorAll("[data-panel][data-user-open]")]
    .map(panel => [panel.dataset.panel, panel.dataset.userOpen]));
  for (const panel of [area, ...area.querySelectorAll("[data-panel]")]) {
    const open = saved.get(panel.dataset.panel);
    if (open !== undefined) {
      panel.dataset.userOpen = open;
      panel.open = open === "true";
    }
  }
}

const workLabels = {reasoning: "思考", thinking: "思考", command: "命令", tool: "工具"};
const workStatuses = {running: "进行中", complete: "已完成", failed: "未成功", interrupted: "已中断"};

function thinkingWorkStep(step) {
  return step.kind === "reasoning" || step.kind === "thinking";
}

function visibleWorkStep(step) {
  return step.kind !== "note" && (!thinkingWorkStep(step) || Boolean(step.text?.trim()));
}

function agentTasks(agents, previous) {
  const panel = previous || element("section", "", "turn-agents");
  if (!previous) {
    panel.append(element("h3", "", "agent-heading"), element("ul", "", "agent-list"));
  }
  setText(panel.firstElementChild, t("子代理"));
  renderList(panel.lastElementChild, agents.map((agent, index) => [agent.id, [agent, index], () => {
    const item = element("li", "", "agent-task");
    const name = agent.name || agent.description || t("子代理 {count}", {count: index + 1});
    const content = element("div", "", "agent-task-content");
    content.append(element("div", name, "agent-task-name"));
    if (agent.name && agent.description) {
      content.append(element("div", agent.description, "agent-task-description"));
    }
    const status = agent.status === "started" ? "已启动" : workStatuses[agent.status] || "状态待确认";
    item.append(content, element("span", t(status), "agent-status " + (agent.status || "unknown")));
    return item;
  }]));
  return panel;
}

function workSteps(steps, html, key, running, previous) {
  const label = t((steps.every(thinkingWorkStep) && "思考") ||
    (steps.every(step => step.kind === "command") && (running ? "命令" : "运行了命令")) || "工作过程");
  const panel = previous || disclosure("turn-work", key + ":work", label, false);
  const list = previous ? panel.lastElementChild : element("ol", "", "turn-steps");
  const height = list.scrollHeight;
  const follow = !previous || height - list.scrollTop - list.clientHeight < 16;
  if (!previous) {
    panel.append(list);
    panel.firstElementChild.addEventListener("click", () => {
      if (narrowLayout.matches && !panel.open) view.followLatest = false;
    });
  }
  setText(panel.firstElementChild, label);
  renderList(list, steps.map((step, index) => [step.id, [step, html[index]], previous => {
    const item = previous || element("li", "", "work-step");
    const status = workStatuses[step.status];
    const parts = [element("span", t(workLabels[step.kind] || "进度"), "step-kind")];
    if (status) parts.push(element("span", t(status) + (step.exit_code ? " · " + step.exit_code : ""), "step-status " + step.status));
    if (step.text) {
      const body = thinkingWorkStep(step)
        ? renderedMarkup(step.text, html[index])
        : element(step.kind === "command" ? "code" : "div", step.text);
      body.classList.add("step-text");
      parts.push(body);
    }
    item.replaceChildren(...parts);
    return item;
  }]));
  if (running && !narrowLayout.matches && panel.open && follow) {
    if (list.isConnected) {
      if (list.scrollHeight !== height) list.scrollTop = list.scrollHeight;
    } else requestAnimationFrame(() => {
      if (list.isConnected && panel.open) list.scrollTop = list.scrollHeight;
    });
  }
  return panel;
}

function changeCounts(added, removed) {
  const counts = element("span", "", "change-counts");
  counts.append(element("span", "+" + added, "lines-added"), element("span", "−" + removed, "lines-removed"));
  return counts;
}

function fileDiff(file) {
  const diff = element("pre", "", "file-diff");
  const code = element("code");
  const lines = file.diff.match(/[^\n]*\n|[^\n]+$/g) || [];
  const hasHunks = lines.some(line => line.startsWith("@@ "));
  let wholeFileClass = "";
  if (!hasHunks) {
    if (file.added && !file.removed) wholeFileClass = "diff-added";
    if (file.removed && !file.added) wholeFileClass = "diff-removed";
  }

  const fileHeader = /^(?:--- |\+\+\+ |index |(?:old|new|deleted)(?: file)? mode |(?:rename|copy) (?:from|to) |(?:dis)?similarity index |Binary files |\*\*\* )/;
  let inHunk = false;
  for (const line of lines) {
    let kind = wholeFileClass;
    if (!kind) {
      if (line.startsWith("@@ ")) {
        inHunk = true;
        kind = "diff-meta";
      } else if (line.startsWith("diff --git ")) {
        inHunk = false;
        kind = "diff-meta";
      } else if (line.startsWith("\\ ") || (!inHunk && fileHeader.test(line))) {
        kind = "diff-meta";
      } else if (line.startsWith("+")) {
        kind = "diff-added";
      } else if (line.startsWith("-")) {
        kind = "diff-removed";
      }
    }
    code.append(element("span", line, kind ? "diff-line " + kind : "diff-line"));
  }
  if (file.truncated) {
    if (!file.diff.endsWith("\n")) code.append("\n");
    code.append(element("span", "… " + t("仅显示部分变更"), "diff-line diff-meta"));
  }
  diff.append(code);
  return diff;
}

function fileChanges(files, key, previous) {
  const panel = disclosure("turn-changes", key + ":files", t("已更改 {count} 个文件", {count: files.length}), true);
  panel.firstChild.append(changeCounts(files.reduce((sum, file) => sum + file.added, 0),
    files.reduce((sum, file) => sum + file.removed, 0)));
  const visibleCount = 3;
  let list = panel;
  for (const [index, file] of files.entries()) {
    if (index === visibleCount) {
      const remaining = files.length - visibleCount;
      list = disclosure("file-changes-more", key + ":more-files", "", false);
      list.firstChild.append(element("span", t("查看另外 {count} 个文件", {count: remaining}), "show-more-files"),
        element("span", t("收起另外 {count} 个文件", {count: remaining}), "hide-more-files"));
      panel.append(list);
    }
    const counts = changeCounts(file.added, file.removed);
    if (!file.diff) {
      const row = element("div", "", "file-change-row");
      row.append(element("span", file.path, "changed-path"), counts);
      list.append(row);
      continue;
    }
    const item = disclosure("file-change", key + ":diff:" + file.path, "", false);
    item.firstChild.append(element("span", file.path, "changed-path"), counts);
    item.addEventListener("toggle", () => {
      if (item.open && !item.lastElementChild.matches(".file-diff")) item.append(fileDiff(file));
    });
    list.append(item);
  }
  if (previous) restorePanels(panel, previous);
  return panel;
}

function previewImages(previews, gallery = element("div", "", "reply-previews")) {
  renderList(gallery, previews.map(preview => [preview.url, preview.name, () => {
    const link = element("a", "", "reply-preview");
    link.href = scopedAsset(preview.url);
    openInNewTab(link);
    const img = element("img");
    img.alt = preview.name;
    img.loading = "lazy";
    img.src = scopedAsset(preview.url);
    link.append(img);
    return link;
  }]));
  return gallery;
}

function renderMessages() {
  const area = $("messages");
  const preview = !view.messages.length && !view.busy && !view.snapshot ? view.data?.preview : "";
  notice($("history-notice"), view.warning || (preview && "原会话预览") || "");
  if (view.renderedHistory === view.messages && view.renderedPreview === preview && view.renderedLanguage === language) return;
  view.renderedHistory = view.messages;
  view.renderedPreview = preview;
  view.renderedLanguage = language;
  const session = `${view.provider}:${view.sessionId}`;
  const rows = view.messages.map((entry, index) => {
    const key = `${session}:${entry.role}:${index}`;
    return [key, entry, row => message(entry.role, entry.text, {...entry, key}, row)];
  });
  if (preview) rows.push([session + ":preview", preview, () => message("user", preview)]);
  renderList(area, rows);
}

function renderRun() {
  const snap = view.snapshot;
  const followUps = snap?.follow_ups || [];
  const sent = followUps.filter(entry => entry.delivery === "sent");
  const key = snap ? JSON.stringify([language, snap.id, snap.output, snap.activity, snap.status, snap.approval?.id, snap.error,
    snap.active_permission_mode?.id, sent.map(entry => [entry.id, entry.output]), snap.replies, view.replyReveal?.end]) : "";
  if (key === liveKey) return;
  liveKey = key;
  renderLiveReply(snap, sent);
  scrollToLatest();
}

function renderPendingQueue() {
  const snap = view.snapshot;
  const pending = snap?.follow_ups?.filter(entry => entry.delivery !== "sent") || [];
  const queueKey = pending.length ? JSON.stringify([language, snap.id, snap.status,
    view.sending, view.stopping, view.withdrawing, pending]) : "";
  if (queueKey === pendingKey) return;
  pendingKey = queueKey;
  renderPendingMessages(snap, pending);
  scrollToLatest();
}

function renderLiveReply(snap, sent) {
  const area = $("live");
  area.hidden = !snap;
  if (!snap) {
    renderList(area, []);
    return;
  }
  const replies = snap.replies || [{after: null, text: snap.output}, ...sent.map(entry => ({after: entry.id, text: entry.output}))]
    .filter(entry => entry.text);
  const byPrompt = new Map(replies.map(reply => [reply.after, reply]));
  const notes = new Map();
  for (const reply of replies) {
    for (const step of reply.work?.steps || []) {
      if (step.kind !== "note" || !step.text) continue;
      const after = Object.hasOwn(step, "after") ? step.after : reply.after;
      if (!notes.has(after)) notes.set(after, []);
      notes.get(after).push(step);
    }
  }
  const lastPrompt = sent.at(-1)?.id ?? null;
  const rows = [];
  for (const [id, entry] of [[null, snap], ...sent.map(entry => [entry.id, entry])]) {
    const key = `${snap.id}:${id || "first"}`;
    rows.push([key + ":prompt", [entry.prompt, entry.images, entry.image_start, entry.local_files], row => runPrompt(snap.id, entry, row)]);
    for (const step of notes.get(id) || []) {
      rows.push([`${key}:note:${step.id}`, step.text, () => markup(step.text)]);
    }
    const reply = byPrompt.get(id);
    const running = id === lastPrompt && snap.status === "running";
    const end = running && view.replyReveal?.after === id ? view.replyReveal.end : null;
    const text = end === null ? reply?.text : reply?.text.slice(0, end);
    if (reply && (text || reply.rendered?.previews?.length || reply.work?.changes?.length || reply.work?.agents?.length ||
      reply.work?.steps?.some(visibleWorkStep))) {
      const replyKey = key + ":reply";
      rows.push([replyKey, [reply, running, end], row => message("assistant", text,
        {...reply, key: replyKey, running, includeNotes: false, streamingText: end !== null || reply.streamingText}, row)]);
    }
  }
  const activity = snap.status === "stopped" ? "已停止" : snap.status === "running" && (snap.activity || "正在回复");
  if (activity) rows.push([snap.id + ":activity", activity, (status = element("div", "", "activity")) => {
    const label = t(activity);
    status.setAttribute("role", "status");
    status.title = label;
    setText(status, label);
    return status;
  }]);
  if (snap.error) rows.push(["error", snap.error, () => element("div", t(snap.error), "notice")]);
  if (snap.approval) {
    const mode = snap.active_permission_mode;
    rows.push([snap.id + ":approval", [snap.approval, mode], () => {
      const box = element("div", "", "approval");
      box.append(element("strong", t("确认操作 · {title}", {title: t(snap.approval.title)})));
      if (mode) box.append(element("p", t("当前回复使用{mode}。", {mode: t(mode.label)}), "approval-mode"));
      box.append(element("pre", snap.approval.details),
        button(t("允许一次"), "primary", () => decide(snap, true)),
        button(t("拒绝"), "secondary", () => decide(snap, false)));
      return box;
    }]);
  }
  renderList(area, rows);
}

function renderPendingMessages(snap, pending) {
  const area = $("pending-messages");
  $("message-queue").hidden = !pending.length;
  const running = snap?.status === "running" && !view.stopping;
  const canSteer = engines[view.provider].steering;
  const rows = pending.map(entry => {
    const key = `${snap.id}:${entry.id}`;
    return [key, [entry, running, canSteer, view.sending, view.withdrawing], () => {
      const label = t(entry.delivery === "unsent"
        ? (entry.mode === "steer" ? "引导未确认" : "未发送")
        : (entry.mode === "steer" ? "正在引导" : "队列中"));
      const row = element("li", "", "queue-row");
      const content = element("div", "", "queue-content");
      const preview = element("div", entry.prompt.trim().replace(/\s+/g, " "), "queue-preview");
      preview.title = entry.prompt;
      const status = element("div", entry.error ? label + " · " + t(entry.error) : label, "queue-status");
      if (entry.delivery === "unsent") status.classList.add("unsent");
      content.append(preview, status);
      const attachments = runAttachments(snap.id, entry);
      if (attachments?.length) content.append(sentImages(attachments));
      row.append(content);
      if (running && entry.delivery === "pending" && entry.mode === "queue") {
        const actions = element("div", "", "queue-actions");
        const withdraw = button(t("撤回修改"), "queue-withdraw", () => withdrawQueued(snap.id, entry.id));
        withdraw.disabled = view.sending || Boolean(view.withdrawing);
        actions.append(withdraw);
        if (canSteer) {
          const steer = button(t("引导会话"), "queue-steer", event => steerQueued(snap.id, entry.id, event.currentTarget));
          steer.disabled = view.withdrawing === entry.id;
          actions.append(steer);
        }
        row.append(actions);
      }
      return row;
    }];
  });
  renderList(area, rows);
}

async function withdrawQueued(runId, messageId) {
  const target = view;
  if (target.run !== runId || target.stopping || target.sending || target.withdrawing) return;
  target.withdrawing = messageId;
  renderControls();
  notice(composerNotice, "");
  try {
    const result = await api(`/runs/${encodeURIComponent(runId)}/messages/${encodeURIComponent(messageId)}/withdraw`, {});
    const files = result.images.map(image => {
      const binary = atob(image.data);
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      const file = new File([bytes], image.name, {type: "image/png"});
      return {file, preview: URL.createObjectURL(file), normalized: true};
    });
    const draft = {prompt: result.prompt, files, withdrawn: true};
    if (view !== target) {
      if (paired) {
        recoveredWithdrawals.set(messageId, {runId, provider: target.provider, sessionId: target.sessionId, draft});
        if (restoreWithdrawals()) {
          renderDraftFiles();
          renderControls();
        }
      } else {
        releaseFiles(files);
      }
      return;
    }
    showDraft(draft);
    if (target.snapshot?.id === runId && target.snapshot.follow_ups) {
      target.snapshot.follow_ups = target.snapshot.follow_ups.filter(entry => entry.id !== messageId);
    }
    $("prompt").focus();
  } catch (problem) {
    if (view === target) notice(composerNotice, problem.message);
  } finally {
    target.withdrawing = null;
    if (view === target) {
      renderDraftFiles();
      renderControls();
    }
  }
}

async function steerQueued(runId, messageId, action) {
  if (view.run !== runId || view.stopping) return;
  action.disabled = true;
  notice(composerNotice, "");
  try {
    await api("/runs/" + encodeURIComponent(runId) + "/messages/" + encodeURIComponent(messageId) + "/steer", {});
  } catch (problem) {
    action.disabled = false;
    if (view.run === runId) notice(composerNotice, problem.message);
  }
}

function sentImages(attachments) {
  const files = element("div", "", "sent-images");
  for (const {name, url} of attachments) {
    const unavailable = () => element("span", t("{name} · 暂不可预览", {name}), "sent-image-unavailable");
    if (!url) {
      files.append(unavailable());
      continue;
    }
    const link = element("a");
    link.href = scopedAsset(url);
    link.title = name;
    openInNewTab(link);
    const img = element("img");
    img.alt = name;
    img.loading = "lazy";
    img.addEventListener("error", () => link.replaceWith(unavailable()), {once: true});
    img.src = scopedAsset(url);
    link.append(img);
    files.append(link);
  }
  return files;
}

function sentFiles(files) {
  const list = element("div", "", "sent-files");
  for (const {name, directory} of files) {
    const chip = element("span", "", "sent-file");
    chip.title = name;
    chip.append(element("span", "", directory ? "folder-mark" : "document-mark"),
      element("span", name, "sent-file-name"));
    list.append(chip);
  }
  return list;
}

function runAttachments(id, entry) {
  const path = "/api/runs/" + encodeURIComponent(id) + "/images/";
  return entry.images?.map((name, index) => ({name, url: path + ((entry.image_start || 0) + index)}));
}

function runPrompt(id, entry, row) {
  return message("user", entry.prompt, {attachments: runAttachments(id, entry), local_files: entry.local_files}, row);
}

function showDraft(draft) {
  const editor = $("prompt");
  if (editor.value.trim() || view.files.length) {
    view.otherDrafts.push({prompt: editor.value, files: view.files, withdrawn: view.editingWithdrawn,turnSettings:new Map([...view.turnSettings || []].map(([key,value]) => [key,{...value}]))});
  }
  editor.value = draft.prompt;
  view.files = draft.files;
  view.editingWithdrawn = draft.withdrawn;
  view.turnSettings = draft.turnSettings || new Map();
  renderDraftSettings();
}

function restoreWithdrawals() {
  if (view.sending || view.withdrawing) return false;
  let restored = false;
  for (const [id, {runId, provider, sessionId, draft}] of recoveredWithdrawals) {
    if (view.run === runId || (sessionId && view.provider === provider && view.sessionId === sessionId)) {
      recoveredWithdrawals.delete(id);
      showDraft(draft);
      restored = true;
    }
  }
  return restored;
}

function releaseFiles(files) {
  for (const entry of files) URL.revokeObjectURL(entry.preview);
}

function clearFiles(target) {
  releaseFiles(target.files);
  target.files = [];
  for (const draft of target.otherDrafts) releaseFiles(draft.files);
  target.otherDrafts = [];
  if (target.startupDraft) {
    releaseFiles(target.startupDraft.files);
    target.startupDraft = null;
  }
}

function addImages(files) {
  if (view.sending) return;
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
  const files = view.sending ? [] : view.files;
  area.hidden = !files.length;
  renderList(area, files.map(entry => [entry.preview, null, () => {
    const node = element("div", "", "draft-file");
    const img = new Image();
    img.src = entry.preview;
    img.alt = "";
    const remove = button("×","icon",() => {
      if (view.sending) return;
      view.files = view.files.filter(item => item !== entry);
      URL.revokeObjectURL(entry.preview);
      renderDraftFiles();
      renderControls();
    });
    remove.setAttribute("aria-label", t("移除 {name}", {name: entry.file.name}));
    node.append(img, element("span",entry.file.name),remove);
    return node;
  }]));
}

function upload(entry) {
  return new Promise((resolve,reject) => {
    const reader = new FileReader();
    reader.addEventListener("load", () => {
      const data = String(reader.result).split(",")[1];
      resolve(entry.normalized
        ? {name: "image.png", display_name: entry.file.name, data}
        : {name: entry.file.name, data});
    }, {once: true});
    reader.addEventListener("error", () => reject(new Error(t("无法读取图片 {name}", {name: entry.file.name}))), {once: true});
    reader.readAsDataURL(entry.file);
  });
}
function renderControls() {
  renderQuestion();
  renderPendingQueue();
  const hasProject = view.sessionId || ($("project-select").value && view.cwd.trim());
  const active = view.busy && Boolean(view.run);
  const draft = $("prompt").value;
  const hasDraft = Boolean(draft.trim() || view.files.length);
  const canSend = hasDraft && (active || hasProject) && !view.permissionLoad && !view.permissionSaving && !sessionPending.has(sessionKey(view));
  const showStop = active && !hasDraft && !view.sending;
  const action = $("compose-action");
  action.type = showStop ? "button" : "submit";
  action.title = t(showStop ? "停止" : "Enter 发送 · Shift+Enter 换行");
  action.classList.toggle("compose-stop", showStop);
  action.disabled = view.sending || view.stopping || Boolean(view.withdrawing) || !(showStop || canSend);
  $("compose-action-icon").className = showStop ? "stop-mark" : "send-mark";
  setText($("compose-action-label"), t(showStop
    ? (view.stopping ? "停止中" : "停止")
    : (view.sending ? "发送中" : "发送")));

  $("compose-editor").dataset.text = draft;
  $("draft-status").hidden = !view.editingWithdrawn && !view.otherDrafts.length;
  $("withdrawn-draft-label").hidden = !view.editingWithdrawn;
  $("switch-draft").hidden = !view.otherDrafts.length;
  $("switch-draft").disabled = view.sending || Boolean(view.withdrawing);
  $("add-images").disabled = view.sending || view.files.length >= 4;
  $("project-select").disabled = view.busy;
  $("project-root").disabled = view.busy;
  $("directory").disabled = view.busy;
  for (const option of $("new-engines").children) option.disabled = view.busy;
  renderModels();
}

function paint() {
  restoreWithdrawals();
  renderFilters();
  renderEngineChoices();
  renderHeader();
  renderSidebar();
  renderMessages();
  renderDraftFiles();
  renderRun();
  renderControls();
  scrollToLatest();
}

async function send() {
  const target = view;
  const action = $("compose-action");
  if (promptComposing || action.disabled || action.type !== "submit") return;
  if (target.permissionMode === undefined) {
    const loaded = await restorePermissionMode(target);
    if (!loaded || view !== target || action.disabled || action.type !== "submit") return;
  }
  const editor = $("prompt");
  const draft = editor.value;
  const prompt = draft.trim();
  const activeRun = target.run;
  const mode = engines[target.provider].steering && !target.editingWithdrawn ? sendMode : "queue";
  const catalog = models.get(target.provider);
  const defaultPermission = target.provider === "codex" ? catalog?.default_permission_mode : null;
  const options = {model: selectedModel(target, catalog), reasoning_effort: target.effort, fast_mode: target.fast,
    permission_mode: selectedPermission(target, catalog) ?? defaultPermission ?? null, collaboration_mode: target.collaborationMode};
  const directory = (target.data?.cwd || target.cwd).trim();
  const projectId = !target.sessionId && target.location.startsWith("project:") ? target.location.slice(8) : null;
  const configuration = draftConfiguration(target);
  const paths = value => value.split("\n").map(s => s.trim()).filter(Boolean);
  const turnConfiguration = {additional_directories:paths(configuration.directories),context_files:paths(configuration.files),
    agent:target.provider !== "codex" ? configuration.agent.trim() || null : null,
    worktree:isNewView(target) && target.provider === "claude" ? configuration.worktree.trim() || null : null};
  target.sending = true;
  if (!activeRun) target.busy = true;
  editor.value = "";
  renderDraftFiles();
  renderControls();
  notice(composerNotice, "");
  try {
    const images = await Promise.all(target.files.map(upload));
    const run = activeRun
      ? await api("/runs/" + activeRun + "/messages", {mode, prompt, images, options: mode === "queue" ? options : undefined})
      : await api("/runs", {
        provider: target.provider, session_id: target.sessionId, ...options,
        cwd: directory, project_id: projectId, prompt, images,
        ...turnConfiguration,
      });
    if (view !== target) return;
    if (run && !target.sessionId) {
      target.startupDraft = {prompt: draft, files: target.files, withdrawn: target.editingWithdrawn};
      localStorage.setItem(folderKey, directory);
      if (projectId) localStorage.setItem(projectKey, projectId);
      else localStorage.removeItem(projectKey);
    } else {
      releaseFiles(target.files);
    }
    target.files = [];
    target.editingWithdrawn = false;
    if (!editor.value.trim() && target.otherDrafts.length) showDraft(target.otherDrafts.shift());
    if (run) {
      target.snapshot = null;
      target.run = run.id;
      target.runModel = run.model ?? options.model;
      target.title = run.title;
      running = [...running.filter(entry => entry.id !== run.id), run];
      renderSidebar();
      renderHeader();
      renderRun();
      watchRun(target);
    }
  } catch (problem) {
    if (view === target) {
      if (!activeRun && !target.run) target.busy = false;
      editor.value = [draft, editor.value].filter(Boolean).join("\n\n");
      notice(composerNotice, problem.message);
    }
  } finally {
    target.sending = false;
    if (view === target) {
      restoreWithdrawals();
      renderDraftFiles();
      renderControls();
    }
  }
}

function stopWatchingRun() {
  runWatcher?.close();
  runWatcher = null;
}

function watchRun(target) {
  const runId = target.run;
  if (!paired || view !== target || !runId || (runWatcher?.target === target && runWatcher.runId === runId)) return;
  stopWatchingRun();
  const path = "/runs/" + encodeURIComponent(runId);
  let timer, source, redraw, retryStreamAt;
  let received = false;
  let failures = 0;
  const watcher = {target, runId, close() {
    clearTimeout(timer);
    cancelAnimationFrame(redraw);
    source?.close();
    target.replyReveal = null;
  }};
  runWatcher = watcher;
  const current = () => paired && runWatcher === watcher && view === target && target.run === runId;

  function drawReply(now) {
    redraw = null;
    if (!current()) return;
    const reveal = target.replyReveal;
    if (reveal) {
      const text = target.snapshot.replies.find(reply => reply.after === reveal.after).text;
      const frames = Math.max(1, Math.ceil((reveal.deadline - now) / 16));
      const step = Math.ceil((text.length - reveal.end) / frames);
      let end = reveal.end + step;
      // Keep an emoji's UTF-16 surrogate pair together.
      const next = text.charCodeAt(end);
      if (next >= 0xdc00 && next <= 0xdfff) end++;
      if (end < text.length) reveal.end = end;
      else target.replyReveal = null;
    }
    renderRun();
    if (target.replyReveal) redraw = requestAnimationFrame(drawReply);
  }

  function paceReply(reply, previousEnd) {
    const previous = target.replyReveal?.after === reply.after ? target.replyReveal : null;
    const end = previous?.end ?? previousEnd;
    const pending = reply.text.length - end;
    const duration = Math.min(480, Math.max(64, pending * 4));
    target.replyReveal = !reducedMotion.matches && pending > (previous ? 0 : 8)
      ? previous ?? {after: reply.after, end, deadline: performance.now() + duration}
      : null;
  }

  function receiveSnapshot(snap, animate) {
    const reply = snap.replies?.at(-1);
    if (animate && reply && snap.status === "running" && target.snapshot?.id === snap.id) {
      const before = target.snapshot.replies?.find(entry => entry.after === reply.after)?.text ?? "";
      if (reply.text.startsWith(before)) paceReply(reply, before.length);
      else target.replyReveal = null;
    } else {
      target.replyReveal = null;
    }
    applyRunSnapshot(target, snap);
    if (target.replyReveal && !redraw) redraw = requestAnimationFrame(drawReply);
  }

  async function pollSnapshot() {
    if (!current()) return;
    try {
      const snap = await api(path);
      if (current()) receiveSnapshot(snap, true);
    } catch (problem) {
      if (current()) notice(composerNotice, problem.message);
    }
    if (current()) {
      timer = setTimeout(retryStreamAt && Date.now() >= retryStreamAt ? connect : pollSnapshot, 450);
    }
  }

  function connect() {
    if (!current()) return;
    if (typeof EventSource !== "function") {
      void pollSnapshot();
      return;
    }
    const connection = new EventSource(devicePrefix + "/api" + path + "/events");
    source = connection;
    let ready = false;
    function disconnected() {
      if (!current() || source !== connection) return;
      clearTimeout(timer);
      connection.close();
      source = null;
      if (received && failures < 2) {
        failures++;
        timer = setTimeout(connect, failures * 400);
      } else {
        if (received) retryStreamAt = Date.now() + 10000;
        void pollSnapshot();
      }
    }
    timer = setTimeout(disconnected, 3500);
    connection.onerror = disconnected;
    connection.addEventListener("snapshot", event => {
      if (!current() || source !== connection) return;
      const snap = JSON.parse(event.data);
      if (snap.id !== runId) return;
      clearTimeout(timer);
      const animate = ready;
      ready = received = true;
      failures = 0;
      setConnection(true);
      receiveSnapshot(snap, animate);
    });
    connection.addEventListener("delta", event => {
      if (!ready || !current() || source !== connection) return;
      const delta = JSON.parse(event.data);
      const reply = applyRunDelta(target.snapshot, delta);
      if (reply) paceReply(reply, reply.text.length - delta.text.length);
      if (!redraw) redraw = requestAnimationFrame(drawReply);
    });
  }
  connect();
}

function applyRunDelta(snap, delta) {
  snap.activity = delta.activity;
  if (!delta.text) return;
  let reply = snap.replies.find(entry => entry.after === delta.after);
  if (!reply) {
    reply = {after: delta.after, text: ""};
    snap.replies.push(reply);
  }
  reply.text += delta.text;
  reply.streamingText = true;
  return reply;
}

function applyRunSnapshot(target, snap) {
  target.snapshot = snap;
  target.contextOverride = null;
  target.title = snap.title;
  target.runModel = snap.model ?? target.runModel;
  target.sessionId ||= snap.session_id;
  target.busy = snap.status === "running";
  if (!target.busy) {
    stopWatchingRun();
    if (target.data) target.data = {...target.data, title: snap.title};
    target.run = null;
    target.stopping = false;
    target.lastFinished = snap.id;
  }
  if (target.startupDraft && (target.sessionId || !target.busy)) {
    const draft = target.startupDraft;
    target.startupDraft = null;
    if (!target.sessionId && !target.sending && (draft.withdrawn || !$("prompt").value && !target.files.length)) {
      showDraft(draft);
    } else {
      releaseFiles(draft.files);
    }
    renderDraftFiles();
  }
  renderHeader();
  renderRun();
  renderControls();
  if (!target.busy) {
    running = running.filter(run => run.id !== snap.id);
    renderSidebar();
    if (target.sessionId) openHistory(target, true);
    if (!historyLoading) refreshHistories(false);
  }
}

async function syncRunning() {
  if (!paired || syncing) return;
  syncing = true;
  try {
    const activity = await api("/activity");
    if (!paired) return;
    // Preserve the order of existing rows so their spinners stay attached.
    const order = new Map(running.map(({id}, index) => [id, index]));
    const list = activity.running.sort((a, b) => (order.get(a.id) ?? order.size) - (order.get(b.id) ?? order.size));
    const changed = JSON.stringify(list) !== JSON.stringify(running);
    running = list;
    if (changed) renderSidebar();
    const latest = activity.finished[0]?.id;
    const titleRevision = activity.title_revision ?? 0;
    if (!historyLoading && ((latest && lastCompleted !== latest) || lastTitleRevision !== titleRevision)) {
      lastCompleted = latest;
      lastTitleRevision = titleRevision;
      refreshHistories(false);
    }
    if (!view.run && !view.busy && view.sessionId) {
      const sameSession = run => run.provider === view.provider && run.session_id === view.sessionId;
      const task = running.find(run => run.id !== view.lastFinished && sameSession(run));
      const finished = activity.finished.find(sameSession);
      if (task) {
        view.run = task.id;
        view.runModel = task.model ?? null;
        view.busy = true;
        renderControls();
        watchRun(view);
      } else if (finished && view.lastFinished !== finished.id) {
        view.lastFinished = finished.id;
        openHistory(view,true);
      } else if (view.provider === "codex" && view.historyRevision && document.visibilityState === "visible") {
        openHistory(view, false, true);
      }
    }
  } catch {} finally {
    syncing = false;
  }
}

async function decide(snap, allow) {
  try {
    await api("/runs/" + snap.id + "/approval", {approval_id: snap.approval.id, allow});
    if (view.run === snap.id && view.snapshot?.approval?.id === snap.approval.id) {
      view.snapshot = {...view.snapshot, approval: null};
      renderRun();
    }
  } catch (problem) {
    notice(composerNotice, problem.message);
  }
}

function questionField(question, index) {
  const field = element("fieldset", "", "question-group");
  const legend = element("legend");
  if (question.header && question.header !== question.text) {
    legend.append(element("span", question.header, "question-header"));
  }
  legend.append(element("span", question.text));
  field.append(legend);

  function choice(title, description) {
    const label = element("label", "", "question-choice");
    const input = element("input");
    input.type = question.multiple ? "checkbox" : "radio";
    input.name = "question-" + index;
    const copy = element("span");
    copy.append(element("strong", title));
    if (description) copy.append(element("small", description));
    label.append(input, copy);
    field.append(label);
    return input;
  }

  const choices = question.options.map(option => [choice(option.label, option.description), option.label]);
  let customChoice, custom;
  if (question.allow_custom) {
    if (choices.length) customChoice = choice(t("自定义答案"));
    custom = element("input", "", "question-custom");
    custom.type = question.secret ? "password" : "text";
    custom.autocomplete = "off";
    custom.hidden = Boolean(customChoice);
    field.append(custom);
  }
  const customTitle = customChoice?.parentElement.querySelector("strong");
  function translate() {
    if (!custom) return;
    if (customTitle) setText(customTitle, t("自定义答案"));
    custom.placeholder = t("输入答案");
    custom.setAttribute("aria-label", t("{question} 自定义答案", {question: question.header || question.text}));
  }
  translate();
  field.addEventListener("change", () => {
    if (customChoice) custom.hidden = !customChoice.checked;
    updateQuestionSubmit();
  });
  custom?.addEventListener("input", updateQuestionSubmit);
  return {field, translate, answer() {
    const values = choices.filter(([input]) => input.checked).map(([, label]) => label);
    if (custom && !custom.hidden) {
      if (!custom.value.trim()) return [];
      values.push(custom.value.trim());
    }
    return [...new Set(values)];
  }};
}

function updateQuestionSubmit() {
  $("question-submit").disabled = !questionForm || questionForm.sending || questionForm.submitted ||
    questionForm.fields.some(({answer}) => !answer().length);
}

function openQuestion() {
  if (!questionForm || questionForm.submitted || questionDialog.open || !quotaPage.hidden) return;
  closePicker();
  closeComposerSettings();
  questionDialog.showModal();
}

function renderQuestion() {
  const snap = view.snapshot;
  const request = paired && snap?.status === "running" && !view.stopping ? snap.question : null;
  if (!request) {
    if (questionForm) {
      questionDialog.close();
      $("question-fields").replaceChildren();
      questionForm = null;
    }
    $("question-reopen").hidden = true;
    return;
  }
  if (questionForm?.id !== request.id) {
    questionForm = {
      run: snap.id, id: request.id, questions: request.questions,
      fields: request.questions.map(questionField), sending: false, submitted: false,
      deferred: !quotaPage.hidden,
    };
    $("question-fields").replaceChildren(...questionForm.fields.map(({field}) => field));
    $("question-fields").scrollTop = 0;
    notice($("question-error"), "");
    updateQuestionSubmit();
    openQuestion();
  }
  $("question-reopen").hidden = questionForm.submitted;
}

async function answerQuestion(event) {
  event.preventDefault();
  const form = questionForm;
  if (!form || $("question-submit").disabled) return;
  form.sending = true;
  updateQuestionSubmit();
  notice($("question-error"), "");
  try {
    const answers = Object.fromEntries(form.questions.map((question, index) => [question.id, form.fields[index].answer()]));
    await api("/runs/" + form.run + "/answer", {question_id: form.id, answers});
    form.submitted = true;
    if (questionForm === form) {
      questionDialog.close();
      $("question-reopen").hidden = true;
    }
  } catch (problem) {
    if (questionForm === form) notice($("question-error"), problem.message);
  } finally {
    form.sending = false;
    updateQuestionSubmit();
  }
}

$("question-form").addEventListener("submit", answerQuestion);
$("question-reopen").addEventListener("click", openQuestion);
$("question-later").addEventListener("click", () => questionDialog.close());

$("pair-form").addEventListener("submit", event => {
  event.preventDefault();
  pair($("pair-code").value.trim());
});
$("gate-devices").addEventListener("click", showDevices);
for (const id of providers) {
  const choice = button(engines[id].label, "provider-choice", () => {
    if (id === quotaProvider) return;
    quotaProvider = id;
    loadQuota();
  });
  choice.value = id;
  $("quota-engines").append(choice);
}
$("open-quota").addEventListener("click", event => {
  if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
  event.preventDefault();
  history.pushState({quotaEntry: true}, "", "#usage");
  syncQuotaPage();
});
for (const choice of $("usage-tabs").children) {
  choice.addEventListener("click", () => {
    const hash = choice.dataset.usageSection === "tokens" ? "#usage" : "#quota";
    history.replaceState(history.state, "", hash);
    syncQuotaPage();
  });
}
for (const choice of $("token-periods").children) {
  choice.addEventListener("click", () => {
    tokenPeriod = choice.value;
    for (const option of $("token-periods").children) {
      option.setAttribute("aria-pressed", String(option === choice));
    }
    renderTokenUsage();
  });
}
$("back-quota").addEventListener("click", () => {
  if (history.state?.quotaEntry) {
    history.back();
  } else {
    history.replaceState(null, "", location.pathname + location.search);
    syncQuotaPage();
  }
});
window.addEventListener("popstate", syncQuotaPage);
window.addEventListener("hashchange", () => {
  const code = takePairingCode();
  if (code && !paired) initialize(code);
  syncQuotaPage();
});
$("refresh-quota").addEventListener("click", () => {
  if (usageSection === "tokens") loadTokenUsage();
  else if (usageSection === "quota") loadQuota();
});
$("start-quota-reset").addEventListener("click", () => {
  quotaResetAttempt = {id: crypto.randomUUID()};
  quotaResetNotice = "";
  renderQuotaReset();
  $("confirm-quota-reset").focus();
});
$("cancel-quota-reset").addEventListener("click", () => {
  quotaResetAttempt = null;
  renderQuotaReset();
  $("start-quota-reset").focus();
});
$("confirm-quota-reset").addEventListener("click", submitQuotaReset);
function bindSettingsDropdown(trigger, choose) {
  const list = $(trigger.getAttribute("aria-controls"));
  const caption = $(trigger.id + "-value");
  const options = [...list.children];
  let selectedIndex = 0;
  let activeIndex = 0;

  function highlight(index) {
    options[activeIndex].classList.remove("is-active");
    activeIndex = index;
    const option = options[index];
    option.classList.add("is-active");
    trigger.setAttribute("aria-activedescendant", option.id);
    option.scrollIntoView({block: "nearest"});
  }

  function position() {
    list.classList.remove("is-up");
    const dialogRect = settingsDialog.getBoundingClientRect();
    const triggerRect = trigger.getBoundingClientRect();
    const spaceBelow = dialogRect.bottom - triggerRect.bottom;
    const spaceAbove = triggerRect.top - dialogRect.top;
    if (spaceBelow < list.offsetHeight + 12 && spaceAbove > spaceBelow) {
      list.classList.add("is-up");
    }
  }

  function open() {
    trigger.focus({preventScroll: true});
    list.hidden = false;
    position();
    trigger.setAttribute("aria-expanded", "true");
    highlight(selectedIndex);
  }

  function close() {
    if (list.hidden) return;
    list.hidden = true;
    list.classList.remove("is-up");
    trigger.setAttribute("aria-expanded", "false");
    trigger.removeAttribute("aria-activedescendant");
    options[activeIndex].classList.remove("is-active");
  }

  function commit(option) {
    close();
    if (option !== options[selectedIndex]) choose(option.dataset.value);
    trigger.focus({preventScroll: true});
  }

  trigger.addEventListener("click", () => list.hidden ? open() : close());
  trigger.addEventListener("keydown", event => {
    if (event.key === "Tab") {
      close();
      return;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (list.hidden) open();
      else {
        const step = event.key === "ArrowDown" ? 1 : -1;
        highlight(Math.min(options.length - 1, Math.max(0, activeIndex + step)));
      }
    } else if (!list.hidden && (event.key === "Home" || event.key === "End")) {
      event.preventDefault();
      highlight(event.key === "Home" ? 0 : options.length - 1);
    } else if (!list.hidden && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault();
      commit(options[activeIndex]);
    }
  });
  list.addEventListener("click", event => {
    const option = event.target.closest('[role="option"]');
    if (option) commit(option);
  });
  settingsDialog.addEventListener("click", event => {
    if (!trigger.parentElement.contains(event.target) && ![...(trigger.labels ?? [])].some(label => label.contains(event.target))) close();
  });
  settingsDialog.addEventListener("cancel", event => {
    if (list.hidden) return;
    event.preventDefault();
    close();
    trigger.focus({preventScroll: true});
  });
  settingsDialog.addEventListener("close", () => {
    if (!settingsDialog.open) close();
  });

  return value => {
    selectedIndex = options.findIndex(option => option.dataset.value === value);
    const selected = options[selectedIndex];
    for (const option of options) option.setAttribute("aria-selected", String(option === selected));
    setText(caption, selected.textContent);
    if (selected.lang) caption.lang = selected.lang;
  };
}

const renderLanguageChoice = bindSettingsDropdown(languageTrigger, value => {
  applyLanguage(value);
  localStorage.setItem(languageKey, language);
});
const renderSendModeChoice = bindSettingsDropdown($("send-mode"), value => {
  sendMode = value;
  renderSendModeChoice(sendMode);
  localStorage.setItem(sendModeKey, sendMode);
});
const renderAppearanceChoice = bindSettingsDropdown($("appearance"), value => {
  window.naroAppearance.set(value);
});
window.addEventListener("naro-appearance-change", () => {
  renderAppearanceChoice(window.naroAppearance.preference);
});
function openSettings() {
  settingsDialog.showModal();
}
$("open-settings").addEventListener("click", openSettings);
$("gate-settings").addEventListener("click", openSettings);
$("settings-close").addEventListener("click", () => settingsDialog.close());
$("switch-device").addEventListener("click", showDevices);
$("device-close").addEventListener("click", () => deviceDialog.close());
for (const entry of [$("gate-scan"), $("device-scan")]) entry.addEventListener("click", startScanner);
$("scan-close").addEventListener("click", closeScanner);
scanDialog.addEventListener("cancel", event => {
  event.preventDefault();
  closeScanner();
});
scanDialog.addEventListener("close", () => { if (!scanDialog.open) stopScanner(); });
window.addEventListener("pagehide", closeScanner);
$("device-form").addEventListener("submit", event => {
  event.preventDefault();
  const url = connectionAddress($("device-address").value);
  if (url && !url.hash) {
    location.assign(url.href);
    return;
  }
  notice($("device-notice"), "请粘贴目标电脑上的设备地址。");
});
$("composer").addEventListener("submit", event => {
  event.preventDefault();
  send();
});
$("compose-action").addEventListener("click", event => {
  if (event.currentTarget.type === "button") stop();
});
$("add-images").addEventListener("click", () => $("image-files").click());
$("switch-draft").addEventListener("click", () => {
  const draft = view.otherDrafts.shift();
  if (!draft) return;
  showDraft(draft);
  renderDraftFiles();
  renderControls();
  $("prompt").focus();
});
$("image-files").addEventListener("change", () => {
  addImages($("image-files").files);
  $("image-files").value = "";
});
const promptEditor = $("prompt");
promptEditor.addEventListener("paste", event => {
  const clipboard = event.clipboardData;
  if (!clipboard) return;
  const isImage = file => file && (file.type.startsWith("image/") || /\.(png|jpe?g|gif|webp)$/i.test(file.name));
  let files = Array.from(clipboard.files || []).filter(isImage);
  if (!files.length) {
    files = Array.from(clipboard.items || [], item => item.kind === "file" ? item.getAsFile() : null).filter(isImage);
  }
  if (!files.length) return;
  event.preventDefault();
  if (view.sending) {
    notice(composerNotice, "消息正在发送，请稍后再粘贴图片。");
    return;
  }
  addImages(files);
});
promptEditor.addEventListener("input", renderControls);
promptEditor.addEventListener("compositionstart", () => { promptComposing = true; });
promptEditor.addEventListener("compositionend", () => { promptComposing = false; });
promptEditor.addEventListener("blur", () => { promptComposing = false; });
promptEditor.addEventListener("keydown", event => {
  // Safari can report the Enter that confirms an IME candidate with keyCode 229.
  if (event.key !== "Enter" || promptComposing || event.isComposing || event.keyCode === 229 ||
      event.shiftKey || event.ctrlKey || event.metaKey || event.altKey) return;
  event.preventDefault();
  if (!event.repeat) send();
});
for (const trigger of [$("project-select"), $("project-root")]) {
  trigger.addEventListener("click", () => openPicker(trigger));
  trigger.addEventListener("keydown", event => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    openPicker(trigger);
  });
}
$("picker-search").addEventListener("input", renderPickerOptions);
$("picker-close").addEventListener("click", () => closePicker(true));
picker.addEventListener("cancel", event => {
  event.preventDefault();
  closePicker(true);
});
picker.addEventListener("click", event => {
  if (event.target === picker) closePicker(true);
});
picker.addEventListener("keydown", event => {
  if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) return;
  const list = $("picker-options");
  const fromSearch = event.target === $("picker-search") && event.key === "ArrowDown";
  if (!list.contains(event.target) && !fromSearch) return;
  const rows = [...list.children];
  if (!rows.length) return;
  const index = rows.indexOf(event.target);
  const previous = list.querySelector('[tabindex="0"]');
  let next = 0;
  if (event.key === "End") next = rows.length - 1;
  if (event.key === "ArrowDown") next = Math.min(index + 1, rows.length - 1);
  if (event.key === "ArrowUp") next = Math.max(index - 1, 0);
  event.preventDefault();
  if (previous) previous.tabIndex = -1;
  rows[next].tabIndex = 0;
  rows[next].focus();
});
window.addEventListener("resize", positionPicker);
$("directory").addEventListener("input", () => {
  view.cwd = $("directory").value;
  renderControls();
  if (typeof syncGitCwd === "function") syncGitCwd();
});
for (const [trigger, panel] of composerSettings) {
  trigger.addEventListener("click", () => {
    const opening = panel.hidden;
    closeComposerSettings(!opening);
    if (!opening) return;
    panel.hidden = false;
    trigger.setAttribute("aria-expanded", "true");
    if (trigger.id === "models") loadModels(view.provider);
    else renderModels();
    const choice = panel.querySelector('[aria-pressed="true"]') ?? panel.querySelector("button");
    choice?.focus({preventScroll: true});
  });
}
document.addEventListener("pointerdown", event => {
  if (!composerSettings.some(([trigger, panel]) => trigger.contains(event.target) || panel.contains(event.target))) {
    closeComposerSettings();
  }
});
$("model-speed").addEventListener("click", event => {
  const toggle = event.currentTarget;
  if (toggle.hidden) return;
  view.model = selectedModel(view);
  view.fast = toggle.getAttribute("aria-pressed") !== "true";
  rememberModelPreference();
  renderModels();
});
$("refresh-models").addEventListener("click", () => loadModels(view.provider, true));
$("session-search").addEventListener("input", renderSidebar);
$("new-chat").addEventListener("click", startNewFromCurrent);
$("quick-new").addEventListener("click", startNewFromCurrent);
$("refresh").addEventListener("click", () => refreshHistories(true));
enableConversationSwipe();
$("open-drawer").addEventListener("click", () => drawer(true));
$("close-drawer").addEventListener("click", () => { drawer(false); $("open-drawer").focus(); });
$("scrim").addEventListener("click", () => { drawer(false); $("open-drawer").focus(); });
document.addEventListener("keydown", event => {
  if (event.key !== "Escape" || deviceDialog.open || scanDialog.open || settingsDialog.open || !quotaPage.hidden) return;
  if (typeof gitState !== "undefined" && gitState.open) {
    closeGitDrawer();
    $("git-toggle").focus();
    event.preventDefault();
  } else if (closeComposerSettings(true)) {
    event.preventDefault();
  } else if (!$("scrim").hidden) {
    drawer(false);
    $("open-drawer").focus();
  }
});
$("disconnect").addEventListener("click", async () => {
  try { await api("/logout", {}); }
  catch (problem) { notice($("device-notice"), problem.message); return; }
  rememberDevice(null);
  rememberQuotaReset(null);
  deviceName = "";
  deviceDialog.close();
  setPaired(false);
});
async function stop() {
  const target = view;
  if (!target.run || target.stopping) return;
  target.stopping = true;
  renderControls();
  try {
    await api("/runs/" + target.run + "/stop", {});
  } catch (problem) {
    if (view !== target) return;
    target.stopping = false;
    notice(composerNotice, problem.message);
    renderControls();
  }
}
setInterval(syncRunning, 2000);
conversation.addEventListener("scroll", () => {
  if (isNewView(view) || !quotaPage.hidden) return;
  view.followLatest = conversation.scrollHeight - conversation.clientHeight - conversation.scrollTop < 16;
}, {passive: true});
if ("ResizeObserver" in window) {
  const followResize = new ResizeObserver(scrollToLatest);
  followResize.observe(conversation);
  followResize.observe(conversation.firstElementChild);
}
document.addEventListener("visibilitychange", () => {
  syncRunningSpinners();
  if (document.visibilityState === "visible") {
    loadModels(view.provider);
    syncRunning();
  } else if (scanDialog.open) closeScanner();
});
reducedMotion.addEventListener("change", syncRunningSpinners);
window.addEventListener("online", () => {
  loadModels(view.provider);
  syncRunning();
});
narrowLayout.addEventListener("change", () => drawer(false));
setupRailResize();

// On iOS the keyboard shrinks the visual viewport while the layout viewport can stay the same.
const viewport = window.visualViewport;
if (viewport) {
  const fitKeyboard = () => {
    const style = document.documentElement.style;
    if (narrowLayout.matches && viewport.scale < 1.05 && window.innerHeight - viewport.height > 90) {
      style.setProperty("--viewport-height", Math.round(viewport.height) + "px");
      style.setProperty("--bottom-space", ".45rem");
    } else {
      style.removeProperty("--viewport-height");
      style.removeProperty("--bottom-space");
    }
  };
  viewport.addEventListener("resize", fitKeyboard);
  narrowLayout.addEventListener("change", fitKeyboard);
  fitKeyboard();
}

async function initializeLanguage() {
  try {
    const response = await fetch("/i18n.json");
    if (response.ok) {
      translations = await response.json();
      const selected = localStorage.getItem(languageKey);
      if (translationIndex.has(selected)) applyLanguage(selected);
      languageTrigger.disabled = false;
    }
  } catch {}
}

function takePairingCode() {
  const code = new URLSearchParams(location.hash.slice(1)).get("pair");
  if (code === null) return null;
  history.replaceState(null, "", location.pathname + location.search);
  return /^\d{8}$/.test(code) ? code : null;
}

async function initialize(code = takePairingCode()) {
  if (code && await pair(code)) return;
  try {
    const access = await api("/access");
    if (paired) return;
    if (access.paired) {
      setPaired(true, access.name);
    } else if (!devicePrefix && access.online === false) {
      for (const id of ["pair-label", "pair-code", "pair-submit"]) $(id).hidden = true;
      renderPairInstructions();
    } else if (access.online === false) {
      notice(pairNotice, "目标电脑未连接，请在目标电脑上打开 Naro。");
    }
  } catch (problem) {
    if (!code) notice(pairNotice, problem.message);
  }
}

if ("serviceWorker" in navigator) navigator.serviceWorker.register("/sw.js").catch(() => {});

// Git workspace drawer
const gitState = {
  open: false,
  cwd: "",
  tab: "changes",
  status: null,
  loading: false,
  pending: false,
  generatingMessage: false,
  serial: 0,
  diffSerial: 0,
  generateSerial: 0,
  error: "",
  notice: "",
  commitMessage: "",
  amend: false,
  newBranch: "",
  selectedDiff: null,
  diff: null,
  diffLoading: false,
  diffError: "",
  confirmDiscard: null,
};

function currentGitCwd() {
  return (view.data?.cwd || view.cwd || "").trim();
}

function updateGitTrigger() {
  const trigger = $("git-toggle");
  const cwd = currentGitCwd();
  trigger.hidden = !cwd;
  trigger.setAttribute("aria-expanded", String(gitState.open));
  let label = "Git";
  if (gitState.status?.is_repo && gitState.cwd === cwd) {
    const st = gitState.status;
    const branch = st.branch || st.head_short || "Git";
    const total = (st.staged?.length || 0) + (st.unstaged?.length || 0) + (st.untracked?.length || 0) + (st.conflicted?.length || 0);
    const shortBranch = branch.length > 16 ? branch.slice(0, 15) + "…" : branch;
    label = total > 0 ? `${shortBranch} · ${total}` : shortBranch;
  }
  setText($("git-toggle-label"), label);
}

function syncGitCwd() {
  const cwd = currentGitCwd();
  if (gitState.cwd !== cwd) {
    gitState.cwd = cwd;
    gitState.status = null;
    gitState.error = "";
    gitState.notice = "";
    gitState.selectedDiff = null;
    gitState.diff = null;
    gitState.diffError = "";
    gitState.diffLoading = false;
    gitState.confirmDiscard = null;
    gitState.generatingMessage = false;
    gitState.serial += 1;
    gitState.diffSerial += 1;
    gitState.generateSerial += 1;
    if (gitState.open) {
      if (cwd) refreshGitStatus();
      else renderGit();
    }
  }
  updateGitTrigger();
}

function openGitDrawer() {
  const cwd = currentGitCwd();
  if (!cwd) return;
  gitState.open = true;
  gitState.cwd = cwd;
  $("git-drawer").hidden = false;
  $("git-backdrop").hidden = false;
  updateGitTrigger();
  refreshGitStatus();
}

function closeGitDrawer() {
  gitState.open = false;
  gitState.confirmDiscard = null;
  $("git-drawer").hidden = true;
  $("git-backdrop").hidden = true;
  updateGitTrigger();
}

async function refreshGitStatus() {
  const cwd = currentGitCwd();
  if (!cwd) {
    renderGit();
    return;
  }
  gitState.cwd = cwd;
  const serial = ++gitState.serial;
  gitState.loading = true;
  gitState.error = "";
  renderGit();
  try {
    const query = new URLSearchParams({cwd}).toString();
    const status = await api("/git/status?" + query);
    if (serial !== gitState.serial) return;
    gitState.loading = false;
    gitState.status = status;
    pruneGitSelectedDiff(status);
    renderGit();
    updateGitTrigger();
  } catch (problem) {
    if (serial !== gitState.serial) return;
    gitState.loading = false;
    gitState.error = problem.message || t("无法读取 Git 状态");
    renderGit();
  }
}

function pruneGitSelectedDiff(status) {
  const sel = gitState.selectedDiff;
  if (!sel || !status?.is_repo) {
    gitState.selectedDiff = null;
    gitState.diff = null;
    return;
  }
  const keep = sel.scope === "staged"
    ? (status.staged || []).some(item => item.path === sel.path)
    : sel.scope === "unstaged"
      ? (status.unstaged || []).some(item => item.path === sel.path) || (status.conflicted || []).some(item => item.path === sel.path)
      : sel.scope === "untracked"
        ? (status.untracked || []).some(item => item.path === sel.path)
        : (status.commits || []).some(item => item.hash === sel.commit);
  if (!keep) {
    gitState.selectedDiff = null;
    gitState.diff = null;
    gitState.diffError = "";
    gitState.diffLoading = false;
  }
}

async function loadGitDiff(selection) {
  const cwd = gitState.cwd || currentGitCwd();
  if (!cwd) return;
  gitState.selectedDiff = selection;
  const serial = ++gitState.diffSerial;
  gitState.diffLoading = true;
  gitState.diffError = "";
  gitState.diff = null;
  renderGit();
  try {
    const params = new URLSearchParams({cwd, scope: selection.scope});
    if (selection.path) params.set("path", selection.path);
    if (selection.commit) params.set("commit", selection.commit);
    const diff = await api("/git/diff?" + params.toString());
    if (serial !== gitState.diffSerial) return;
    gitState.diffLoading = false;
    gitState.diff = diff;
    renderGit();
  } catch (problem) {
    if (serial !== gitState.diffSerial) return;
    gitState.diffLoading = false;
    gitState.diffError = problem.message || t("无法读取文件差异");
    renderGit();
  }
}

async function runGitAction(action) {
  if (gitState.pending || gitState.generatingMessage) return;
  const cwd = gitState.cwd || currentGitCwd();
  if (!cwd) return;
  const serial = ++gitState.serial;
  gitState.pending = true;
  gitState.error = "";
  gitState.notice = "";
  renderGit();
  try {
    const result = await api("/git/action", {cwd, ...action});
    if (serial !== gitState.serial) return;
    gitState.pending = false;
    gitState.confirmDiscard = null;
    gitState.notice = result.summary || "";
    if (action.action === "commit") {
      gitState.commitMessage = "";
      gitState.amend = false;
    }
    if (action.action === "create_branch") {
      gitState.newBranch = "";
    }
    gitState.status = result.status;
    pruneGitSelectedDiff(result.status);
    renderGit();
    updateGitTrigger();
    if (gitState.selectedDiff) loadGitDiff(gitState.selectedDiff);
  } catch (problem) {
    if (serial !== gitState.serial) return;
    gitState.pending = false;
    gitState.error = problem.message || t("操作失败");
    renderGit();
  }
}

async function generateGitCommitMessage() {
  if (gitState.pending || gitState.generatingMessage) return;
  const cwd = gitState.cwd || currentGitCwd();
  if (!cwd) return;
  const serial = ++gitState.generateSerial;
  gitState.generatingMessage = true;
  gitState.error = "";
  gitState.notice = "";
  renderGit();
  try {
    const result = await api("/git/commit-message", {
      cwd,
      amend: gitState.amend,
      provider: view.provider,
      model: selectedModel(view) || null,
      language,
    });
    if (serial !== gitState.generateSerial) return;
    gitState.generatingMessage = false;
    gitState.commitMessage = result.message || "";
    gitState.notice = "已生成提交信息";
    renderGit();
  } catch (problem) {
    if (serial !== gitState.generateSerial) return;
    gitState.generatingMessage = false;
    gitState.error = problem.message || t("操作失败");
    renderGit();
  }
}

function gitStatusClass(code) {
  if (code === "A" || code === "?") return "is-added";
  if (code === "D") return "is-removed";
  if (code.length > 1) return "is-conflict";
  return "is-modified";
}

function renderGitDiffCard() {
  const card = element("section", "", "git-diff-card");
  const head = element("div", "", "git-diff-head");
  const title = gitState.diff?.title || gitState.selectedDiff?.path || gitState.selectedDiff?.commit?.slice(0, 12) || "";
  head.append(element("span", title, "git-diff-title"));
  const meta = element("div", "", "git-diff-meta");
  if (gitState.diff) {
    if (gitState.diff.added > 0) meta.append(element("span", "+" + gitState.diff.added, "change-added"));
    if (gitState.diff.removed > 0) meta.append(element("span", "-" + gitState.diff.removed, "change-removed"));
  }
  const closeBtn = element("button", "×", "git-icon-btn");
  closeBtn.type = "button";
  closeBtn.setAttribute("aria-label", t("关闭差异"));
  closeBtn.addEventListener("click", () => {
    gitState.selectedDiff = null;
    gitState.diff = null;
    gitState.diffError = "";
    gitState.diffLoading = false;
    renderGit();
  });
  meta.append(closeBtn);
  head.append(meta);
  card.append(head);

  if (gitState.diffLoading) {
    card.append(element("p", t("正在加载差异…"), "session-hint"));
  } else if (gitState.diffError) {
    card.append(element("p", gitState.diffError, "notice"));
  } else if (gitState.diff?.binary) {
    card.append(element("p", t("二进制文件，无法显示文本差异。"), "session-hint"));
  } else if (!gitState.diff?.diff) {
    card.append(element("p", t("没有文本差异。"), "session-hint"));
  } else {
    card.append(fileDiff({
      diff: gitState.diff.diff,
      added: gitState.diff.added,
      removed: gitState.diff.removed,
      truncated: gitState.diff.truncated,
    }));
  }
  return card;
}

function renderGitFileGroup(title, files, kind) {
  const frag = document.createDocumentFragment();
  const header = element("div", "", "git-group-header");
  header.append(element("h3", `${title} (${files.length})`));
  if (kind === "staged") {
    const btn = element("button", t("全部取消暂存"), "git-link-btn is-muted");
    btn.type = "button";
    btn.disabled = gitState.pending;
    btn.addEventListener("click", () => runGitAction({action: "unstage", paths: []}));
    header.append(btn);
  } else if (kind === "unstaged" || kind === "untracked") {
    const btn = element("button", t("全部暂存"), "git-link-btn");
    btn.type = "button";
    btn.disabled = gitState.pending;
    btn.addEventListener("click", () => runGitAction({action: "stage", paths: files.map(item => item.path)}));
    header.append(btn);
  }
  frag.append(header);

  const list = element("div", "", "git-file-list");
  for (const file of files) {
    const scope = kind === "staged" ? "staged" : kind === "untracked" ? "untracked" : "unstaged";
    const selected = gitState.selectedDiff?.scope === scope && gitState.selectedDiff?.path === file.path;
    const confirming = gitState.confirmDiscard?.path === file.path && gitState.confirmDiscard?.untracked === (kind === "untracked");
    const row = element("div", "", "git-file-row" + (selected ? " is-selected" : ""));
    row.append(element("span", file.status, "git-status-badge " + gitStatusClass(file.status)));
    const display = file.orig_path ? `${file.orig_path} → ${file.path}` : file.path;
    const nameBtn = element("button", display, "git-file-name");
    nameBtn.type = "button";
    nameBtn.title = display;
    nameBtn.addEventListener("click", () => loadGitDiff({scope, path: file.path}));
    row.append(nameBtn);

    const actions = element("div", "", "git-row-actions");
    if (confirming) {
      const yes = element("button", t("确认丢弃"), "git-link-btn is-danger");
      yes.type = "button";
      yes.disabled = gitState.pending;
      yes.addEventListener("click", () => runGitAction({action: "discard", paths: [file.path], untracked: kind === "untracked"}));
      const no = element("button", t("取消"), "git-link-btn is-muted");
      no.type = "button";
      no.disabled = gitState.pending;
      no.addEventListener("click", () => { gitState.confirmDiscard = null; renderGit(); });
      actions.append(yes, no);
    } else if (kind === "staged") {
      const unstage = element("button", "−", "git-icon-btn");
      unstage.type = "button";
      unstage.title = t("取消暂存");
      unstage.setAttribute("aria-label", t("取消暂存"));
      unstage.disabled = gitState.pending;
      unstage.addEventListener("click", () => runGitAction({action: "unstage", paths: [file.path]}));
      actions.append(unstage);
    } else {
      const stage = element("button", "+", "git-icon-btn is-add");
      stage.type = "button";
      stage.title = t("暂存");
      stage.setAttribute("aria-label", t("暂存"));
      stage.disabled = gitState.pending;
      stage.addEventListener("click", () => runGitAction({action: "stage", paths: [file.path]}));
      actions.append(stage);
      if (kind === "unstaged" || kind === "untracked") {
        const discard = element("button", "↩", "git-icon-btn is-danger");
        discard.type = "button";
        discard.title = t("丢弃更改");
        discard.setAttribute("aria-label", t("丢弃更改"));
        discard.disabled = gitState.pending;
        discard.addEventListener("click", () => {
          gitState.confirmDiscard = {path: file.path, untracked: kind === "untracked"};
          renderGit();
        });
        actions.append(discard);
      }
    }
    row.append(actions);
    list.append(row);
  }
  frag.append(list);
  return frag;
}

function renderGit() {
  updateGitTrigger();
  if (!gitState.open) return;
  const st = gitState.status;
  const isRepo = Boolean(st?.is_repo);
  setText($("git-repo-name"), isRepo ? (st.repo_name || "Git") : "Git");

  const branchPill = $("git-branch-pill");
  if (isRepo) {
    setText(branchPill, st.detached ? `HEAD ${st.head_short || ""}`.trim() : (st.branch || "HEAD"));
    branchPill.hidden = false;
  } else {
    branchPill.hidden = true;
  }

  const ab = $("git-ahead-behind");
  if (isRepo && (st.ahead > 0 || st.behind > 0)) {
    const parts = [];
    if (st.ahead > 0) parts.push("↑" + st.ahead);
    if (st.behind > 0) parts.push("↓" + st.behind);
    setText(ab, parts.join(" "));
    ab.hidden = false;
  } else {
    ab.hidden = true;
  }

  $("git-refresh").classList.toggle("is-spinning", gitState.loading || gitState.pending);
  $("git-refresh").disabled = gitState.pending;

  const totalChanges = isRepo
    ? (st.staged?.length || 0) + (st.unstaged?.length || 0) + (st.untracked?.length || 0) + (st.conflicted?.length || 0)
    : 0;
  const branchesCount = isRepo ? (st.branches?.length || 0) : 0;
  setText($("git-tab-changes"), totalChanges > 0 ? `${t("变更")} (${totalChanges})` : t("变更"));
  setText($("git-tab-branches"), branchesCount > 0 ? `${t("分支")} (${branchesCount})` : t("分支"));
  setText($("git-tab-history"), t("历史"));
  for (const tabBtn of $("git-tabs").querySelectorAll(".git-tab")) {
    tabBtn.setAttribute("aria-selected", String(tabBtn.dataset.tab === gitState.tab));
  }

  const sync = $("git-sync");
  const hasRemotes = isRepo && (st.remotes?.length || 0) > 0;
  sync.hidden = !isRepo;
  for (const id of ["git-fetch", "git-pull", "git-push"]) {
    $(id).disabled = gitState.pending || !hasRemotes;
  }

  const errorBanner = $("git-error");
  errorBanner.hidden = !gitState.error;
  if (gitState.error) setText(errorBanner, gitState.error);

  const noticeBanner = $("git-notice");
  noticeBanner.hidden = Boolean(gitState.error) || !gitState.notice;
  if (gitState.notice) setText(noticeBanner, t(gitState.notice));

  const body = $("git-body");
  body.replaceChildren();

  if (!gitState.cwd) {
    body.append(element("p", t("请先选择工作目录。"), "git-empty"));
    return;
  }
  if (!st) {
    body.append(element("p", t(gitState.loading ? "正在读取 Git 状态…" : "无法读取 Git 状态"), "git-empty"));
    return;
  }
  if (!st.is_repo) {
    body.append(element("p", t("当前工作目录不是 Git 仓库。"), "git-empty"));
    return;
  }

  if (gitState.tab === "changes") {
    const busy = gitState.pending || gitState.generatingMessage;
    const form = element("form", "", "git-card");
    const textarea = element("textarea", "", "git-commit-input");
    textarea.placeholder = t("输入提交信息…");
    textarea.value = gitState.commitMessage;
    textarea.disabled = busy;
    const syncButtons = () => {
      commitBtn.disabled = busy || (!gitState.amend && !gitState.commitMessage.trim()) || (!st.staged?.length && !gitState.amend);
      aiBtn.disabled = busy || (totalChanges === 0 && !(gitState.amend && st.commits?.length));
    };
    textarea.addEventListener("input", () => {
      gitState.commitMessage = textarea.value;
      syncButtons();
    });
    const bar = element("div", "", "git-commit-bar");
    const amendLabel = element("label", "", "compact-check");
    const amendInput = element("input");
    amendInput.type = "checkbox";
    amendInput.checked = gitState.amend;
    amendInput.disabled = busy || !(st.commits?.length);
    amendInput.addEventListener("change", () => {
      gitState.amend = amendInput.checked;
      syncButtons();
    });
    amendLabel.append(amendInput, element("span", t("修正上次提交")));

    const actions = element("div", "", "git-commit-actions");
    const aiBtn = element("button", "", "git-ai-btn");
    aiBtn.type = "button";
    aiBtn.title = t("使用 AI 生成提交信息");
    aiBtn.setAttribute("aria-label", t("使用 AI 生成提交信息"));
    const aiIcon = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    aiIcon.setAttribute("width", "14");
    aiIcon.setAttribute("height", "14");
    aiIcon.setAttribute("viewBox", "0 0 16 16");
    aiIcon.setAttribute("fill", "none");
    aiIcon.setAttribute("stroke", "currentColor");
    aiIcon.setAttribute("stroke-width", "1.45");
    aiIcon.setAttribute("stroke-linecap", "round");
    aiIcon.setAttribute("stroke-linejoin", "round");
    aiIcon.setAttribute("aria-hidden", "true");
    const bolt = document.createElementNS("http://www.w3.org/2000/svg", "path");
    bolt.setAttribute("d", "M9 1.75 3.75 9H8l-1 5.25L12.25 7H8l1-5.25Z");
    aiIcon.append(bolt);
    aiBtn.append(aiIcon, element("span", t(gitState.generatingMessage ? "生成中…" : "AI 生成")));
    aiBtn.addEventListener("click", generateGitCommitMessage);

    const commitBtn = element("button", t("提交"), "primary");
    commitBtn.type = "submit";
    syncButtons();
    actions.append(aiBtn, commitBtn);
    bar.append(amendLabel, actions);
    form.append(textarea, bar);
    if (!st.staged?.length && !gitState.amend && totalChanges > 0) {
      form.append(element("p", t("暂存更改后再提交。"), "session-hint"));
    }
    form.addEventListener("submit", event => {
      event.preventDefault();
      if (commitBtn.disabled) return;
      runGitAction({action: "commit", message: gitState.commitMessage, amend: gitState.amend});
    });
    body.append(form);

    if (totalChanges === 0) {
      body.append(element("p", t("工作区干净，没有未提交更改。"), "git-empty"));
    } else {
      if (st.truncated) body.append(element("p", t("更改文件较多，仅显示部分条目。"), "session-hint"));
      if (st.conflicted?.length) body.append(renderGitFileGroup(t("合并冲突"), st.conflicted, "conflicted"));
      if (st.staged?.length) body.append(renderGitFileGroup(t("已暂存"), st.staged, "staged"));
      if (st.unstaged?.length) body.append(renderGitFileGroup(t("更改"), st.unstaged, "unstaged"));
      if (st.untracked?.length) body.append(renderGitFileGroup(t("未跟踪"), st.untracked, "untracked"));
    }
    if (gitState.selectedDiff && gitState.selectedDiff.scope !== "commit") {
      body.append(renderGitDiffCard());
    }
  } else if (gitState.tab === "branches") {
    const createCard = element("form", "", "git-card");
    createCard.append(element("span", t("新建分支"), "session-hint"));
    const row = element("div", "", "git-branch-form");
    const input = element("input");
    input.placeholder = t("输入新分支名称");
    input.value = gitState.newBranch;
    input.disabled = gitState.pending;
    input.spellcheck = false;
    const createBtn = element("button", t("创建并切换"), "secondary");
    createBtn.type = "submit";
    createBtn.disabled = gitState.pending || !gitState.newBranch.trim();
    input.addEventListener("input", () => {
      gitState.newBranch = input.value;
      createBtn.disabled = gitState.pending || !gitState.newBranch.trim();
    });
    createCard.addEventListener("submit", event => {
      event.preventDefault();
      if (createBtn.disabled) return;
      runGitAction({action: "create_branch", name: gitState.newBranch.trim(), checkout: true});
    });
    row.append(input, createBtn);
    createCard.append(row);
    body.append(createCard);

    if (!st.branches?.length) {
      body.append(element("p", t("暂无分支。"), "git-empty"));
    } else {
      for (const branch of st.branches) {
        const card = element("div", "", "git-item-card" + (branch.current ? " is-current" : ""));
        const top = element("div", "", "git-item-top");
        const nameWrap = element("div", "", "git-item-name");
        nameWrap.append(element("span", branch.name));
        if (branch.commit_short) nameWrap.append(element("span", branch.commit_short, "git-hash"));
        top.append(nameWrap);
        if (branch.current) {
          top.append(element("span", t("当前分支"), "git-branch-pill"));
        } else {
          const sw = element("button", t("切换"), "git-mini-btn");
          sw.type = "button";
          sw.disabled = gitState.pending;
          sw.addEventListener("click", () => runGitAction({action: "checkout", branch: branch.name}));
          top.append(sw);
        }
        card.append(top);
        if (branch.upstream || branch.subject) {
          const meta = element("div", "", "git-item-meta");
          if (branch.upstream) {
            let up = branch.upstream;
            if (branch.ahead > 0) up += ` ↑${branch.ahead}`;
            if (branch.behind > 0) up += ` ↓${branch.behind}`;
            meta.append(element("span", up, "git-hash"));
          }
          if (branch.subject) meta.append(element("span", branch.subject));
          card.append(meta);
        }
        body.append(card);
      }
    }
  } else if (gitState.tab === "history") {
    if (!st.commits?.length) {
      body.append(element("p", t("暂无提交记录。"), "git-empty"));
    } else {
      for (const commit of st.commits) {
        const selected = gitState.selectedDiff?.scope === "commit" && gitState.selectedDiff?.commit === commit.hash;
        const card = element("div", "", "git-item-card" + (selected ? " is-selected" : ""));
        const btn = element("button", "", "git-commit-btn");
        btn.type = "button";
        btn.append(element("span", commit.subject || commit.short_hash, "git-commit-subject"));
        const meta = element("div", "", "git-item-meta");
        meta.append(element("span", commit.short_hash, "git-hash"), element("span", commit.author), element("span", commit.relative_date));
        btn.append(meta);
        btn.addEventListener("click", () => loadGitDiff({scope: "commit", commit: commit.hash}));
        card.append(btn);
        body.append(card);
      }
    }
    if (gitState.selectedDiff?.scope === "commit") {
      body.append(renderGitDiffCard());
    }
  }
}

$("git-toggle").addEventListener("click", () => {
  if (gitState.open) closeGitDrawer();
  else openGitDrawer();
});
$("git-close").addEventListener("click", closeGitDrawer);
$("git-backdrop").addEventListener("click", closeGitDrawer);
$("git-refresh").addEventListener("click", refreshGitStatus);
$("git-fetch").addEventListener("click", () => runGitAction({action: "fetch"}));
$("git-pull").addEventListener("click", () => runGitAction({action: "pull"}));
$("git-push").addEventListener("click", () => runGitAction({action: "push"}));
for (const tabBtn of $("git-tabs").querySelectorAll(".git-tab")) {
  tabBtn.addEventListener("click", () => {
    gitState.tab = tabBtn.dataset.tab;
    renderGit();
  });
}

applyLanguage(defaultLanguage);
initializeLanguage();
initialize();
