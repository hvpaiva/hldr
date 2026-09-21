// The vim layer. Loaded by keys.js on the first vim key; every page works
// without it. Buffers are pages, so opening a file is navigating to it and
// session state (tabs, history, options, the pending message) lives in
// sessionStorage.
(() => {
  const $ = (s, r = document) => r.querySelector(s);
  const $$ = (s, r = document) => [...r.querySelectorAll(s)];
  const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const ss = {
    get: (k, d) => { try { return JSON.parse(sessionStorage.getItem("hldr." + k)) ?? d; } catch { return d; } },
    set: (k, v) => { try { sessionStorage.setItem("hldr." + k, JSON.stringify(v)); } catch { /* private mode */ } },
  };
  const ed = $("#ed"), win = $("#win"), tree = $("#tree"), toggle = $("#tt");
  let buf = $(".buf", win);
  const here = document.body.dataset.path;
  const version = ($(".status .ver")?.textContent.match(/hldr ([\w.-]+)/) || [])[1] || "";
  const mobile = () => matchMedia("(max-width: 760px)").matches;
  const BANNER = "ooooo ooooo ooooo\n 888   888   888\n 888   888   888\n 888ooo888   888\n 888   888   888     o\no888o o888o o888oooo88\n\nooooooooo  oooooooooo\n 888    88o 888    888\n 888    888 888oooo88\n 888    888 888  88o\no888ooo88  o888o  88o8";

  // files: everything the tree links to, plus the pages outside it
  const FILES = new Map([["README.md", "/"], ["[colorscheme]", "/theme"], ["help.txt", "/help"]]);
  $$("a[data-file]", tree).forEach((a) => FILES.set(a.dataset.file, a.getAttribute("href")));
  FILES.set("projects/", "/projects");
  const paths = () => [...FILES.keys()].filter((p) => p !== "[colorscheme]");
  const byPath = (arg) => {
    const a = arg.replace(/^\.?\//, "").toLowerCase();
    if (!a) return null;
    const ps = [...FILES.keys()];
    return ps.find((p) => p.toLowerCase() === a) || ps.find((p) => p.toLowerCase().startsWith(a)) || ps.find((p) => p.toLowerCase().includes(a));
  };
  const go = (href, note) => { if (note) ss.set("msg", note); location.href = href; };

  // cmdline, pager, finder
  ed.insertAdjacentHTML("beforeend",
    '<div class="cmd" id="cmd"><span id="lead"></span><span class="cwrap" id="cwrap" hidden><span class="ghost" id="ghost" aria-hidden="true"></span>' +
    '<input id="cin" aria-label="command line" aria-controls="pum" autocomplete="off" spellcheck="false"></span><span class="out" id="cout"></span>' +
    '<ul class="pum" id="pum" role="listbox" aria-label="completions"></ul></div>');
  win.insertAdjacentHTML("beforeend",
    '<div class="more" id="more" role="log" aria-live="polite"><pre id="more-out"></pre><p>Press ENTER or type command to continue</p></div>' +
    '<div class="scrim" id="scrim"></div><div class="float" id="finder" role="dialog" aria-label="find file">' +
    '<header><span>find files</span><button type="button" data-shut aria-label="close">esc ✕</button></header>' +
    '<label><b>&gt;</b><input id="fq" autocomplete="off" spellcheck="false" aria-label="file name"></label><ul id="fl" role="listbox"></ul></div>');
  const cmdEl = $("#cmd"), cin = $("#cin"), cwrap = $("#cwrap"), cout = $("#cout"), lead = $("#lead"), pum = $("#pum"), ghostEl = $("#ghost");

  // options survive navigation
  const opts = ss.get("set", {});
  const applyOpts = () => ["nonu", "rnu", "nowrap", "ch1"].forEach((o) => ed.classList.toggle(o, !!opts[o]));
  applyOpts();

  // messages
  const log = ss.get("log", []);
  let msgTimer = 0;
  const msg = (text, kind = "") => {
    log.push(text); ss.set("log", log.slice(-40));
    clearTimeout(msgTimer);
    cwrap.hidden = true; lead.textContent = "";
    cout.textContent = text; cout.className = "out" + (kind ? " " + kind : "");
    cmdEl.classList.add("on");
    if (!opts.ch1) msgTimer = setTimeout(() => { if (cwrap.hidden) { cmdEl.classList.remove("on"); cout.textContent = ""; } }, 4000);
  };
  const more = (text) => { $("#more-out").textContent = text; $("#more").classList.add("on"); };
  const closeMore = () => $("#more").classList.remove("on");

  // cursor
  const lines = () => [...buf.children];
  let line = 0;
  const mark = (n, scroll = true) => {
    const ls = lines();
    if (!ls.length) return;
    line = Math.max(0, Math.min(n, ls.length - 1));
    ls.forEach((l, i) => l.classList.toggle("cl", i === line));
    if (opts.rnu) ls.forEach((l, i) => (l.dataset.n = i === line ? i + 1 : Math.abs(i - line)));
    const pct = ls.length <= 1 ? "All" : line === 0 ? "Top" : line === ls.length - 1 ? "Bot" : Math.round((line / (ls.length - 1)) * 100) + "%";
    $("#pos").textContent = `${line + 1}:1  ${pct}`;
    if (scroll) {
      const r = ls[line], top = r.offsetTop, h = win.clientHeight;
      if (top < win.scrollTop + 24 || top + r.offsetHeight > win.scrollTop + h - 48) win.scrollTop = top - h / 3;
    }
    const t = ls[line].dataset.theme;
    if (t) preview(ls[line]);
  };
  const center = () => { const r = lines()[line]; win.scrollTop = r.offsetTop - win.clientHeight / 2 + r.offsetHeight / 2; };
  const para = (d) => { const ls = lines(); let i = line + d; while (i > 0 && i < ls.length - 1 && ls[i].textContent.trim() !== "") i += d; mark(i); };
  const half = () => Math.max(3, Math.floor(win.clientHeight / 24 / 2));
  const linkOn = (i = line) => lines()[i]?.querySelector("a");

  // colorscheme: the line carries its palette as --t-* properties
  const VARS = ["bg", "fg", "fg-dim", "fg-muted", "accent", "accent2", "highlight", "special", "teal", "error", "border"];
  const preview = (el) => {
    const src = el.style, root = document.documentElement.style;
    VARS.forEach((v) => root.setProperty("--" + v, src.getPropertyValue("--t-" + v)));
    root.setProperty("--accent-dim", src.getPropertyValue("--t-accent").trim() + "18");
    root.setProperty("color-scheme", src.getPropertyValue("--t-scheme"));
  };

  const follow = (a, newTab) => {
    if (!a) { msg("E434: Can't find tag pattern: no link on this line", "e"); return; }
    if (newTab) { window.open(a.href, "_blank", "noopener"); return; }
    a.click();
  };

  // search
  let search = ss.get("search", { pat: "", dir: 1 });
  const clearSearch = () => { $$("mark", buf).forEach((m) => m.replaceWith(...m.childNodes)); buf.normalize(); };
  const highlight = (pat) => {
    clearSearch();
    if (!pat) return [];
    const exact = /[A-Z]/.test(pat), needle = exact ? pat : pat.toLowerCase(), hits = [];
    lines().forEach((l, i) => {
      const walker = document.createTreeWalker(l, NodeFilter.SHOW_TEXT), nodes = [];
      while (walker.nextNode()) nodes.push(walker.currentNode);
      nodes.forEach((node) => {
        const text = node.nodeValue, hay = exact ? text : text.toLowerCase();
        let j = hay.indexOf(needle), last = 0;
        if (j < 0) return;
        const frag = document.createDocumentFragment();
        while (j >= 0) {
          frag.append(text.slice(last, j));
          const m = document.createElement("mark");
          m.textContent = text.slice(j, j + needle.length);
          frag.append(m);
          last = j + needle.length;
          j = hay.indexOf(needle, last);
        }
        frag.append(text.slice(last));
        node.replaceWith(frag);
        if (!hits.includes(i)) hits.push(i);
      });
    });
    return hits;
  };
  const jump = (dir) => {
    if (!search.pat) { msg("E35: No previous regular expression", "e"); return; }
    const hits = highlight(search.pat);
    if (!hits.length) { msg(`E486: Pattern not found: ${search.pat}`, "e"); return; }
    const d = dir * search.dir;
    let next = d > 0 ? hits.find((h) => h > line) : [...hits].reverse().find((h) => h < line), wrapped = false;
    if (next === undefined) { next = d > 0 ? hits[0] : hits[hits.length - 1]; wrapped = true; }
    mark(next);
    $$("mark", lines()[next]).forEach((m) => m.classList.add("now"));
    if (wrapped) msg(d > 0 ? "search hit BOTTOM, continuing at TOP" : "search hit TOP, continuing at BOTTOM", "w");
    else msg(`${search.dir > 0 ? "/" : "?"}${search.pat}   [${hits.indexOf(next) + 1}/${hits.length}]`);
  };

  // modes
  let treeFocus = false;
  const setMode = (m) => {
    const el = $("#mode");
    el.textContent = { normal: "NORMAL", cmd: "COMMAND", search: "SEARCH", tree: "NEO-TREE" }[m];
    el.className = "mode" + (m === "cmd" || m === "search" ? " m-cmd" : m === "tree" ? " m-tree" : "");
  };

  // command line + completion
  const hist = ss.get("hist", []);
  let histPos = hist.length, lineBefore = 0;
  const prompt = (l) => {
    clearTimeout(msgTimer); closeMore();
    lead.textContent = l; cout.textContent = "";
    cwrap.hidden = false; cin.value = ""; cmdEl.classList.add("on");
    lineBefore = line; setMode(l === ":" ? "cmd" : "search");
    cin.focus(); complete();
  };
  const endPrompt = () => {
    closeComp(); cwrap.hidden = true; cin.blur(); lead.textContent = "";
    if (!opts.ch1 && !cout.textContent) cmdEl.classList.remove("on");
    setMode(treeFocus ? "tree" : "normal");
    if (!treeFocus) win.focus({ preventScroll: true });
  };

  const fuzzy = (n, h) => {
    let i = 0, sc = 0;
    const hits = [], hay = h.toLowerCase();
    for (const ch of n.toLowerCase()) {
      const j = hay.indexOf(ch, i);
      if (j < 0) return null;
      sc += j === i ? 2 : 1; hits.push(j); i = j + 1;
    }
    if (hay.startsWith(n.toLowerCase())) sc += 100;
    return { sc, hits };
  };
  const THEMES = $$("[data-theme]", buf).map((l) => l.dataset.theme);
  const CMDS = [
    ["edit", "open a file"], ["find", "find a file"], ["colorscheme", "palette page, or set one"], ["checkhealth", "is the site up?"],
    ["help", "the manual"], ["buffers", "list open buffers"], ["ls", "list open buffers"], ["bnext", "next buffer"], ["bprevious", "previous buffer"],
    ["bdelete", "close this buffer"], ["messages", "message history"], ["nohlsearch", "clear search highlight"], ["set", "options"],
    ["version", "build info"], ["intro", "start screen"], ["Neotree", "toggle the tree"], ["NvimTreeToggle", "toggle the tree"],
    ["Telescope", "find a file"], ["smile", ""], ["quit", "leave"], ["write", "save"], ["wq", "save and leave"],
  ];
  const SETS = ["number", "nonumber", "relativenumber", "norelativenumber", "wrap", "nowrap", "cmdheight=0", "cmdheight=1"];
  const HELP = ["hldr-move", "hldr-files", "hldr-search", "hldr-commands"];
  const argPool = (c) => {
    if (/^(e|edit|find|fin|b|buffer|sp|vs|split|vsplit)$/.test(c)) return paths().map((p) => [p, ""]);
    if (/^colo(rscheme)?$/.test(c)) return (THEMES.length ? THEMES : ss.get("themes", [])).map((t) => [t, ""]);
    if (/^h(elp)?$/.test(c)) return HELP.map((t) => [t, ""]);
    if (/^(set|se)$/.test(c)) return SETS.map((o) => [o, ""]);
    return [];
  };
  if (THEMES.length) ss.set("themes", THEMES);
  const comp = { items: [], sel: -1, prefix: "", token: "", sugg: "" };
  const renderPum = () => {
    pum.innerHTML = comp.items.map((x, i) => `<li data-i="${i}" role="option" class="${i === comp.sel ? "on" : ""}"><span>${[...x.n].map((c, k) => (x.m.hits.includes(k) ? `<b>${esc(c)}</b>` : esc(c))).join("")}</span>${x.d ? `<span class="d">${esc(x.d)}</span>` : ""}</li>`).join("");
    pum.classList.toggle("on", comp.items.length > 0);
    pum.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  };
  const renderGhost = () => {
    const v = cin.value;
    comp.sugg = "";
    if (v && cin.selectionStart === v.length && comp.sel < 0) {
      comp.sugg = [...hist].reverse().find((h) => h.startsWith(v) && h !== v) || "";
      const best = comp.items[0];
      if (!comp.sugg && best && best.n.toLowerCase().startsWith(comp.token.toLowerCase())) {
        const full = comp.prefix + best.n;
        if (full.startsWith(v) && full !== v) comp.sugg = full;
      }
    }
    ghostEl.innerHTML = comp.sugg ? `<span class="ty">${esc(v)}</span>${esc(comp.sugg.slice(v.length))}` : "";
  };
  const complete = () => {
    comp.sel = -1;
    if (lead.textContent !== ":") { comp.items = []; renderPum(); ghostEl.innerHTML = ""; return; }
    const v = cin.value, m = v.match(/^(\S*)(\s+)(.*)$/);
    const token = m ? m[3] : v, pool = m ? argPool(m[1]) : CMDS;
    comp.prefix = m ? m[1] + " " : ""; comp.token = token;
    let items = pool.map(([n, d]) => ({ n, d, m: token ? fuzzy(token, n) : { sc: 0, hits: [] } })).filter((x) => x.m);
    if (!m && !token) items = [];
    items.sort((a, b) => b.m.sc - a.m.sc);
    if (items.length === 1 && items[0].n === token) items = [];
    comp.items = items.slice(0, 60);
    renderPum(); renderGhost();
  };
  const pick = (i) => {
    if (!comp.items.length) return;
    comp.sel = (i + comp.items.length) % comp.items.length;
    cin.value = comp.prefix + comp.items[comp.sel].n;
    ghostEl.innerHTML = ""; comp.sugg = ""; renderPum();
  };
  const accept = (text) => {
    const x = comp.items.find((it) => comp.prefix + it.n === text);
    cin.value = text + (!comp.prefix && x && argPool(x.n).length ? " " : "");
    complete(); cin.focus();
  };
  const closeComp = () => { comp.items = []; comp.sel = -1; comp.sugg = ""; renderPum(); ghostEl.innerHTML = ""; };
  pum.addEventListener("mousedown", (e) => e.preventDefault());
  pum.addEventListener("click", (e) => { const li = e.target.closest("li[data-i]"); if (li) accept(comp.prefix + comp.items[+li.dataset.i].n); });
  cin.addEventListener("input", () => {
    if (lead.textContent !== ":") { const hits = highlight(cin.value); if (hits.length) mark(hits.find((h) => h >= lineBefore) ?? hits[0]); return; }
    complete();
  });
  cin.addEventListener("keyup", (e) => { if (e.key === "ArrowLeft" || e.key === "Home") renderGhost(); });
  cin.addEventListener("keydown", (e) => {
    e.stopPropagation();
    const l = lead.textContent;
    if (e.key === "Escape") { e.preventDefault(); if (l !== ":") { clearSearch(); mark(lineBefore); } cout.textContent = ""; endPrompt(); return; }
    if (e.key === "Backspace" && !cin.value) { e.preventDefault(); cout.textContent = ""; endPrompt(); return; }
    if (e.key === "Enter") {
      e.preventDefault();
      const v = cin.value;
      cout.textContent = ""; endPrompt();
      if (l === ":") { if (v.trim()) { hist.push(v); ss.set("hist", hist.slice(-100)); } histPos = hist.length; ex(v); }
      else if (v) { search = { pat: v, dir: l === "/" ? 1 : -1 }; ss.set("search", search); mark(lineBefore); jump(1); }
      else if (search.pat) { search.dir = l === "/" ? 1 : -1; jump(1); }
      return;
    }
    if (l !== ":") return;
    if (e.key === "ArrowUp" || e.key === "ArrowDown") {
      e.preventDefault();
      histPos = Math.max(0, Math.min(hist.length, histPos + (e.key === "ArrowUp" ? -1 : 1)));
      cin.value = hist[histPos] || ""; complete(); return;
    }
    if ((e.key === "ArrowRight" || e.key === "End") && comp.sugg && cin.selectionStart === cin.value.length) { e.preventDefault(); accept(comp.sugg); return; }
    if (e.ctrlKey && "njpk".includes(e.key)) { e.preventDefault(); pick(comp.sel + ("nj".includes(e.key) ? 1 : -1)); return; }
    if (e.ctrlKey && e.key === "e") { e.preventDefault(); closeComp(); return; }
    if (e.key === "Tab") {
      e.preventDefault();
      if (comp.items.length) pick(comp.sel + (e.shiftKey ? -1 : 1));
      else if (comp.sugg) accept(comp.sugg);
    }
  });

  // tabs (keys.js owns the list)
  const tabs = () => window.hldrTabs.list();
  const cycle = (d) => { const t = tabs(), i = t.findIndex((x) => x.h === here); if (t.length > 1) go(t[(i + d + t.length) % t.length].h); };

  // ex commands
  const setOpt = (o) => {
    const [k, v] = o.split("=");
    const flip = (name, on) => { opts[name] = on; ss.set("set", opts); applyOpts(); mark(line, false); };
    if (/^(nu|number)$/.test(k)) flip("nonu", false);
    else if (/^(nonu|nonumber)$/.test(k)) flip("nonu", true);
    else if (/^(nu|number)!$/.test(k)) flip("nonu", !opts.nonu);
    else if (/^(rnu|relativenumber)$/.test(k)) { opts.nonu = false; flip("rnu", true); }
    else if (/^(nornu|norelativenumber)$/.test(k)) flip("rnu", false);
    else if (/^(rnu|relativenumber)!$/.test(k)) flip("rnu", !opts.rnu);
    else if (k === "wrap") flip("nowrap", false);
    else if (k === "nowrap") flip("nowrap", true);
    else if (k === "wrap!") flip("nowrap", !opts.nowrap);
    else if (/^(ch|cmdheight)$/.test(k) && /^[01]$/.test(v)) { flip("ch1", v === "1"); if (v === "0") cmdEl.classList.remove("on"); }
    else if (/^(ro|readonly|noma|nomodifiable)$/.test(k)) msg("already. the source is git.");
    else if (/^(noro|noreadonly|ma|modifiable)$/.test(k)) msg("E21: Cannot make changes, 'modifiable' is off. Open a PR instead", "e");
    else msg(`E518: Unknown option: ${k}`, "e");
  };
  const ex = (raw) => {
    const s = raw.trim().replace(/^:+/, "");
    if (!s) return;
    if (/^\d+$/.test(s)) { mark(+s - 1); return; }
    if (s.startsWith("!")) { msg("E145: Shell commands and some functionality not allowed in rvim", "e"); return; }
    const [c, ...rest] = s.split(/\s+/), arg = rest.join(" ");
    const is = (...names) => names.includes(c);
    if (is("e!")) { location.reload(); return; }
    if (is("e", "edit", "find", "fin", "b", "buffer", "sp", "vs", "split", "vsplit", "tabe", "tabedit")) {
      if (!arg) { if (is("find", "fin")) finder(); else msg("E32: No file name", "e"); return; }
      if (/^\d+$/.test(arg) && is("b", "buffer")) { const t = tabs()[+arg - 1]; if (t) go(t.h); else msg(`E86: Buffer ${arg} does not exist`, "e"); return; }
      const p = byPath(arg);
      if (p) go(FILES.get(p), `"${p}" [readonly]`); else msg(`E447: Can't find file "${arg}" in path`, "e");
      return;
    }
    if (is("colo", "colorscheme")) {
      if (!arg) { go("/theme"); return; }
      const pool = argPool("colo").map(([n]) => n);
      const t = pool.find((x) => x === arg) || pool.find((x) => x.startsWith(arg));
      if (t) go(`/theme/${t}`, `colorscheme ${t}`); else msg(`E185: Cannot find color scheme '${arg}'`, "e");
      return;
    }
    if (is("h", "help")) {
      if (s === "help!" || s === "h!") { msg("E478: Don't panic!", "e"); return; }
      if (arg === "42") { more("What is the meaning of life, the universe and everything?\nDouglas Adams, the only person who knew what this question really was about is\nnow dead, unfortunately.  So now you might wonder what the meaning of death\nis..."); return; }
      if (!arg) { go("/help"); return; }
      const tag = HELP.find((t) => t.includes(arg));
      if (tag) go(`/help#${tag}`); else msg(`E149: Sorry, no help for ${arg}`, "e");
      return;
    }
    if (is("checkhealth", "che")) { health(); return; }
    if (is("bn", "bnext")) { cycle(1); return; }
    if (is("bp", "bprevious", "bN")) { cycle(-1); return; }
    if (is("bd", "bdelete", "bw", "bwipeout")) { window.hldrTabs.close(here); return; }
    if (is("ls", "buffers", "files")) { more(tabs().map((t, i) => `${String(i + 1).padStart(3)} ${t.h === here ? "%a" : "  "}  "${t.l}"`).join("\n")); return; }
    if (is("messages", "mes")) { more(log.slice(-20).join("\n") || "(no messages)"); return; }
    if (is("noh", "nohlsearch")) { clearSearch(); return; }
    if (is("set", "se")) {
      if (!arg) { more(`  ${opts.nonu ? "no" : "  "}number   ${opts.rnu ? "  " : "no"}relativenumber   ${opts.nowrap ? "no" : "  "}wrap   cmdheight=${opts.ch1 ? 1 : 0}   readonly`); return; }
      rest.forEach(setOpt); return;
    }
    if (is("Neotree", "NvimTreeToggle", "NERDTreeToggle", "Lex", "Lexplore", "Ex", "Explore")) { toggleTree(); return; }
    if (is("Telescope", "FzfLua", "Files", "Pick")) { finder(); return; }
    if (is("version", "ve")) { more(`HLDR v${version} (hvpaiva.dev)\nBuild type: Release · maud + axum + sqlite (WAL)\n\nFeatures: +curl +ansi16 +sqlite +markdown +colorschemes -javascript_required\n\n   system vimrc file: "content/profile.yaml"\n     user vimrc file: none. you're a visitor.`); return; }
    if (is("smile")) { more(BANNER + "\n\n              thanks for reading the source."); return; }
    if (is("intro", "Alpha", "Dashboard")) { intro(); return; }
    if (is("q", "quit", "qa", "qall", "clo", "close")) { msg("this is a website. Close the tab, or stay: :help", "w"); return; }
    if (is("q!", "qa!", "cq")) { msg("E37: No write since last change. Just kidding, there were none. The tab stays.", "w"); return; }
    if (is("w", "write", "up", "update", "wa")) { msg("E45: 'readonly' option is set (add ! to override)", "e"); return; }
    if (is("w!", "wq", "wq!", "x", "xa", "wqa")) { msg("E212: Can't open file for writing. The source is git: github.com/hvpaiva/hldr", "e"); return; }
    if (is("Lazy", "Mason", "PackerSync", "PlugInstall")) { msg("0 plugins. it's server-rendered HTML.", "w"); return; }
    msg(`E492: Not an editor command: ${s}`, "e");
  };

  // transient buffers: drawn in place, never in the URL
  const transient = (file, ft, html, cls = "") => {
    const b = document.createElement("article");
    b.className = "buf" + (cls ? " " + cls : "");
    b.dataset.path = file; b.dataset.ft = ft;
    b.innerHTML = html;
    buf.replaceWith(b); buf = b;
    $(".status .file label").textContent = file;
    $(".status .ft").textContent = ft;
    win.scrollTop = 0; mark(0, false);
  };
  const intro = () => {
    history.replaceState(null, "", "/");
    const opt = (k, label, cmd, attr) => `<a href="#" ${attr} data-key="${k}"><span class="kk">${k}</span><span>${label}</span><span class="mk">${cmd}</span></a>`;
    transient("[No Name]", "", `<div class="splash"><pre class="tildes" aria-hidden="true">${"~\n".repeat(120)}</pre><div class="intro-in">` +
      `<pre class="ban" aria-label="HLDR">${BANNER}</pre>` +
      `<p class="iv">HLDR v${esc(version)}</p><p class="mk">hvpaiva.dev · readable by anyone, edited only through git</p><div class="opts">` +
      opt("e", "README.md", ":e README.md", 'data-go="/"') + opt("f", "find file", ":find", 'data-ex="find"') +
      opt("p", "projects/", ":e projects/", 'data-go="/projects"') + opt("c", "colorscheme", ":colo", 'data-go="/theme"') +
      opt("h", "help", ":help", 'data-go="/help"') + opt("q", "quit", ":q", 'data-ex="q"') +
      `</div><p class="mk">every buffer is closed. pick one, or type :help&lt;Enter&gt; or &lt;F1&gt;</p></div></div>`, "intro");
    $("#bufs").innerHTML = '<span class="tab on"><a href="/">[No Name]</a></span>';
  };
  const health = async () => {
    const row = (id) => `<p id="${id}"><span class="mk">- </span>…</p>`;
    transient("health://", "checkhealth", `<h1 class="h1">hldr: require("hldr.health").check()</h1><p></p><h2 class="h2">site</h2>${row("hz")}${row("rz")}<p></p>` +
      `<h2 class="h2">content</h2><p><span class="ok">- OK</span> ${esc($(".status .ver a")?.textContent || "")} indexed</p><p></p>` +
      `<h2 class="h2">ui</h2><p><span class="ok">- OK</span> colorscheme ${esc(document.documentElement.dataset.theme)}</p>` +
      `<p><span class="ok">- OK</span> javascript: optional; every file is a URL</p>`);
    for (const [id, path] of [["hz", "/healthz"], ["rz", "/readyz"]]) {
      const t0 = performance.now();
      try {
        const r = await fetch(path, { cache: "no-store" });
        const ms = (performance.now() - t0).toFixed(1);
        let v = ""; try { v = (await r.json()).version; } catch { /* not json */ }
        $("#" + id).innerHTML = `<span class="${r.ok ? "ok" : "err"}">- ${r.ok ? "OK" : "ERROR"}</span> ${path} ${r.status}${v ? " · version " + esc(v) : ""} · ${ms}ms`;
      } catch { $("#" + id).innerHTML = `<span class="err">- ERROR</span> ${path} unreachable`; }
    }
  };

  // finder
  const finderEl = $("#finder"), fq = $("#fq"), fl = $("#fl"), scrim = $("#scrim");
  let fsel = 0, fshown = [];
  const renderFinder = () => {
    const q = fq.value;
    fshown = paths().map((p) => ({ p, m: q ? fuzzy(q, p) : { sc: 0, hits: [] } })).filter((x) => x.m);
    if (q) fshown.sort((a, b) => b.m.sc - a.m.sc);
    fsel = Math.min(fsel, Math.max(0, fshown.length - 1));
    fl.innerHTML = fshown.map(({ p, m }, i) => `<li data-i="${i}" class="${i === fsel ? "on" : ""}" role="option"><span>${[...p].map((c, k) => (m.hits.includes(k) ? `<mark>${esc(c)}</mark>` : esc(c))).join("")}</span></li>`).join("") || '<li class="d">no match</li>';
    fl.querySelector(".on")?.scrollIntoView({ block: "nearest" });
  };
  const finder = () => { closeFloats(); fq.value = ""; fsel = 0; finderEl.classList.add("on"); scrim.classList.add("on"); renderFinder(); fq.focus(); };
  const closeFloats = () => { finderEl.classList.remove("on"); scrim.classList.remove("on"); closeMore(); if (document.activeElement === fq) win.focus({ preventScroll: true }); };
  const openPick = (i) => { const x = fshown[i]; closeFloats(); if (x) go(FILES.get(x.p)); };
  fq.addEventListener("input", () => { fsel = 0; renderFinder(); });
  fq.addEventListener("keydown", (e) => {
    e.stopPropagation();
    if (e.key === "Escape") closeFloats();
    else if (e.key === "ArrowDown" || (e.ctrlKey && "nj".includes(e.key))) { e.preventDefault(); fsel = Math.min(fsel + 1, fshown.length - 1); renderFinder(); }
    else if (e.key === "ArrowUp" || (e.ctrlKey && "pk".includes(e.key))) { e.preventDefault(); fsel = Math.max(fsel - 1, 0); renderFinder(); }
    else if (e.key === "Enter") { e.preventDefault(); openPick(fsel); }
  });
  fl.addEventListener("click", (e) => { const li = e.target.closest("li[data-i]"); if (li) openPick(+li.dataset.i); });
  scrim.addEventListener("click", closeFloats);
  $$("[data-shut]").forEach((b) => b.addEventListener("click", closeFloats));

  // tree
  let tcur = 0;
  const toggleTree = (show) => {
    const hidden = show === undefined ? !toggle.checked : !show;
    toggle.checked = mobile() ? !hidden : hidden;
    toggle.dispatchEvent(new Event("change"));
    if (hidden && treeFocus) focusTree(false);
  };
  const treeShown = () => (mobile() ? toggle.checked : !toggle.checked);
  const rows = () => $$(".row, summary", tree).filter((r) => r.offsetParent !== null);
  const markT = () => {
    const rs = rows();
    tcur = Math.max(0, Math.min(tcur, rs.length - 1));
    rs.forEach((r, i) => r.classList.toggle("tcur", treeFocus && i === tcur));
    rs[tcur]?.scrollIntoView({ block: "nearest" });
  };
  const focusTree = (on) => {
    treeFocus = on;
    tree.classList.toggle("focus", on);
    if (on) {
      if (!treeShown()) toggleTree(true);
      const i = rows().findIndex((r) => r.classList.contains("cur"));
      tcur = i < 0 ? 0 : i;
    } else win.focus({ preventScroll: true });
    setMode(on ? "tree" : "normal"); markT();
  };
  const treeKey = (k, e) => {
    const r = rows()[tcur], d = r?.parentElement;
    const isDir = r?.tagName === "SUMMARY";
    if (k === "j" || k === "ArrowDown") { tcur++; markT(); }
    else if (k === "k" || k === "ArrowUp") { tcur--; markT(); }
    else if (k === "g") { tcur = 0; markT(); }
    else if (k === "G") { tcur = rows().length - 1; markT(); }
    else if (k === "l" || k === "o" || k === "Enter" || k === "ArrowRight") {
      const a = r?.querySelector("a");
      if (isDir && (!d.open || !a || k !== "Enter")) { d.open = !d.open || k === "l"; markT(); }
      else if (a) { focusTree(false); a.click(); }
    } else if (k === "h" || k === "ArrowLeft") {
      if (isDir && d.open) d.open = false;
      else { const p = r?.closest("details:not([open]), details")?.querySelector(":scope > summary"); if (p) tcur = rows().indexOf(p); }
      markT();
    } else if (k === "Escape" || k === "q") focusTree(false);
    else return false;
    e.preventDefault(); return true;
  };

  // clicks
  document.addEventListener("click", (e) => {
    const t = e.target;
    if ($("#more").classList.contains("on") && !t.closest("#more")) closeMore();
    const x = t.closest("[data-ex]");
    if (x) { e.preventDefault(); ex(x.dataset.ex); return; }
    const g = t.closest("[data-go]");
    if (g) { e.preventDefault(); go(g.dataset.go); return; }
    const row = t.closest(".buf > *");
    if (row && !t.closest("a")) {
      if (treeFocus) focusTree(false);
      mark(lines().indexOf(row), false);
      if (row.dataset.theme) follow(row.querySelector("a"));
    }
  });
  $("#more").addEventListener("click", closeMore);
  win.addEventListener("focusin", (e) => { const row = e.target.closest?.(".buf > *"); if (row && e.target.tagName === "A") mark(lines().indexOf(row), false); });

  // normal mode
  let pend = "", count = "", pendTimer = 0;
  const showcmd = () => ($("#showcmd").textContent = count + pend.replace(/ /g, "␣"));
  const reset = () => { pend = ""; count = ""; showcmd(); };
  const RO = "E21: Cannot make changes, 'modifiable' is off";
  const key = (e) => {
    if (e.target === cin || e.target === fq) return;
    if (e.metaKey || e.altKey) return;
    const el = document.activeElement;
    if (el?.matches("input:not(.tt), textarea, [contenteditable]")) return;
    const k = e.key;
    if (k === "F1") { e.preventDefault(); ex("help"); return; }
    if ($("#more").classList.contains("on")) {
      if (k === ":") { e.preventDefault(); closeMore(); prompt(":"); return; }
      if (k !== "Shift" && k !== "Control") { e.preventDefault(); closeMore(); }
      return;
    }
    if (finderEl.classList.contains("on")) { if (k === "Escape") closeFloats(); return; }
    const n = +count || 1;
    if (e.ctrlKey) {
      const act = {
        d: () => mark(line + half() * n), u: () => mark(line - half() * n),
        f: () => mark(line + half() * 2 * n), b: () => mark(line - half() * 2 * n),
        o: () => history.back(), i: () => history.forward(),
        h: () => focusTree(true), l: () => focusTree(false),
        "]": () => follow(el?.closest?.(".buf a") || linkOn()),
        r: () => msg("Already at newest change"),
        w: () => { pend = "^W"; showcmd(); },
      }[k];
      if (act) { e.preventDefault(); if (k !== "w") reset(); act(); }
      return;
    }
    if (pend === "^W") {
      e.preventDefault(); reset();
      if (k === "h" || k === "ArrowLeft") focusTree(true);
      else if (k === "l" || k === "ArrowRight") focusTree(false);
      else if (k === "w") focusTree(!treeFocus);
      else if (k === "q" || k === "c") window.hldrTabs.close(here);
      return;
    }
    if (treeFocus && treeKey(k, e)) return;
    if (el?.closest?.(".tree") && k === "Enter") return;
    if (k === "Shift" || k === "Control") return;
    if (k === "Escape") { reset(); closeFloats(); if (treeFocus) focusTree(false); if (!cwrap.hidden) endPrompt(); if (mobile()) toggle.checked = false; return; }
    if (buf.classList.contains("intro") && !pend && !count) { const a = $(`.buf.intro [data-key="${k}"]`); if (a) { e.preventDefault(); a.click(); return; } }
    if (/^[0-9]$/.test(k) && !pend && (count || k !== "0")) { count += k; showcmd(); return; }
    const seq = pend + k;
    clearTimeout(pendTimer);
    const done = () => { e.preventDefault(); reset(); };
    switch (seq) {
      case ":": done(); prompt(":"); return;
      case "/": case "?": done(); prompt(k); return;
      case "n": done(); for (let i = 0; i < n; i++) jump(1); return;
      case "N": done(); for (let i = 0; i < n; i++) jump(-1); return;
      case "*": { done(); const w = (lines()[line]?.textContent.match(/[\w-]{3,}/) || [""])[0]; if (w) { search = { pat: w, dir: 1 }; ss.set("search", search); jump(1); } return; }
      case "j": case "ArrowDown": done(); mark(line + n); return;
      case "k": case "ArrowUp": done(); mark(line - n); return;
      case "G": { const c = count; done(); mark(c ? +c - 1 : 1e9); return; }
      case "gg": { const c = count; done(); mark(c ? +c - 1 : 0); return; }
      case "gt": done(); cycle(1); return;
      case "gT": done(); cycle(-1); return;
      case "gx": done(); follow(linkOn(), true); return;
      case "gf": done(); follow(linkOn()); return;
      case "zz": done(); center(); return;
      case "zt": done(); win.scrollTop = lines()[line].offsetTop; return;
      case "}": done(); for (let i = 0; i < n; i++) para(1); return;
      case "{": done(); for (let i = 0; i < n; i++) para(-1); return;
      case "Enter": if (el?.tagName === "A" && el.closest(".buf")) return; done(); follow(linkOn()); return;
      case "Tab": {
        if (el !== win) return;
        const a = linkOn() || lines().slice(line).map((l) => l.querySelector("a")).find(Boolean);
        if (a) { done(); a.focus(); }
        return;
      }
      case "-": done(); go(here.startsWith("/projects/") ? "/projects" : "/"); return;
      case " e": done(); toggleTree(); return;
      case "  ": case " ff": done(); finder(); return;
      case " fh": done(); ex("help"); return;
      case "ZZ": done(); ex("wq"); return;
      case "ZQ": done(); ex("q!"); return;
      case "u": done(); msg("Already at oldest change"); return;
      case "yy": done(); navigator.clipboard?.writeText(lines()[line]?.textContent || ""); msg("1 line yanked"); return;
      case "dd": case "i": case "a": case "o": case "O": case "A": case "I": case "x": case "p": case "P": case "s": case "c": case "r": case "R": done(); msg(RO, "e"); return;
      case "q": done(); msg("recording @… no. there are no macros here.", "w"); return;
    }
    if (["g", "z", "Z", "d", "y", " ", " f"].includes(seq)) { e.preventDefault(); pend = seq; showcmd(); pendTimer = setTimeout(reset, 1200); return; }
    reset();
  };
  document.addEventListener("keydown", key);

  // boot
  win.focus({ preventScroll: true });
  const tagLine = location.hash && document.getElementById(location.hash.slice(1))?.closest(".buf > *");
  const mine = $$("[data-theme]", buf).find((l) => l.firstElementChild?.textContent.startsWith("*"));
  mark(tagLine ? lines().indexOf(tagLine) : mine ? lines().indexOf(mine) : 0, !!(tagLine || mine));
  const note = ss.get("msg", "");
  if (note) { ss.set("msg", ""); msg(note); }
  window.hldrVim = { key, run: (c) => ex(c === "find" ? "find" : c), intro };
})();
