const tauri = window.__TAURI__ || {};
const invokeFn = tauri?.tauri?.invoke || tauri?.core?.invoke || tauri?.invoke || null;
const openFn = tauri?.dialog?.open || null;
const saveFn = tauri?.dialog?.save || null;

function ensureDesktopApi(name, fn) {
  if (typeof fn === "function") return fn;
  throw new Error(`Tauri API 不可用：${name}。请使用 npm run tauri:dev 启动应用。`);
}

function invoke(command, args) {
  return ensureDesktopApi("invoke", invokeFn)(command, args);
}

function openDialog(options) {
  return ensureDesktopApi("dialog.open", openFn)(options);
}

function saveDialog(options) {
  return ensureDesktopApi("dialog.save", saveFn)(options);
}

/**
 * 测试/调试：同时输出到控制台并追加到本地 app.log（路径见 get_app_log_path）。
 * 在 DevTools 中也可调用 window.appLog("debug", "步骤", obj)。
 */
function appLog(level, ...parts) {
  const msg = parts
    .map((x) => (typeof x === "string" ? x : JSON.stringify(x)))
    .join(" ");
  const fn = level === "error" ? console.error : level === "warn" ? console.warn : console.log;
  fn(`[app] ${msg}`);
  if (!invokeFn) return;
  const lvl = ["error", "warn", "debug", "info"].includes(level) ? level : "info";
  invoke("append_app_log", { level: lvl, message: msg }).catch(() => {});
}

const $ = (id) => document.getElementById(id);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const btnImport = $("btnImport");
const btnParse = $("btnParse");
const btnExport = $("btnExport");
const btnClear = $("btnClear");
const btnScore = $("btnScore");

const currentFile = $("currentFile");
const parseProgressText = $("parseProgressText");

const jdInput = $("jdInput");
const jdPositionInput = $("jdPositionInput");
const jdTopN = $("jdTopN");
const scoreOut = $("scoreOut");
const parseQueue = $("parseQueue");
const pageTitle = $("pageTitle");
const libraryTableBody = $("libraryTableBody");
const jdList = $("jdList");
const recentList = $("recentList");
const jdResultTable = $("jdResultTable");

const libraryKeyword = $("libraryKeyword");
const libraryDegree = $("libraryDegree");
const libraryYears = $("libraryYears");
const librarySkill = $("librarySkill");
const templatePreview = $("templatePreview");
const resumeDetailContent = $("resumeDetailContent");
const resumeDetailBasic = $("resumeDetailBasic");
const resumeDetailWorkBody = $("resumeDetailWorkBody");
const resumeDetailProjectBody = $("resumeDetailProjectBody");
const resumeDetailBack = $("resumeDetailBack");
const jdCompareBack = $("jdCompareBack");
const jdCompareReportContent = $("jdCompareReportContent");
const tplCandidateList = $("tplCandidateList");

let importedPath = null;
let selectedQueueIndex = -1;
let importQueue = [];
let lastResumeObj = null;
let libraryRecords = [];
let jdRecords = [];
/** 最近一次「JD 筛选 / 计算匹配分」返回的候选人数，用于首页「匹配中简历」卡片 */
let lastJdMatchCount = 0;
let lastJdRankingRows = [];
let resumeDetailBackTarget = "library";
let selectedTemplateKeys = new Set();
let filteredLibraryRecords = [];
let importInProgress = false;
let appSettings = {
  llama_cli_path: "",
  model_path: "",
  threads: 4,
  temperature: 0.1,
};

const TITLE_MAP = {
  dashboard: "首页概览",
  library: "简历库",
  parse: "简历导入 & 解析",
  jd: "JD 管理 & 筛选",
  jdCompareReport: "候选人对比报告",
  resumeDetail: "简历详情",
};

const PAGE_KEYS = Object.keys(TITLE_MAP);

function switchPage(page, syncHash = true) {
  const target = PAGE_KEYS.includes(page) ? page : "dashboard";
  $$(".nav-item").forEach((x) => x.classList.remove("active"));
  document.querySelector(`.nav-item[data-page="${target}"]`)?.classList.add("active");
  $$(".page").forEach((p) => p.classList.remove("active"));
  document.querySelector(`.page[data-page="${target}"]`)?.classList.add("active");
  pageTitle.textContent = TITLE_MAP[target] || "简历管理";
  if (syncHash && window.location.hash !== `#${target}`) {
    window.location.hash = target;
  }
}

