// Run before styles load so a saved dark theme never flashes a light page.
(() => {
  const key = "narolink.appearance";
  const root = document.documentElement;
  const system = matchMedia("(prefers-color-scheme: dark)");
  const normalize = value => ["light", "dark"].includes(value) ? value : "system";
  let preference = "system";
  try { preference = normalize(localStorage.getItem(key)); } catch {}

  function apply() {
    const theme = preference === "system" ? (system.matches ? "dark" : "light") : preference;
    root.dataset.appearance = preference;
    root.dataset.theme = theme;
    document.querySelector('meta[name="theme-color"]').content = theme === "dark" ? "#171b22" : "#ffffff";
    window.dispatchEvent(new Event("naro-appearance-change"));
  }

  window.naroAppearance = {
    get preference() { return preference; },
    set(value) {
      preference = normalize(value);
      try { localStorage.setItem(key, preference); } catch {}
      apply();
    },
  };
  system.addEventListener("change", () => { if (preference === "system") apply(); });
  window.addEventListener("storage", event => {
    if (event.key === key || event.key === null) {
      preference = normalize(event.newValue);
      apply();
    }
  });
  apply();
})();
