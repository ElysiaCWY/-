import { PAGE_KEYS, TITLE_MAP, pageTitle } from "./state.js";
import { $$ } from "./utils.js";

export const pageCallbacks = { onPageSwitch: null };

export function switchPage(page, push = true) {
  if (!PAGE_KEYS.has(page)) page = "dashboard";
  if (pageTitle) pageTitle.textContent = TITLE_MAP[page] || page;

  const currentActive = document.querySelector(".page.active");
  if (currentActive) {
    currentActive.classList.remove("active");
    currentActive.style.display = "none";
  }

  const target = document.querySelector(`.page[data-page="${CSS.escape(page)}"]`);
  if (target) {
    target.classList.add("active");
    target.style.display = "";
  }

  $$(".nav-item").forEach((el) => {
    const p = el.dataset.page;
    if (p === page) el.classList.add("active");
    else el.classList.remove("active");
  });

  if (push) {
    if (window.location.hash !== `#${page}`) {
      window.location.hash = page;
    }
  }

  if (pageCallbacks.onPageSwitch) pageCallbacks.onPageSwitch(page);
}

export function setupNav() {
  $$(".nav-item").forEach((btn) => {
    btn.addEventListener("click", (event) => {
      event.preventDefault();
      const page = btn.dataset.page;
      if (!page) return;
      if (window.location.hash === `#${page}`) switchPage(page, false);
      else window.location.hash = page;
    });
  });
}

export function clickNav(page) {
  if (!page) return;
  if (window.location.hash === `#${page}`) {
    switchPage(page, false);
  } else {
    window.location.hash = page;
  }
}

export function applyHashRoute() {
  const hash = (window.location.hash || "").replace("#", "").trim();
  switchPage(hash || "dashboard", false);
}

export function jumpToStandalonePage(page) {
  clickNav(page);
}