function setupNav() {
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

function jumpToStandalonePage(page) {
  clickNav(page);
}

function bindQuickActions() {
  $("quickImport")?.addEventListener("click", () => {
    clickNav("parse");
    handleImportClick();
  });
  $("quickNewJd")?.addEventListener("click", () => jumpToStandalonePage("jd"));
  $("quickBatchParse")?.addEventListener("click", () => jumpToStandalonePage("parse"));
  $("quickExport")?.addEventListener("click", () => jumpToStandalonePage("jd"));
  $("gotoParse")?.addEventListener("click", () => jumpToStandalonePage("parse"));
  $("gotoJd")?.addEventListener("click", () => jumpToStandalonePage("jd"));
  $("gotoTemplate")?.addEventListener("click", () => jumpToStandalonePage("jd"));
  resumeDetailBack?.addEventListener("click", () => jumpToStandalonePage(resumeDetailBackTarget || "library"));
  jdCompareBack?.addEventListener("click", () => jumpToStandalonePage("jd"));

  $("btnNewJd")?.addEventListener("click", async () => {
    const title = window.prompt("请输入 JD 标题：");
    if (!title) return;
    const text = jdInput.value.trim() || window.prompt("请输入 JD 内容：") || "";
    if (!text) {
      alert("JD 内容不能为空。");
      return;
    }
    try {
      await invoke("save_jd_record", { title, text });
      await refreshJdList();
      alert("JD 已保存。");
    } catch (e) {
      alert(String(e));
    }
  });

  $("libraryFilterBtn")?.addEventListener("click", applyLibraryFilters);
  $("libraryResetBtn")?.addEventListener("click", () => {
    libraryKeyword.value = "";
    libraryDegree.value = "";
    libraryYears.value = "";
    librarySkill.value = "";
    filteredLibraryRecords = [...libraryRecords];
    renderLibraryTable(filteredLibraryRecords);
  });

  $("tplRegenerate")?.addEventListener("click", updateTemplatePreview);
  $("tplExportPdf")?.addEventListener("click", async () => {
    try {
      const { selectedRows, selectedRecords } = getSelectedTemplateResolvedRecords();
      if (!selectedRows.length || !selectedRecords.length) {
        alert("请先选择候选人并生成预览内容。");
        return;
      }
      const firstName = sanitizeFileName(selectedRecords[0]?.data?.basicInfo?.name || selectedRecords[0]?.row?.candidateName || "标准简历");
      const outPath = await saveDialog({
        defaultPath: `${firstName}.pdf`,
        filters: [{ name: "PDF", extensions: ["pdf"] }],
      });
      if (!outPath) return;
      const { dirPath, sep } = splitPathForBatch(outPath);
      const usedNameCount = new Map();
      let ok = 0;
      const failed = [];
      for (let i = 0; i < selectedRecords.length; i += 1) {
        const item = selectedRecords[i];
        const baseName = sanitizeFileName(item.data?.basicInfo?.name || item.row?.candidateName || `候选人${i + 1}`);
        const seen = usedNameCount.get(baseName) || 0;
        usedNameCount.set(baseName, seen + 1);
        const fileName = seen === 0 ? `${baseName}.pdf` : `${baseName}_${seen + 1}.pdf`;
        const targetPath = dirPath ? `${dirPath}${sep}${fileName}` : fileName;
        const includeSkills = Boolean($("tplSkill")?.checked);
        const jsonPath = String(item.row?.jsonPath || "").trim();
        try {
          if (jsonPath) {
            await invoke("export_resume_pdf_from_json", { jsonPath, outPath: targetPath, includeSkills });
          } else {
            const content = buildTemplateBlock(item.data);
            await invoke("export_resume_pdf", { content, outPath: targetPath });
          }
          ok += 1;
        } catch (err) {
          failed.push(`${fileName}: ${String(err)}`);
        }
      }
      if (!failed.length) {
        alert(`PDF 导出成功：共 ${ok} 份\n目录：${dirPath || "."}`);
      } else {
        alert(`PDF 导出完成：成功 ${ok}，失败 ${failed.length}\n${failed.slice(0, 3).join("\n")}`);
      }
    } catch (e) {
      alert(String(e));
    }
  });

  $("jdCompareBtn")?.addEventListener("click", () => {
    renderJdCompareReport(lastJdRankingRows);
    jumpToStandalonePage("jdCompareReport");
  });
}

function clickNav(page) {
  if (!page) return;
  if (window.location.hash === `#${page}`) {
    switchPage(page, false);
  } else {
    window.location.hash = page;
  }
}

function applyHashRoute() {
  const hash = (window.location.hash || "").replace("#", "").trim();
  switchPage(hash || "dashboard", false);
}

function settings() {
  return appSettings;
}

async function loadSettingsFromFile() {
  try {
    const s = await invoke("load_app_settings");
    appSettings = {
      llama_cli_path: (s.llamaCliPath || "").trim(),
      model_path: (s.modelPath || "").trim(),
      threads: Number(s.threads ?? 4),
      temperature: Number(s.temperature ?? 0.1),
    };
  } catch (e) {
    appSettings = {
      llama_cli_path: "",
      model_path: "",
      threads: 4,
      temperature: 0.1,
    };
    alert(`加载配置失败：${String(e)}\n请检查项目根目录 app-config.json`);
  }
}

function setBusy(b) {
  if (btnImport) btnImport.disabled = b;
  if (btnParse) btnParse.disabled = b;
  if (btnExport) btnExport.disabled = b;
  if (btnScore) btnScore.disabled = b;
  if (btnClear) btnClear.disabled = b;
}

function renderLibraryTable(records) {
  if (!records.length) {
    libraryTableBody.innerHTML = '<tr><td colspan="9" class="muted">暂无简历数据</td></tr>';
    return;
  }
  libraryTableBody.innerHTML = records
    .map((r) => {
      const b = r.data?.basicInfo || {};
      const edu = b.education?.[0]?.degree || "";
      const latestWork = r.data?.workExperience?.["1"] || {};
      const workYears = calcWorkYears(r.data?.workExperience || {});
      const importedDate = formatDateFromEpoch(r.createdAt || r.created_at || "");
      return `<tr>
        <td>${b.name || "-"}</td>
        <td>${b.gender || "-"}</td>
        <td>${b.age || "-"}</td>
        <td>${edu || "-"}</td>
        <td>${latestWork.position || "-"}</td>
        <td>${workYears}</td>
        <td>${importedDate}</td>
          <td><button type="button" class="library-detail-btn" data-id="${r.id}">查看详情</button></td>
          <td><button type="button" class="ghost library-delete-btn" data-id="${r.id}">删除</button></td>
      </tr>`;
    })
    .join("");

  $$(".library-detail-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const id = btn.dataset.id;
      const rec = libraryRecords.find((x) => x.id === id);
      if (!rec) return;
      resumeDetailBackTarget = "library";
      renderResumeDetailPage(rec);
      jumpToStandalonePage("resumeDetail");
    });
  });

  $$(".library-delete-btn").forEach((btn) => {
    btn.addEventListener("click", async (event) => {
      event.preventDefault();
      event.stopPropagation();
      const id = btn.dataset.id;
      if (!id) return;
      try {
        await invoke("delete_resume_record", { id });
        await refreshLibraryAndStats();
      } catch (e) {
        alert(String(e));
      }
    });
  });
}

