(() => {
  const help = document.querySelector(".keys-help");
  const rows = [...document.querySelectorAll("tr.post-row")];
  const prev = document.querySelector("a[rel=prev]");
  const next = document.querySelector("a[rel=next]");
  let chord = "";
  let idx = -1;
  let timer = 0;

  const go = (href) => {
    if (href) location.href = href;
  };

  const pick = (i) => {
    if (!rows.length) return;
    idx = Math.max(0, Math.min(i, rows.length - 1));
    rows.forEach((row, n) => row.classList.toggle("is-active", n === idx));
    rows[idx].querySelector("a")?.focus();
  };

  document.addEventListener("click", (event) => {
    if (event.target === help?.querySelector(".keys-help-backdrop")) {
      help.open = false;
    }
  });

  document.addEventListener("keydown", (event) => {
    if (event.metaKey || event.ctrlKey || event.altKey) return;
    const el = document.activeElement;
    if (
      el &&
      (el.tagName === "INPUT" ||
        el.tagName === "TEXTAREA" ||
        el.isContentEditable)
    ) {
      return;
    }

    if (event.key === "Escape") {
      if (help?.open) {
        help.open = false;
        event.preventDefault();
      }
      chord = "";
      return;
    }

    if (event.key === "?") {
      event.preventDefault();
      if (help) help.open = !help.open;
      chord = "";
      return;
    }

    if (help?.open) return;

    if (chord === "g") {
      chord = "";
      clearTimeout(timer);
      const dest = { h: "/", p: "/projects", a: "/about", t: "/theme" }[
        event.key
      ];
      if (dest) {
        event.preventDefault();
        go(dest);
      }
      return;
    }

    if (event.key === "g") {
      event.preventDefault();
      chord = "g";
      timer = setTimeout(() => {
        chord = "";
      }, 800);
      return;
    }

    if ((event.key === "h" || event.key === "ArrowLeft") && prev) {
      event.preventDefault();
      go(prev.href);
      return;
    }
    if ((event.key === "l" || event.key === "ArrowRight") && next) {
      event.preventDefault();
      go(next.href);
      return;
    }
    if (event.key === "j") {
      event.preventDefault();
      pick(idx < 0 ? 0 : idx + 1);
    }
    if (event.key === "k") {
      event.preventDefault();
      pick(idx < 0 ? 0 : idx - 1);
    }
    if (event.key === "Enter" && idx >= 0) {
      const href = rows[idx].querySelector(".post-title a")?.href;
      if (href) {
        event.preventDefault();
        go(href);
      }
    }
  });
})();
