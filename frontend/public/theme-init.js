// Paint the stored palette before the first frame. Without this the page
// renders Paper light, then flips once the app boots.
//
// index.html loads this file synchronously in <head>, before <body> is
// parsed. It is a file of its own, not an inline script, because the shell's
// Content-Security-Policy (server/src/spa.rs) allows scripts from 'self' only.
// The keys match the theme and locale stores (src/stores/theme.ts,
// src/i18n/index.ts).
(function () {
  try {
    var themes = ["paper", "slate", "forest", "ink"];
    var theme = localStorage.getItem("bookclub.theme");
    var mode = localStorage.getItem("bookclub.mode");
    if (themes.indexOf(theme) === -1) theme = "paper";
    if (mode !== "light" && mode !== "dark") {
      mode = window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "dark"
        : "light";
    }
    var locale = localStorage.getItem("bookclub.locale");
    if (locale !== "en" && locale !== "de") {
      locale = (navigator.language || "").toLowerCase().indexOf("de") === 0
        ? "de"
        : "en";
    }
    var root = document.documentElement;
    root.setAttribute("data-theme", theme);
    root.setAttribute("data-mode", mode);
    root.lang = locale;
  } catch (error) {
    /* No storage access: the :root defaults stand. */
  }
})();