function formatDateFromEpoch(v) {
  const n = Number(v);
  if (!Number.isFinite(n) || n <= 0) return "-";
  const d = new Date(n * 1000);
  if (Number.isNaN(d.getTime())) return "-";
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function parsePeriodStart(period) {
  const text = String(period || "").trim();
  if (!text) return null;

  // 支持 YYYY.MM / YYYY-MM / YYYY/MM / YYYY年MM月
  let m = text.match(/(\d{4})\s*[.\-/年]\s*(\d{1,2})/);
  if (m) {
    const y = Number(m[1]);
    const mon = Number(m[2]);
    if (Number.isFinite(y) && Number.isFinite(mon) && mon >= 1 && mon <= 12) {
      return { y, mon };
    }
  }

  // 支持 YYYYMM（如 201912~202006）
  m = text.match(/(\d{4})(\d{2})/);
  if (m) {
    const y = Number(m[1]);
    const mon = Number(m[2]);
    if (Number.isFinite(y) && Number.isFinite(mon) && mon >= 1 && mon <= 12) {
      return { y, mon };
    }
  }

  // 仅识别到年份时，默认按 1 月处理
  m = text.match(/(\d{4})/);
  if (m) {
    const y = Number(m[1]);
    if (Number.isFinite(y)) {
      return { y, mon: 1 };
    }
  }

  return null;
}

function calcWorkYears(workExp) {
  const items = Object.values(workExp || {});
  const starts = items
    .map((x) => parsePeriodStart(x?.period))
    .filter(Boolean)
    .sort((a, b) => (a.y - b.y) || (a.mon - b.mon));

  if (!starts.length) return "-";

  const start = starts[0];
  const now = new Date();
  const months = (now.getFullYear() - start.y) * 12 + (now.getMonth() + 1 - start.mon);
  const years = Math.max(0, months / 12);
  if (years < 1) return "1年以下";
  return `${years.toFixed(1)}年`;
}

function escapeHtml(v) {
  return String(v ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function renderResumeDetailPage(rec) {
  if (!resumeDetailContent || !resumeDetailBasic || !resumeDetailWorkBody || !resumeDetailProjectBody) return;

  const b = rec?.data?.basicInfo || {};
  const edu = b.education?.[0] || {};
  const work = rec?.data?.workExperience || {};
  const proj = rec?.data?.projectExperience || {};

  const basicItems = [
    ["姓名", b.name || "-"],
    ["性别", b.gender || "-"],
    ["年龄", b.age || "-"],
    ["最高学历", edu.degree || "-"],
    ["毕业院校", edu.school || "-"],
    ["毕业时间", edu.graduationDate || "-"],
    ["目标岗位", work?.["1"]?.position || "-"],
    ["工作年限", calcWorkYears(work)],
    ["导入日期", formatDateFromEpoch(rec.createdAt || rec.created_at || "")],
    ["技能标签", (b.skills || []).join("、") || "-"],
  ];

  resumeDetailBasic.innerHTML = basicItems
    .map(([k, v]) => `<div class="detail-item"><span class="detail-k">${escapeHtml(k)}</span><span class="detail-v">${escapeHtml(v)}</span></div>`)
    .join("");

  const workRows = Object.keys(work)
    .sort((a, b) => Number(a) - Number(b))
    .map((k, index) => ({ ...(work[k] || {}), __idx: index }))
    .sort((a, b) => {
      const sa = parsePeriodStart(a.period);
      const sb = parsePeriodStart(b.period);
      if (sa && sb) {
        if (sa.y !== sb.y) return sa.y - sb.y;
        if (sa.mon !== sb.mon) return sa.mon - sb.mon;
      } else if (sa && !sb) {
        return -1;
      } else if (!sa && sb) {
        return 1;
      }
      return a.__idx - b.__idx;
    });

  if (!workRows.length) {
    resumeDetailWorkBody.innerHTML = '<tr><td colspan="4" class="muted">暂无工作经历</td></tr>';
  } else {
    resumeDetailWorkBody.innerHTML = workRows
      .map((w) => `<tr>
        <td>${escapeHtml(w.period || "-")}</td>
        <td>${escapeHtml(w.company || "-")}</td>
        <td>${escapeHtml(w.position || "-")}</td>
        <td>${escapeHtml(w.description || "-")}</td>
      </tr>`)
      .join("");
  }

  const projectRows = Object.keys(proj)
    .sort((a, b) => Number(a) - Number(b))
    .map((k) => proj[k] || {});

  if (!projectRows.length) {
    resumeDetailProjectBody.innerHTML = '<tr><td colspan="3" class="muted">暂无项目经历</td></tr>';
  } else {
    resumeDetailProjectBody.innerHTML = projectRows
      .map((p) => `<tr>
        <td>${escapeHtml(p.projectName || "-")}</td>
        <td>${escapeHtml(p.projectDescription || "-")}</td>
        <td>${escapeHtml(p.projectAchievements || "-")}</td>
      </tr>`)
      .join("");
  }
}

function applyLibraryFilters() {
  const keyword = (libraryKeyword?.value || "").trim().toLowerCase();
  const degree = (libraryDegree?.value || "").trim().toLowerCase();
  const skill = (librarySkill?.value || "").trim().toLowerCase();

  filteredLibraryRecords = libraryRecords.filter((r) => {
    const b = r.data?.basicInfo || {};
    const name = (b.name || "").toLowerCase();
    const skills = (b.skills || []).join(" ").toLowerCase();
    const topEdu = (b.education?.[0]?.degree || "").toLowerCase();
    const pos = (r.data?.workExperience?.["1"]?.position || "").toLowerCase();

    if (keyword && !(name.includes(keyword) || skills.includes(keyword) || pos.includes(keyword))) return false;
    if (degree && !topEdu.includes(degree)) return false;
    if (skill && !skills.includes(skill)) return false;
    return true;
  });

  renderLibraryTable(filteredLibraryRecords);
}

function renderRecent(records) {
  if (!records.length) {
    recentList.innerHTML = '<tr><td colspan="4" class="muted">暂无数据</td></tr>';
    return;
  }
  recentList.innerHTML = records
    .slice(0, 5)
    .map((r) => {
      const b = r.data?.basicInfo || {};
      return `<tr>
        <td>${b.name || "-"}</td>
        <td>-</td>
        <td>${r.data?.workExperience?.["1"]?.position || "-"}</td>
        <td>已完成</td>
      </tr>`;
    })
    .join("");
}

function renderJdList(records) {
  if (!jdList) {
    return;
  }
  if (!records.length) {
    jdList.innerHTML = "<li>暂无 JD</li>";
    return;
  }
  jdList.innerHTML = records
    .slice(0, 20)
    .map((r) => `<li><strong>${r.title}</strong><br/><span class="small">${(r.text || "").slice(0, 80)}...</span></li>`)
    .join("");
}

function countImportedToday(records) {
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const startSec = Math.floor(start.getTime() / 1000);
  const endSec = startSec + 86400;
  let n = 0;
  for (const r of records) {
    const t = parseInt(r.createdAt, 10);
    if (Number.isFinite(t) && t >= startSec && t < endSec) n += 1;
  }
  return n;
}

/** 与「开始解析」按钮一致：已抽取文本且尚未标记为已完成的队列项 */
function pendingParseQueueCount() {
  return importQueue.filter((x) => !x.error && x.text && x.status !== "已完成").length;
}

function updateStats() {
  const elTotal = $("statTotal");
  const elToday = $("statToday");
  const elPending = $("statPending");
  const elMatching = $("statMatching");
  if (!elTotal || !elToday || !elPending || !elMatching) return;
  elTotal.textContent = String(libraryRecords.length);
  elToday.textContent = String(countImportedToday(libraryRecords));
  elPending.textContent = String(pendingParseQueueCount());
  elMatching.textContent = String(lastJdMatchCount);
}

async function refreshLibraryAndStats() {
  libraryRecords = await invoke("list_resume_library");
  filteredLibraryRecords = [...libraryRecords];
  renderLibraryTable(filteredLibraryRecords);
  renderRecent(libraryRecords);
  updateStats();
  updateTemplatePreview();
}

async function refreshJdList() {
  jdRecords = await invoke("list_jd_records");
  renderJdList(jdRecords);
  updateStats();
}

function renderQueueTable() {
  if (!importQueue.length) {
    parseQueue.innerHTML = '<tr><td colspan="3" class="muted">暂无任务</td></tr>';
    if (parseProgressText) parseProgressText.textContent = "0/0 (0%)";
    updateStats();
    return;
  }

  const done = importQueue.filter((x) => x.status === "已完成" || x.status === "需修正").length;
  const total = importQueue.length;
  const percent = Math.round((done / total) * 100);
  if (parseProgressText) {
    parseProgressText.textContent = `${done}/${total} (${percent}%)`;
  }

  parseQueue.innerHTML = importQueue
    .map((item) => {
      const status = item.status || "待解析";
      const progress = Number.isFinite(item.progress) ? Math.max(0, Math.min(100, Math.round(item.progress))) : 0;
      const elapsedText = formatDurationMs(item.parseElapsedMs);
      const isError = status === "需修正";
      return `<tr>
        <td>${item.fileName}</td>
        <td>
          <div class="queue-progress-wrap">
            <div class="queue-progress-meta">
              <span>${status}</span>
              <span>${progress}%</span>
            </div>
            <div class="queue-progress-track">
              <div class="queue-progress-fill${isError ? " is-error" : ""}" style="width:${progress}%"></div>
            </div>
          </div>
        </td>
        <td>${elapsedText}</td>
      </tr>`;
    })
    .join("");
  updateStats();
}

function clearProgressTimer(item) {
  if (item && item.progressTimer) {
    clearInterval(item.progressTimer);
    item.progressTimer = null;
  }
}

function animateProgressTo(item, target, stepMs = 24) {
  if (!item) return;
  const safeTarget = Math.max(0, Math.min(100, Math.round(target)));
  const current = Number.isFinite(item.progress) ? Math.round(item.progress) : 0;
  if (current === safeTarget) {
    item.progress = safeTarget;
    renderQueueTable();
    return;
  }

  clearProgressTimer(item);
  const direction = safeTarget > current ? 1 : -1;
  item.progress = current;

  item.progressTimer = setInterval(() => {
    const now = Number.isFinite(item.progress) ? Math.round(item.progress) : 0;
    const next = now + direction;
    const reached = direction > 0 ? next >= safeTarget : next <= safeTarget;
    item.progress = reached ? safeTarget : next;
    renderQueueTable();
    if (reached) {
      clearProgressTimer(item);
    }
  }, stepMs);
}

function formatDurationMs(ms) {
  if (!Number.isFinite(ms) || ms < 0) return "-";
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = Math.round(ms / 100) / 10;
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = (totalSeconds % 60).toFixed(1).padStart(4, "0");
  return `${minutes}m ${seconds}s`;
}

function renderStructuredCards(resume) {
  const b = resume?.basicInfo || {};
  basicInfoCard.textContent = [
    `姓名：${b.name || ""}`,
    `年龄：${b.age || ""}`,
    `性别：${b.gender || ""}`,
    `学历：${b.education?.[0]?.degree || ""}`,
  ].join(" | ");

  skillsTags.innerHTML = "";
  const skills = Array.isArray(b.skills) ? b.skills : [];
  if (!skills.length) {
    skillsTags.innerHTML = '<span class="muted small">暂无技能标签</span>';
    return;
  }
  skills.forEach((s) => {
    const el = document.createElement("span");
    el.className = "tag";
    el.textContent = s;
    skillsTags.appendChild(el);
  });
}

function updateTemplatePreview() {
  const { selectedRows, selectedRecords } = getSelectedTemplateResolvedRecords();
  if (!selectedRows.length || !selectedRecords.length) {
    templatePreview.textContent = "请先在上方“候选人选择”勾选要生成的简历。";
    return;
  }
  const blocks = selectedRecords.map((item) => buildTemplateBlock(item.data));
  const missingCount = Math.max(0, selectedRows.length - selectedRecords.length);
  const warning = missingCount > 0
    ? `提示：有 ${missingCount} 份候选人未匹配到完整简历数据，已自动跳过。\n\n`
    : "";
  templatePreview.textContent = `${warning}${blocks.join("\n\n------------------------------\n\n")}`;
}

function cleanInline(v) {
  return String(v ?? "").replace(/\s+/g, " ").trim();
}

function buildTemplateBlock(src) {
  const b = src.basicInfo || {};
  const workRows = Object.keys(src.workExperience || {})
    .sort((a, b) => Number(a) - Number(b))
    .map((k) => src.workExperience?.[k] || {})
    .filter((x) => x.company || x.position || x.period || x.description);
  const projectRows = Object.keys(src.projectExperience || {})
    .sort((a, b) => Number(a) - Number(b))
    .map((k) => src.projectExperience?.[k] || {})
    .filter((x) => x.projectName || x.projectDescription || x.projectAchievements);
  const lines = [];
  lines.push(`# ${cleanInline(b.name) || "候选人姓名"}`);
  lines.push("## 基础信息");
  lines.push(`- 性别：${cleanInline(b.gender) || "-"}`);
  lines.push(`- 年龄：${cleanInline(b.age) || "-"}`);
  lines.push(`- 联系方式：${cleanInline(b.contact) || "-"}`);

  const eduRows = Array.isArray(b.education) ? b.education.filter((e) => e?.school || e?.degree || e?.major || e?.period || e?.graduationDate) : [];
  if (eduRows.length) {
    lines.push("## 教育背景");
    eduRows.forEach((e) => {
      const period = cleanInline(e.period || e.graduationDate || "-");
      lines.push(`- ${cleanInline(e.school) || "-"} / ${cleanInline(e.degree) || "-"} / ${cleanInline(e.major) || "-"} / ${period}`);
    });
  }

  if ($("tplSkill")?.checked && Array.isArray(b.skills) && b.skills.length) {
    lines.push("## 技能");
    lines.push(`- ${(b.skills || []).map((s) => cleanInline(s)).filter(Boolean).join(" / ")}`);
  }

  if (workRows.length) {
    lines.push("## 工作经历");
    workRows.forEach((w) => {
      lines.push(`- ${cleanInline(w.company) || "-"} / ${cleanInline(w.position) || "-"} / ${cleanInline(w.period) || "-"}`);
      if (w.description) lines.push(`  - ${cleanInline(w.description)}`);
    });
  }

  if (projectRows.length) {
    lines.push("## 项目经历");
    projectRows.forEach((p) => {
      lines.push(`- ${cleanInline(p.projectName) || "-"}：${cleanInline(p.projectDescription) || "-"}`);
      if (p.projectAchievements) lines.push(`  - 成果：${cleanInline(p.projectAchievements)}`);
    });
  }
  return lines.join("\n");
}

function getSelectedTemplateResolvedRecords() {
  const selectedRows = getCurrentTemplateRows().filter((row) => selectedTemplateKeys.has(rowKeyForTemplate(row)));
  const selectedRecords = selectedRows
    .map((row) => ({ row, rec: resolveResumeRecordFromJdRow(row) }))
    .filter((x) => Boolean(x.rec))
    .map((x) => ({ row: x.row, data: x.rec.data }));
  return { selectedRows, selectedRecords };
}

function sanitizeFileName(name) {
  const cleaned = String(name || "")
    .replace(/[\\/:*?"<>|]/g, "_")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned || "候选人";
}

function splitPathForBatch(path) {
  const raw = String(path || "");
  const slashIdx = raw.lastIndexOf("/");
  const backslashIdx = raw.lastIndexOf("\\");
  const idx = Math.max(slashIdx, backslashIdx);
  if (idx < 0) return { dirPath: "", sep: "\\" };
  const sep = idx === backslashIdx ? "\\" : "/";
  return { dirPath: raw.slice(0, idx), sep };
}

function rowKeyForTemplate(row) {
  return String(row.resumeId || row.parsedId || row.sourceFile || row.candidateName || "").trim();
}

function getCurrentTemplateRows() {
  const selectedTopN = Math.max(1, Number(jdTopN?.value || 10));
  return (lastJdRankingRows || []).slice(0, selectedTopN);
}

function getSelectedTemplateRecords() {
  const rows = getCurrentTemplateRows();
  return rows
    .filter((row) => selectedTemplateKeys.has(rowKeyForTemplate(row)))
    .map((row) => resolveResumeRecordFromJdRow(row))
    .filter(Boolean)
    .map((rec) => rec.data);
}

function renderTemplateCandidatePicker(rows) {
  if (!tplCandidateList) return;
  if (!rows.length) {
    tplCandidateList.innerHTML = '<span class="muted">请先在上方执行“计算匹配分”，此处将同步展示筛选名单。</span>';
    selectedTemplateKeys = new Set();
    updateTemplatePreview();
    return;
  }

  const nextSelected = new Set();
  rows.forEach((row) => {
    const key = rowKeyForTemplate(row);
    if (!key) return;
    if (selectedTemplateKeys.has(key) || selectedTemplateKeys.size === 0) {
      nextSelected.add(key);
    }
  });
  selectedTemplateKeys = nextSelected;

  tplCandidateList.innerHTML = rows
    .map((row) => {
      const key = rowKeyForTemplate(row);
      const checked = selectedTemplateKeys.has(key) ? "checked" : "";
      const label = `${row.candidateName || "-"}（匹配分：${row.score ?? 0}）`;
      return `<label style="display:flex;align-items:center;justify-content:space-between;gap:10px;margin:6px 0;padding:6px 8px;border:1px solid var(--line);border-radius:8px;background:#fff;">
        <span>${escapeHtml(label)}</span>
        <input type="checkbox" class="tpl-candidate-checkbox" data-key="${escapeHtml(key)}" ${checked} />
      </label>`;
    })
    .join("");

  $$(".tpl-candidate-checkbox").forEach((el) => {
    el.addEventListener("change", () => {
      const key = String(el.dataset.key || "").trim();
      if (!key) return;
      if (el.checked) selectedTemplateKeys.add(key);
      else selectedTemplateKeys.delete(key);
      updateTemplatePreview();
    });
  });
  updateTemplatePreview();
}

function resolveResumeRecordFromJdRow(row) {
  if (!row) return null;
  const resumeId = String(row.resumeId || "").trim();
  if (resumeId) {
    const byId = libraryRecords.find((r) => String(r.id || "").trim() === resumeId);
    if (byId) return byId;
  }

  const candidateName = String(row.candidateName || "").trim().toLowerCase();
  const sourceFile = String(row.sourceFile || "").trim().toLowerCase();
  const sourceBase = sourceFile.split(/[\\/]/).pop() || "";
  const age = String(row.age || "").trim();
  const contact = String(row.contact || "").replace(/\D+/g, "");

  const exact = libraryRecords.find((r) => {
    const b = r.data?.basicInfo || {};
    const nameMatched = candidateName && String(b.name || "").trim().toLowerCase() === candidateName;
    const sourceMatched = sourceFile && String(r.sourceFile || "").trim().toLowerCase() === sourceFile;
    const sourceBaseMatched = sourceBase && (String(r.sourceFile || "").trim().toLowerCase().split(/[\\/]/).pop() || "") === sourceBase;
    const ageMatched = age && String(b.age || "").trim() === age;
    const contactMatched = contact && String(b.contact || "").replace(/\D+/g, "").endsWith(contact);
    return (nameMatched && sourceMatched)
      || (nameMatched && sourceBaseMatched)
      || (nameMatched && ageMatched && contactMatched);
  });
  if (exact) return exact;

  if (candidateName) {
    const sameName = libraryRecords.filter((r) => String(r.data?.basicInfo?.name || "").trim().toLowerCase() === candidateName);
    if (sameName.length === 1) return sameName[0];
  }
  return null;
}

function renderJdRanking(rows) {
  if (!rows.length) {
    jdResultTable.innerHTML = '<tr><td colspan="4" class="muted">未发现可用于筛选的本地解析结果</td></tr>';
    return;
  }
  const selectedTopN = Math.max(1, Number(jdTopN?.value || 10));
  jdResultTable.innerHTML = rows
    .slice(0, selectedTopN)
    .map((x, idx) => {
      const b = x.scoreBreakdown || {};
      const breakdown = `技${b.skillScore ?? 0}/年${b.yearsScore ?? 0}/学${b.degreeScore ?? 0}/工${b.workScore ?? 0}/项${b.projectScore ?? 0}`;
      return `<tr>
        <td>${x.candidateName || "-"}</td>
        <td>${(x.matchedKeywords || []).join(", ") || "-"}</td>
        <td>${x.score}<br/><span class="small muted">${breakdown}</span></td>
        <td><button type="button" class="jd-detail-btn" data-idx="${idx}">查看详情</button></td>
      </tr>`;
    })
    .join("");

  $$(".jd-detail-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = Number(btn.dataset.idx);
      if (!Number.isFinite(idx)) return;
      const row = rows[idx];
      const rec = resolveResumeRecordFromJdRow(row);
      if (!rec) {
        alert("未找到对应的简历详情，可能已被删除。请先到简历库确认数据。");
        return;
      }
      resumeDetailBackTarget = "jd";
      renderResumeDetailPage(rec);
      jumpToStandalonePage("resumeDetail");
    });
  });
}

function renderJdCompareReport(rows) {
  if (!jdCompareReportContent) return;
  if (!rows || !rows.length) {
    jdCompareReportContent.innerHTML = '<p class="muted">暂无可对比数据，请先在 JD 页面计算匹配分。</p>';
    return;
  }

  const selectedTopN = Math.max(1, Number(jdTopN?.value || 10));
  const reportRows = rows.slice(0, selectedTopN).map((row) => {
    const b = row.scoreBreakdown || {};
    const rec = resolveResumeRecordFromJdRow(row);
    const workYears = rec ? calcWorkYears(rec.data?.workExperience || {}) : (row.workYears || "-");
    const degree = rec?.data?.basicInfo?.education?.[0]?.degree || row.degree || "-";
    return {
      name: row.candidateName || "-",
      score: row.score ?? 0,
      matchedKeywords: (row.matchedKeywords || []).join("、") || "-",
      skillScore: b.skillScore ?? 0,
      yearsScore: b.yearsScore ?? 0,
      degreeScore: b.degreeScore ?? 0,
      workScore: b.workScore ?? 0,
      projectScore: b.projectScore ?? 0,
      degree,
      workYears,
    };
  });

  const avgScore = Math.round(reportRows.reduce((sum, x) => sum + Number(x.score || 0), 0) / reportRows.length);
  const top = reportRows[0];
  const weak = reportRows[reportRows.length - 1];
  jdCompareReportContent.innerHTML = `
    <div class="mini-card">
      <h3>报告摘要</h3>
      <p class="small muted">候选人数：${reportRows.length}，平均匹配分：${avgScore}，最高分：${top.name}（${top.score}），最低分：${weak.name}（${weak.score}）。</p>
    </div>
    <table class="table">
      <thead>
        <tr>
          <th>姓名</th>
          <th>总分</th>
          <th>学历</th>
          <th>工作年限</th>
          <th>分项（技/年/学/工/项）</th>
          <th>匹配关键词</th>
        </tr>
      </thead>
      <tbody>
        ${reportRows.map((x) => `<tr>
          <td>${escapeHtml(x.name)}</td>
          <td>${escapeHtml(String(x.score))}</td>
          <td>${escapeHtml(x.degree)}</td>
          <td>${escapeHtml(x.workYears)}</td>
          <td>${escapeHtml(`${x.skillScore}/${x.yearsScore}/${x.degreeScore}/${x.workScore}/${x.projectScore}`)}</td>
          <td>${escapeHtml(x.matchedKeywords)}</td>
        </tr>`).join("")}
      </tbody>
    </table>
  `;
}

async function handleImportClick() {
  if (importInProgress) return;
  try {
    importInProgress = true;
    setBusy(true);
    currentFile.textContent = "正在选择文件...";
    const selected = await openDialog({
      multiple: true,
      filters: [{
        name: "Resume",
        extensions: ["docx", "pdf"]
      }]
    });

    let files = [];
    if (Array.isArray(selected)) {
      files = selected;
    } else if (selected === null) {
      currentFile.textContent = "已取消选择";
      return;
    } else {
      files = [selected];
    }

    if (!files.length) {
      currentFile.textContent = "未选择文件";
      return;
    }
    
    currentFile.textContent = `准备导入 ${files.length} 个文件...`;

    importQueue.forEach((x) => clearProgressTimer(x));
    importQueue = files.map((filePath) => ({
      filePath,
      fileName: filePath.split(/[\\/]/).pop() || filePath,
      status: "抽取中",
      progress: 0,
      progressTimer: null,
      parseElapsedMs: null,
      text: "",
      error: "",
    }));
    renderQueueTable();

    for (let i = 0; i < importQueue.length; i += 1) {
      const item = importQueue[i];
      try {
        item.status = "抽取中";
        animateProgressTo(item, 20, 28);
        renderQueueTable();
        const extracted = await invoke("extract_text", { filePath: item.filePath });
        clearProgressTimer(item);
        item.text = extracted || "";
        item.status = "待解析";
        animateProgressTo(item, 30, 20);
      } catch (err) {
        clearProgressTimer(item);
        item.error = String(err);
        item.status = "需修正";
        animateProgressTo(item, 100, 10);
      }
      renderQueueTable();
    }

    const firstOk = importQueue.findIndex((x) => !x.error && x.text);
    const successCount = importQueue.filter((x) => !x.error && x.text).length;
    const failCount = importQueue.length - successCount;
    if (firstOk >= 0) {
      selectedQueueIndex = firstOk;
      importedPath = importQueue[firstOk].filePath;
      currentFile.textContent = importedPath;
    } else {
      importedPath = null;
      currentFile.textContent = "未选择";
    }

    if (failCount !== 0) {
      const firstErr = importQueue.find((x) => x.error)?.error || "未知错误";
      alert(`导入完成：成功 ${successCount}，失败 ${failCount}\n首个失败原因：${firstErr}`);
    }
  } catch (e) {
    console.error(e);
    alert(String(e));
    currentFile.textContent = `导入失败：${String(e)}`;
  } finally {
    setBusy(false);
    importInProgress = false;
  }
}

function bindImportTriggers() {
  btnImport?.addEventListener("click", (event) => {
    event.preventDefault();
    handleImportClick();
  });

  // 兜底：即使按钮直接绑定失效，也能通过事件委托触发导入。
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target.closest("button") : null;
    if (!target || target.id !== "btnImport") return;
    event.preventDefault();
    handleImportClick();
  });
}

