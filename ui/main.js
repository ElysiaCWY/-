// ── Core ──
import "./core/api.js";
import {
  filteredLibraryRecords,
  jdRecords,
  resumeDetailBackTarget,
  quickImport,
  quickNewJd,
  quickBatchParse,
  quickExport,
  gotoParse,
  gotoJd,
  gotoTemplate,
  resumeDetailBack,
  jdCompareBack,
  jdInput,
} from "./core/state.js";
import { pageCallbacks, setupNav, applyHashRoute, jumpToStandalonePage, clickNav } from "./core/router.js";
import { $, escapeHtml } from "./core/utils.js";

// ── Pages ──
import { loadSettingsFromFile, initSettingsPage } from "./pages/settings.js";
import { initWord2PdfPage } from "./pages/word2pdf.js";
import {
  initLibraryPage,
  renderLibraryTable,
  updateLibraryTotalCount,
  refreshLibraryAndStats,
  libraryCallbacks,
} from "./pages/library.js";
import {
  initJdPage,
  renderJdList,
  renderTemplateCandidatePicker,
  getCurrentTemplateRows,
  updateTemplatePreview,
  renderJdRanking,
  renderJdCompareReport,
  refreshJdList,
} from "./pages/jd.js";
import { initParsePage, handleImportClick, renderQueueTable } from "./pages/parse.js";
import { initTokenStatsPage, loadTokenStatsPage } from "./pages/tokenStats.js";
import { invoke } from "./core/api.js";

// ── Token 消耗概况（首页卡片）──

function formatTokens(n) {
  if (n == null || !Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
}

async function loadTokenSummary() {
  const el = $("tokenSummaryContent");
  if (!el) return;
  try {
    const s = await invoke("get_token_stats", { limit: 0 });
    if (s.callCount === 0) {
      el.innerHTML = `
        <div class="row" style="justify-content:space-between;align-items:center">
          <span class="muted">暂无记录，使用模型后将自动统计。</span>
          <button onclick="window.location.hash='tokenStats'" class="ghost" style="font-size:0.9em">查看详情</button>
        </div>`;
      return;
    }
    el.innerHTML = `
      <div class="row" style="justify-content:space-between;align-items:center">
        <span>累计调用 <strong>${s.callCount}</strong> 次，消耗 <strong>${formatTokens(s.totalTokens)}</strong> tokens（输入 ${formatTokens(s.totalPromptTokens)} / 输出 ${formatTokens(s.totalCompletionTokens)}）</span>
        <button onclick="window.location.hash='tokenStats'" class="ghost" style="font-size:0.9em">查看详情</button>
      </div>`;
  } catch (e) {
    el.innerHTML = `<span class="muted">加载失败</span>`;
  }
}

// ── Wire cross-module callbacks ──

libraryCallbacks.onRefresh = updateTemplatePreview;

pageCallbacks.onPageSwitch = (page) => {
  switch (page) {
    case "dashboard":
      loadTokenSummary();
      break;
    case "tokenStats":
      loadTokenStatsPage();
      break;
    case "jd":
      renderJdList(jdRecords);
      renderTemplateCandidatePicker(getCurrentTemplateRows());
      break;
    case "library":
      renderLibraryTable(filteredLibraryRecords);
      updateLibraryTotalCount();
      break;
    case "parse":
      renderQueueTable();
      break;
    case "resumeDetail":
      break;
    case "jdCompareReport":
      break;
    default:
      break;
  }
};

// ── Quick action buttons (always visible) ──

quickImport?.addEventListener("click", () => {
  clickNav("parse");
  handleImportClick();
});
quickNewJd?.addEventListener("click", () => jumpToStandalonePage("jd"));
quickBatchParse?.addEventListener("click", () => jumpToStandalonePage("parse"));
quickExport?.addEventListener("click", () => jumpToStandalonePage("jd"));
gotoParse?.addEventListener("click", () => jumpToStandalonePage("parse"));
gotoJd?.addEventListener("click", () => jumpToStandalonePage("jd"));
gotoTemplate?.addEventListener("click", () => jumpToStandalonePage("jd"));
resumeDetailBack?.addEventListener("click", () => jumpToStandalonePage(resumeDetailBackTarget.value || "library"));
jdCompareBack?.addEventListener("click", () => jumpToStandalonePage("jd"));

// ── Initialize all modules ──

setupNav();
initSettingsPage();
initWord2PdfPage();
initLibraryPage();
initJdPage();
initParsePage();
initTokenStatsPage();

window.addEventListener("hashchange", applyHashRoute);

// ── Load initial data ──

refreshLibraryAndStats().catch((e) => console.error(e));
refreshJdList().catch((e) => console.error(e));
loadSettingsFromFile();
loadTokenSummary().catch((e) => console.error(e));
applyHashRoute();
