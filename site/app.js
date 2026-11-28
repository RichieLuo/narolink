(function () {
  "use strict";

  const I18N = window.NARO_I18N;
  const SNIPPETS = {
    go: `go build -trimpath -ldflags="-s -w" -o naro-relay .
export NARO_PUBLIC_URL=https://naro.example.com
export NARO_RELAY_TOKEN="$(openssl rand -hex 32)"
./naro-relay`,

    docker: `# Docker Compose (scratch image, read-only, non-root)
export NARO_PUBLIC_URL=https://naro.example.com
export NARO_RELAY_TOKEN="$(openssl rand -hex 32)"
docker compose up -d --build

# Or run directly with Docker
docker build -t naro-relay .
docker run -d --name naro-relay --restart unless-stopped \\
  --read-only --cap-drop ALL --security-opt no-new-privileges \\
  -p 127.0.0.1:7319:7319 \\
  -e NARO_PUBLIC_URL=https://naro.example.com \\
  -e NARO_RELAY_TOKEN="$NARO_RELAY_TOKEN" \\
  naro-relay`,

    build: `# macOS (Rust 1.89+ & Go 1.23+)
sh scripts/package-macos.sh
go build -o dist/naro-relay .

# Windows (Visual Studio Developer PowerShell)
cargo build --release --locked
go build -o dist/naro-relay.exe .`
  };

  const urlParams = new URLSearchParams(window.location.search);
  const queryTheme = urlParams.get("theme");
  const queryLang = urlParams.get("lang");

  const state = {
    lang: (queryLang && I18N[queryLang]) ? queryLang : (localStorage.getItem("naro-site-lang") || detectLang()),
    themePref: (queryTheme === "dark" || queryTheme === "light" || queryTheme === "system")
      ? queryTheme
      : (localStorage.getItem("naro-site-theme") || "system"),
    engine: "codex",
    view: "chat",
    snippet: "go",
    handoffState: null, // null | 'approved' | 'steered'
    sim: "stream"
  };

  function detectLang() {
    const nav = (navigator.language || "zh-CN").toLowerCase();
    if (nav.startsWith("ja")) return "ja";
    if (nav.startsWith("zh")) return "zh-CN";
    return "en";
  }

  function t(key) {
    const dict = I18N[state.lang] || I18N["zh-CN"];
    return dict[key] !== undefined ? dict[key] : I18N["zh-CN"][key];
  }

  /* -----------------------------------------------------------------------
     Theme Management (Light / Dark / System — mirrors Naro App)
     ----------------------------------------------------------------------- */
  const mediaDark = window.matchMedia("(prefers-color-scheme: dark)");
  const themeIcons = {
    light:
      '<circle cx="12" cy="12" r="4"></circle><path d="M12 2v2"></path><path d="M12 20v2"></path><path d="m4.93 4.93 1.41 1.41"></path><path d="m17.66 17.66 1.41 1.41"></path><path d="M2 12h2"></path><path d="M20 12h2"></path><path d="m6.34 17.66-1.41 1.41"></path><path d="m19.07 4.93-1.41 1.41"></path>',
    dark: '<path d="M12 3a6 6 0 0 0 9 9 9 9 0 1 1-9-9Z"></path>',
    system:
      '<rect x="2" y="3" width="20" height="14" rx="2"></rect><line x1="8" y1="21" x2="16" y2="21"></line><line x1="12" y1="17" x2="12" y2="21"></line>'
  };

  function applyTheme(pref) {
    state.themePref = pref;
    localStorage.setItem("naro-site-theme", pref);
    const resolvedDark = pref === "dark" || (pref === "system" && mediaDark.matches);
    document.documentElement.dataset.theme = resolvedDark ? "dark" : "light";
    document.documentElement.dataset.themePref = pref;

    const icon = document.getElementById("theme-icon");
    const toggle = document.getElementById("theme-toggle");
    if (icon) icon.innerHTML = themeIcons[pref] || themeIcons.system;
    if (toggle) {
      const labels = { light: "Light", dark: "Dark", system: "System" };
      toggle.title = "Theme: " + labels[pref];
      toggle.setAttribute("aria-label", "Theme: " + labels[pref]);
    }
  }

  mediaDark.addEventListener("change", () => {
    if (state.themePref === "system") applyTheme("system");
  });

  /* -----------------------------------------------------------------------
     Language Application
     ----------------------------------------------------------------------- */
  function applyLanguage(lang) {
    if (!I18N[lang]) lang = "zh-CN";
    state.lang = lang;
    localStorage.setItem("naro-site-lang", lang);
    document.documentElement.lang = lang;
    document.title = t("pageTitle");

    document.querySelectorAll(".lang-btn").forEach((btn) => {
      btn.setAttribute("aria-pressed", String(btn.dataset.lang === lang));
    });

    document.querySelectorAll("[data-i18n]").forEach((el) => {
      const key = el.dataset.i18n;
      const val = t(key);
      if (typeof val === "string") el.textContent = val;
    });

    document.querySelectorAll("[data-tags]").forEach((container) => {
      const items = t(container.dataset.tags);
      if (Array.isArray(items)) {
        container.innerHTML = items.map((tag) => `<span class="engine-tag">${tag}</span>`).join("");
      }
    });

    document.querySelectorAll("[data-pills]").forEach((container) => {
      const items = t(container.dataset.pills);
      if (Array.isArray(items)) {
        container.innerHTML = items.map((pill) => `<span class="spec-pill">${pill}</span>`).join("");
      }
    });

    renderEnginePreview();
    renderHandoffFeedback();
    updateRelayLogText(state.sim);
  }

  /* -----------------------------------------------------------------------
     Hero Interactive Handoff Studio
     ----------------------------------------------------------------------- */
  function renderEnginePreview() {
    const dict = I18N[state.lang] || I18N["zh-CN"];
    const cfg = dict.engines[state.engine];
    if (!cfg) return;

    document.querySelectorAll(".engine-pill").forEach((btn) => {
      btn.setAttribute("aria-pressed", String(btn.dataset.engine === state.engine));
    });

    document.getElementById("mock-session-title").textContent = cfg.sessionTitle;
    document.getElementById("mock-cwd").textContent = cfg.cwd;
    document.getElementById("mock-model-chip").textContent = cfg.model;
    document.getElementById("mock-perm-chip").textContent = cfg.perm;
    const fastChip = document.getElementById("mock-fast-chip");
    fastChip.hidden = !cfg.fast;

    document.getElementById("mock-user-msg").textContent = cfg.userMsg;
    document.getElementById("mock-assistant-msg").innerHTML = cfg.assistantHtml;
    document.getElementById("phone-summary").textContent = cfg.phoneSummary;
    document.getElementById("phone-cmd").textContent = cfg.phoneCmd;
  }

  function renderHandoffFeedback() {
    const banner = document.getElementById("mock-handoff-banner");
    const bannerText = document.getElementById("mock-handoff-text");
    const toastText = document.getElementById("phone-toast-text");

    if (state.handoffState === "approved") {
      banner.hidden = false;
      bannerText.textContent = t("desktopApprovedNotice");
      toastText.textContent = t("phoneToastApproved");
    } else if (state.handoffState === "steered") {
      banner.hidden = false;
      bannerText.textContent = t("desktopSteerNotice");
      toastText.textContent = t("phoneToastSteered");
    } else {
      banner.hidden = true;
      toastText.textContent = t("phoneToastIdle");
    }
  }

  function triggerWireBurst(kind) {
    state.handoffState = kind;
    switchWorkspaceView("chat");
    renderHandoffFeedback();

    const bridge = document.getElementById("wire-bridge");
    bridge.classList.remove("is-burst");
    void bridge.offsetWidth;
    bridge.classList.add("is-burst");
    setTimeout(() => bridge.classList.remove("is-burst"), 1200);
  }

  function switchWorkspaceView(view) {
    state.view = view;
    document.querySelectorAll(".window-tab").forEach((tab) => {
      tab.setAttribute("aria-selected", String(tab.dataset.view === view));
    });
    document.querySelectorAll("[data-view-panel]").forEach((panel) => {
      panel.hidden = panel.dataset.viewPanel !== view;
    });
  }

  /* -----------------------------------------------------------------------
     Relay Interactive Packet Simulator
     ----------------------------------------------------------------------- */
  let simTimer = null;

  function pulseNode(id, delayMs) {
    setTimeout(() => {
      const el = document.getElementById(id);
      if (!el) return;
      el.classList.add("is-highlight");
      setTimeout(() => el.classList.remove("is-highlight"), 680);
    }, delayMs);
  }

  function firePacket(id, dirClass, delayMs) {
    setTimeout(() => {
      const el = document.getElementById(id);
      if (!el) return;
      el.classList.remove("animate-right", "animate-left");
      void el.offsetWidth;
      el.classList.add(dirClass);
    }, delayMs);
  }

  function updateRelayLogText(sim) {
    const keyMap = {
      stream: "simLogStream",
      approval: "simLogApproval",
      stop: "simLogStop",
      heartbeat: "simLogHeartbeat"
    };
    const logEl = document.getElementById("relay-log-text");
    if (logEl) logEl.textContent = t(keyMap[sim] || "simLogDefault");
  }

  function runRelaySimulation(sim) {
    state.sim = sim;
    clearTimeout(simTimer);

    document.querySelectorAll(".sim-btn").forEach((btn) => {
      btn.classList.toggle("is-active", btn.dataset.sim === sim);
    });
    updateRelayLogText(sim);

    if (sim === "heartbeat") {
      pulseNode("node-host", 0);
      firePacket("pkt-right-back", "animate-left", 80);
      pulseNode("node-relay", 520);
      firePacket("pkt-right-fwd", "animate-right", 650);
      pulseNode("node-host", 1150);
      return;
    }

    // Client -> Relay -> Host -> Relay -> Client
    pulseNode("node-client", 0);
    firePacket("pkt-left-fwd", "animate-right", 60);
    pulseNode("node-relay", 460);
    firePacket("pkt-right-fwd", "animate-right", 560);
    pulseNode("node-host", 960);

    // Response / SSE return
    firePacket("pkt-right-back", "animate-left", 1120);
    pulseNode("node-relay", 1520);
    firePacket("pkt-left-back", "animate-left", 1620);
    pulseNode("node-client", 2020);
  }

  /* -----------------------------------------------------------------------
     Deploy Code Tabs & Copy
     ----------------------------------------------------------------------- */
  function renderSnippet(key) {
    state.snippet = key;
    document.querySelectorAll(".code-tab").forEach((tab) => {
      tab.setAttribute("aria-selected", String(tab.dataset.snippet === key));
    });
    const codeEl = document.getElementById("deploy-code");
    if (codeEl) codeEl.textContent = SNIPPETS[key] || SNIPPETS.go;
  }

  async function copyCurrentSnippet() {
    const text = SNIPPETS[state.snippet] || "";
    const labelEl = document.getElementById("copy-code-label");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      document.execCommand("copy");
      ta.remove();
    }
    if (labelEl) {
      labelEl.textContent = t("copiedLabel");
      setTimeout(() => {
        labelEl.textContent = t("copyLabel");
      }, 1500);
    }
  }

  /* -----------------------------------------------------------------------
     Scroll Reveal Observer
     ----------------------------------------------------------------------- */
  function setupReveal() {
    const els = document.querySelectorAll(".reveal");
    if (!("IntersectionObserver" in window)) return;
    const vh = window.innerHeight || 900;
    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            entry.target.classList.remove("is-pending");
            entry.target.classList.add("is-visible");
            io.unobserve(entry.target);
          }
        });
      },
      { threshold: 0.05, rootMargin: "0px 0px -20px 0px" }
    );
    els.forEach((el) => {
      const rect = el.getBoundingClientRect();
      if (rect.top > vh * 0.96) {
        el.classList.add("is-pending");
        io.observe(el);
      }
    });
  }

  /* -----------------------------------------------------------------------
     Bind Events & Initialize
     ----------------------------------------------------------------------- */
  document.getElementById("theme-toggle").addEventListener("click", () => {
    const order = ["system", "light", "dark"];
    const next = order[(order.indexOf(state.themePref) + 1) % order.length];
    applyTheme(next);
  });

  document.querySelectorAll(".lang-btn").forEach((btn) => {
    btn.addEventListener("click", () => applyLanguage(btn.dataset.lang));
  });

  document.querySelectorAll(".engine-pill").forEach((btn) => {
    btn.addEventListener("click", () => {
      state.engine = btn.dataset.engine;
      state.handoffState = null;
      renderEnginePreview();
      renderHandoffFeedback();
    });
  });

  document.querySelectorAll(".window-tab").forEach((tab) => {
    tab.addEventListener("click", () => switchWorkspaceView(tab.dataset.view));
  });

  document.getElementById("btn-phone-approve").addEventListener("click", () => {
    triggerWireBurst("approved");
  });

  document.getElementById("btn-phone-steer").addEventListener("click", () => {
    triggerWireBurst("steered");
  });

  document.querySelectorAll(".sim-btn").forEach((btn) => {
    btn.addEventListener("click", () => runRelaySimulation(btn.dataset.sim));
  });

  document.querySelectorAll(".code-tab").forEach((tab) => {
    tab.addEventListener("click", () => renderSnippet(tab.dataset.snippet));
  });

  document.getElementById("copy-code-btn").addEventListener("click", copyCurrentSnippet);

  // Boot
  applyTheme(state.themePref);
  applyLanguage(state.lang);
  renderSnippet("go");
  setupReveal();
})();