bindImportTriggers();

btnParse.addEventListener("click", async () => {
  try {
    setBusy(true);
    if (!appSettings.llama_cli_path || !appSettings.model_path) {
      alert("请先在项目根目录 app-config.json 中配置 llamaCliPath 和 modelPath");
      return;
    }
    const pending = importQueue.filter((x) => !x.error && x.text && x.status !== "已完成");
    if (!pending.length) {
      alert("没有可解析任务。请先导入并抽取文本。");
      return;
    }

    let okCount = 0;
    let failCount = 0;
    for (const item of importQueue) {
      if (item.error || !item.text || item.status === "已完成") continue;
      item.status = "解析中";
      animateProgressTo(item, 60, 18);
      const parseStartedAt = Date.now();
      renderQueueTable();
      currentFile.textContent = item.filePath;

      // 解析阶段缓慢逼近 95%，避免进度条突然跳变。
      const parseDriftTimer = setInterval(() => {
        const p = Number.isFinite(item.progress) ? item.progress : 0;
        if (p < 95) {
          item.progress = Math.min(95, Math.round(p) + 1);
          renderQueueTable();
          return;
        }
        clearInterval(parseDriftTimer);
      }, 140);

      try {
        const parsed = await invoke("parse_resume", { text: item.text, settings: settings() });
        lastResumeObj = parsed;

        const savedRecord = await invoke("save_resume_to_library", {
          sourceFile: item.filePath || "manual-input",
          resumeObj: parsed,
        });

        await invoke("save_parsed_result_json", {
          sourceFile: item.filePath || "manual-input",
          resumeId: savedRecord?.id || "",
          resumeObj: parsed,
        });

        await refreshLibraryAndStats();

        clearInterval(parseDriftTimer);
        clearProgressTimer(item);
        item.status = "已完成";
        animateProgressTo(item, 100, 10);
        item.parseElapsedMs = Date.now() - parseStartedAt;
        okCount += 1;
      } catch (e) {
        clearInterval(parseDriftTimer);
        clearProgressTimer(item);
        item.error = String(e);
        item.status = "需修正";
        animateProgressTo(item, 100, 10);
        item.parseElapsedMs = Date.now() - parseStartedAt;
        failCount += 1;
      }
      renderQueueTable();
    }
    alert(`批量解析完成：成功 ${okCount}，失败 ${failCount}`);
  } catch (e) {
    alert(String(e));
  } finally {
    setBusy(false);
  }
});

