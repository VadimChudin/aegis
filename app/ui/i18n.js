"use strict";

// Interface languages. Strings are looked up by their English text, so texts that come from
// the core (setting labels, help, broker requirements, check names) translate the same way.
(() => {
  const LANGS = [
    ["en", "English"],
    ["ru", "Русский"],
    ["kk", "Қазақша"],
  ];
  const I = {
    LANGS,
    lang: "en",
    t(s, vars) {
      const d = window.I18N_DICT && window.I18N_DICT[I.lang];
      let out = (I.lang !== "en" && d && d[s]) || s;
      if (vars) out = out.replace(/\{(\w+)\}/g, (m, k) => (k in vars ? String(vars[k]) : m));
      return out;
    },
    detect() {
      const n = (navigator.language || "").toLowerCase();
      return n.startsWith("ru") ? "ru" : n.startsWith("kk") ? "kk" : "en";
    },
    set(lang) {
      I.lang = LANGS.some(([l]) => l === lang) ? lang : "en";
      document.documentElement.lang = I.lang;
      I.apply(document);
    },
    /** Translates static text, tooltips and accessible labels. */
    apply(root) {
      root.querySelectorAll("[data-i18n]").forEach((el) => {
        if (!el.dataset.i18nSrc) el.dataset.i18nSrc = el.textContent.trim();
        el.textContent = I.t(el.dataset.i18nSrc);
      });
      root.querySelectorAll("[data-i18n-title]").forEach((el) => {
        if (!el.dataset.i18nTitleSrc) el.dataset.i18nTitleSrc = el.getAttribute("title") || "";
        el.setAttribute("title", I.t(el.dataset.i18nTitleSrc));
      });
      root.querySelectorAll("[data-i18n-aria-label]").forEach((el) => {
        if (!el.dataset.i18nAriaLabelSrc) el.dataset.i18nAriaLabelSrc = el.getAttribute("aria-label") || "";
        el.setAttribute("aria-label", I.t(el.dataset.i18nAriaLabelSrc));
      });
    },
  };
  window.I18N = I;
})();
