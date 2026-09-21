(() => {
  const doc = document.documentElement;
  const body = document.body;
  doc.classList.add("js");
  const release = new URL(document.currentScript.src).search;

  const store = (area, key, fallback) => {
    try {
      return JSON.parse(area.getItem(key)) ?? fallback;
    } catch {
      return fallback;
    }
  };
  const save = (area, key, value) => {
    try {
      area.setItem(key, JSON.stringify(value));
    } catch {
      /* private mode */
    }
  };

  const folds = store(localStorage, "hldr.fold", {});
  document.querySelectorAll("details[data-fold]").forEach((d) => {
    if (d.dataset.fold in folds) d.open = folds[d.dataset.fold];
    d.addEventListener("toggle", () => {
      folds[d.dataset.fold] = d.open;
      save(localStorage, "hldr.fold", folds);
    });
  });

  const toggle = document.getElementById("tt");
  const wide = matchMedia("(min-width: 761px)");
  if (wide.matches && store(localStorage, "hldr.notree", false)) toggle.checked = true;
  toggle.addEventListener("change", () => {
    if (wide.matches) save(localStorage, "hldr.notree", toggle.checked);
  });

  const here = body.dataset.path;
  const bar = document.getElementById("bufs");
  const label = bar.querySelector("a").textContent;
  const tabs = store(sessionStorage, "hldr.bufs", []).filter((t) => t.h !== here);
  if (body.dataset.ft) tabs.push({ h: here, l: label });
  save(sessionStorage, "hldr.bufs", tabs);

  const draw = () => {
    bar.replaceChildren(
      ...tabs.map((t) => {
        const tab = document.createElement("span");
        tab.className = "tab" + (t.h === here ? " on" : "");
        const a = document.createElement("a");
        a.href = t.h;
        a.textContent = t.l;
        const x = document.createElement("button");
        x.type = "button";
        x.textContent = "×";
        x.setAttribute("aria-label", "close " + t.l);
        x.dataset.close = t.h;
        tab.append(a, x);
        return tab;
      }),
    );
    bar.querySelector(".on")?.scrollIntoView({ block: "nearest", inline: "nearest" });
  };
  draw();

  let loading;
  const vim = () => {
    save(sessionStorage, "hldr.vim", true);
    loading ??= new Promise((done) => {
      const s = document.createElement("script");
      s.src = "/vim.js" + release;
      s.onload = () => done(window.hldrVim);
      document.head.append(s);
    });
    return loading;
  };

  const close = (href) => {
    const i = tabs.findIndex((t) => t.h === href);
    if (i < 0) return;
    tabs.splice(i, 1);
    save(sessionStorage, "hldr.bufs", tabs);
    if (href !== here) return draw();
    const next = tabs[Math.min(i, tabs.length - 1)];
    if (next) location.href = next.h;
    else vim().then((v) => v.intro());
  };

  document.addEventListener("click", (e) => {
    const x = e.target.closest("[data-close]");
    if (x) return close(x.dataset.close);
    const c = e.target.closest("[data-cmd=find], [data-cmd=checkhealth]");
    if (c) {
      e.preventDefault();
      vim().then((v) => v.run(c.dataset.cmd));
    }
  });

  const TRIGGER = new Set([":", "/", "?", "j", "k", "g", "G", "n", "N", "{", "}", "*", "F1"]);
  const onKey = (e) => {
    if (window.hldrVim) return;
    const el = document.activeElement;
    if (e.metaKey || e.altKey || el?.matches("input:not(.tt), textarea, [contenteditable]")) return;
    const ctrl = e.ctrlKey && "dufbhl".includes(e.key);
    if (!ctrl && (e.ctrlKey || !TRIGGER.has(e.key))) return;
    e.preventDefault();
    vim().then((v) => v.key(e));
  };
  document.addEventListener("keydown", onKey);

  if (store(sessionStorage, "hldr.vim", false)) vim();
  window.hldrTabs = { list: () => tabs, close, vim };
})();