btnExport?.addEventListener("click", async () => {
  try {
    setBusy(true);
    if (!lastResumeObj) {
      alert("没有可导出的结构化结果。请先解析。");
      return;
    }
    const outPath = await saveDialog({
      defaultPath: "resume_data.js",
      filters: [{ name: "JavaScript", extensions: ["js"] }],
    });
    if (!outPath) return;
    await invoke("export_js", { resumeObj: lastResumeObj, outPath });
    alert("导出成功：" + outPath);
  } catch (e) {
    alert(String(e));
  } finally {
    setBusy(false);
  }
});

btnScore.addEventListener("click", async () => {
  try {
    setBusy(true);
    const position = (jdPositionInput?.value || "").trim();
    const jd = jdInput.value.trim();
    if (!jd) {
      alert("请粘贴 JD 文本。");
      return;
    }
    if (!position) {
      alert("请先输入岗位。");
      return;
    }

    const limit = Number(jdTopN?.value || 10);
    const rows = await invoke("jd_filter_by_keywords", { position, jdText: jd, limit });

    if (!rows.length) {
      lastJdMatchCount = 0;
      lastJdRankingRows = [];
      renderTemplateCandidatePicker([]);
      scoreOut.textContent = "score=- (matched=0/0)";
      renderJdRanking([]);
      updateStats();
      return;
    }

    const top = rows[0];
    const b = top.scoreBreakdown || {};
    lastJdMatchCount = rows.length;
    lastJdRankingRows = rows.slice();
    renderTemplateCandidatePicker(getCurrentTemplateRows());
    scoreOut.textContent = `top=${top.score} | 技${b.skillScore ?? 0} 年${b.yearsScore ?? 0} 学${b.degreeScore ?? 0} 工${b.workScore ?? 0} 项${b.projectScore ?? 0}`;
    renderJdRanking(rows);
    updateStats();
  } catch (e) {
    alert(String(e));
  } finally {
    setBusy(false);
  }
});

btnClear.addEventListener("click", () => {
  importQueue.forEach((item) => clearProgressTimer(item));
  importedPath = null;
  selectedQueueIndex = -1;
  importQueue = [];
  lastResumeObj = null;
  lastJdMatchCount = 0;
  lastJdRankingRows = [];
  selectedTemplateKeys = new Set();
  renderTemplateCandidatePicker([]);
  currentFile.textContent = "未选择";
  renderQueueTable();
  jdInput.value = "";
  scoreOut.textContent = "";
});

setupNav();
bindQuickActions();
if (invokeFn) {
  window.appLog = appLog;
  invoke("get_app_log_path")
    .then((p) => console.info("[resume-manager] 测试日志文件:", p))
    .catch(() => {});
  appLog("info", "前端已加载");
}
refreshLibraryAndStats().catch((e) => console.error(e));
refreshJdList().catch((e) => console.error(e));
loadSettingsFromFile();
applyHashRoute();
window.addEventListener("hashchange", applyHashRoute);

["tplSkill"].forEach((id) => {
  $(id)?.addEventListener("change", updateTemplatePreview);
});

jdTopN?.addEventListener("change", () => {
  renderJdRanking(lastJdRankingRows);
  renderTemplateCandidatePicker(getCurrentTemplateRows());
});

