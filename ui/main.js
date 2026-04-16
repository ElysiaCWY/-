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

const $ = (id) => document.getElementById(id);
const $$ = (sel) => Array.from(document.querySelectorAll(sel));

const fileInput = $("fileInput");
const btnImport = $("btnImport");
const btnParse = $("btnParse");
const btnExport = $("btnExport");
const btnClear = $("btnClear");
const btnScore = $("btnScore");

const currentFile = $("currentFile");
const textPreview = $("textPreview");
const jsonPreview = $("jsonPreview");

const jdInput = $("jdInput");
const scoreOut = $("scoreOut");
const parseQueue = $("parseQueue");
const basicInfoCard = $("basicInfoCard");
const skillsTags = $("skillsTags");
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

let importedPath = null;
let selectedQueueIndex = -1;
let importQueue = [];
let lastResumeObj = null;
let libraryRecords = [];
let jdRecords = [];
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
  template: "模板管理",
};

const PAGE_KEYS = Object.keys(TITLE_MAP);

function selectedPathsFromInput() {
  const files = Array.from(fileInput?.files || []);
  if (!files.length) return [];

  const paths = files
    .map((f) => (typeof f.path === "string" ? f.path.trim() : ""))
    .filter(Boolean);

  return paths;
}

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
  $("gotoTemplate")?.addEventListener("click", () => jumpToStandalonePage("template"));

  $("btnReparse")?.addEventListener("click", () => btnParse.click());
  $("btnSaveLibrary")?.addEventListener("click", async () => {
    try {
      if (!lastResumeObj) {
        alert("请先完成解析，再保存到简历库。");
        return;
      }
      await invoke("save_resume_to_library", {
        sourceFile: importedPath || "manual-input",
        resumeObj: lastResumeObj,
      });
      await refreshLibraryAndStats();
      alert("已保存到简历库。");
      jumpToStandalonePage("library");
    } catch (e) {
      alert(String(e));
    }
  });
  $("btnGenerateResume")?.addEventListener("click", () => jumpToStandalonePage("template"));
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
  $("tplExportWord")?.addEventListener("click", () => alert("桌面版原型：Word 导出将于下一阶段接入。"));
  $("tplExportPdf")?.addEventListener("click", () => alert("桌面版原型：PDF 导出将于下一阶段接入。"));

  $("jdExportBtn")?.addEventListener("click", () => alert("桌面版原型：名单导出将于下一阶段接入。"));
  $("jdViewResumeBtn")?.addEventListener("click", () => jumpToStandalonePage("library"));
  $("jdCompareBtn")?.addEventListener("click", () => alert("桌面版原型：对比报告将于下一阶段接入。"));
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
  btnImport.disabled = b;
  btnParse.disabled = b;
  btnExport.disabled = b;
  btnScore.disabled = b;
}

function renderLibraryTable(records) {
  if (!records.length) {
    libraryTableBody.innerHTML = '<tr><td colspan="7" class="muted">暂无简历数据</td></tr>';
    return;
  }
  libraryTableBody.innerHTML = records
    .map((r) => {
      const b = r.data?.basicInfo || {};
      const edu = b.education?.[0]?.degree || "";
      const skills = (b.skills || []).slice(0, 3).join(", ");
      return `<tr>
        <td>${b.name || "-"}</td>
        <td>${b.gender || "-"}</td>
        <td>${b.age || "-"}</td>
        <td>${edu || "-"}</td>
        <td>${skills || "-"}</td>
        <td>-</td>
        <td>查看详情</td>
      </tr>`;
    })
    .join("");
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
  if (!records.length) {
    jdList.innerHTML = "<li>暂无 JD</li>";
    return;
  }
  jdList.innerHTML = records
    .slice(0, 20)
    .map((r) => `<li><strong>${r.title}</strong><br/><span class="small">${(r.text || "").slice(0, 80)}...</span></li>`)
    .join("");
}

function updateStats() {
  $("statTotal").textContent = String(libraryRecords.length);
  $("statToday").textContent = "0";
  $("statPending").textContent = parseQueue.textContent?.includes("待解析") ? "1" : "0";
  $("statMatching").textContent = jdRecords.length > 0 ? String(Math.min(libraryRecords.length, 9)) : "0";
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
    parseQueue.innerHTML = '<tr><td colspan="4" class="muted">暂无任务</td></tr>';
    return;
  }
  parseQueue.innerHTML = importQueue
    .map((item, idx) => {
      const status = item.status || "待解析";
      const mode = item.mode || "规则";
      return `<tr>
        <td>${item.fileName}</td>
        <td>${status}</td>
        <td>${mode}</td>
        <td>
          <button class="queue-view-btn" data-idx="${idx}">查看结果</button>
          <button class="queue-use-btn" data-idx="${idx}">设为当前</button>
        </td>
      </tr>`;
    })
    .join("");

  $$(".queue-view-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = Number(btn.dataset.idx);
      const item = importQueue[idx];
      if (!item) return;
      if (item.error) {
        alert(item.error);
        return;
      }
      textPreview.value = item.text || "";
      currentFile.textContent = item.filePath;
      importedPath = item.filePath;
      selectedQueueIndex = idx;
    });
  });

  $$(".queue-use-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const idx = Number(btn.dataset.idx);
      const item = importQueue[idx];
      if (!item) return;
      if (item.error) {
        alert("该文件抽取失败，请先修正后再解析。");
        return;
      }
      textPreview.value = item.text || "";
      currentFile.textContent = item.filePath;
      importedPath = item.filePath;
      selectedQueueIndex = idx;
      alert(`已切换当前文件：${item.fileName}`);
    });
  });
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
  const src = lastResumeObj || libraryRecords[0]?.data;
  if (!src) {
    templatePreview.textContent = "请先在“简历导入 & 解析”完成解析，再来此页面生成预览。";
    return;
  }
  const b = src.basicInfo || {};
  const work = src.workExperience?.["1"] || {};
  const proj = src.projectExperience?.["1"] || {};
  const lines = [];
  lines.push(`# ${b.name || "候选人姓名"}`);
  lines.push(`- 性别：${b.gender || "-"}  年龄：${b.age || "-"}`);
  lines.push(`- 核心技能：${(b.skills || []).join(" / ") || "-"}`);
  if ($("tplEdu")?.checked) {
    const e = b.education?.[0] || {};
    lines.push(`- 教育：${e.school || "-"} ${e.degree || ""} ${e.major || ""}`.trim());
  }
  lines.push(`- 最近工作：${work.company || "-"} / ${work.position || "-"} / ${work.period || "-"}`);
  if ($("tplProject")?.checked) {
    lines.push(`- 代表项目：${proj.projectName || "-"}：${proj.projectAchievements || proj.projectDescription || "-"}`);
  }
  if ($("tplCert")?.checked) {
    lines.push(`- 证书：${(b.certificates || []).join("、") || "-"}`);
  }
  if ($("tplSelf")?.checked) {
    lines.push("- 自我评价：做事认真，具备良好的沟通与执行能力。");
  }
  templatePreview.textContent = lines.join("\n");
}

function renderJdRanking(result) {
  if (!libraryRecords.length) {
    jdResultTable.innerHTML = '<tr><td colspan="3" class="muted">简历库为空，请先保存简历</td></tr>';
    return;
  }
  const rows = libraryRecords
    .map((r) => {
      const b = r.data?.basicInfo || {};
      const skills = (b.skills || []).map((x) => x.toLowerCase());
      const keywords = (result.matched_keywords || []).filter((k) => skills.some((s) => s.includes(k.toLowerCase())));
      const localScore = Math.max(0, result.score - Math.max(0, 5 - keywords.length) * 4);
      return { name: b.name || "-", keywords, score: localScore };
    })
    .sort((a, b) => b.score - a.score)
    .slice(0, 10);
  jdResultTable.innerHTML = rows
    .map((x) => `<tr><td>${x.name}</td><td>${x.keywords.join(", ") || "-"}</td><td>${x.score}</td></tr>`)
    .join("");
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
        extensions: ["txt", "pdf", "docx", "doc", "jpg", "jpeg", "png", "md"]
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

    importQueue = files.map((filePath) => ({
      filePath,
      fileName: filePath.split(/[\\/]/).pop() || filePath,
      status: "解析中",
      mode: "规则",
      text: "",
      error: "",
    }));
    renderQueueTable();

    for (let i = 0; i < importQueue.length; i += 1) {
      const item = importQueue[i];
      try {
        const extracted = await invoke("extract_text", { filePath: item.filePath });
        item.text = extracted || "";
        item.status = "待解析";
      } catch (err) {
        item.error = String(err);
        item.status = "需修正";
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
      textPreview.value = importQueue[firstOk].text || "";
    } else {
      importedPath = null;
      currentFile.textContent = "未选择";
      textPreview.value = "";
    }

    if (failCount === 0) {
      alert(`导入完成：成功 ${successCount} 个文件`);
    } else {
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
    const text = textPreview.value.trim();
    if (!text) {
      alert("没有可解析的文本。请先导入并抽取文本。");
      return;
    }
    const parsed = await invoke("parse_resume", { text, settings: settings() });
    lastResumeObj = parsed;
    jsonPreview.textContent = JSON.stringify(parsed, null, 2);
    renderStructuredCards(parsed);
    if (selectedQueueIndex >= 0 && importQueue[selectedQueueIndex]) {
      importQueue[selectedQueueIndex].status = "已完成";
      importQueue[selectedQueueIndex].mode = "模型";
      renderQueueTable();
    }
  } catch (e) {
    alert(String(e));
    if (selectedQueueIndex >= 0 && importQueue[selectedQueueIndex]) {
      importQueue[selectedQueueIndex].status = "需修正";
      importQueue[selectedQueueIndex].mode = "模型";
      renderQueueTable();
    }
  } finally {
    setBusy(false);
  }
});

btnExport.addEventListener("click", async () => {
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
    if (!lastResumeObj) {
      alert("请先解析出结构化结果，再进行 JD 筛选。");
      return;
    }
    const jd = jdInput.value.trim();
    if (!jd) {
      alert("请粘贴 JD 文本。");
      return;
    }
    const result = await invoke("jd_score_v1", { resumeObj: lastResumeObj, jdText: jd });
    scoreOut.textContent = `score=${result.score} (matched=${result.matched_keywords.length}/${result.total_keywords})`;
    renderJdRanking(result);
  } catch (e) {
    alert(String(e));
  } finally {
    setBusy(false);
  }
});

btnClear.addEventListener("click", () => {
  importedPath = null;
  selectedQueueIndex = -1;
  importQueue = [];
  lastResumeObj = null;
  currentFile.textContent = "未选择";
  textPreview.value = "";
  jsonPreview.textContent = "";
  renderQueueTable();
  basicInfoCard.textContent = "暂无";
  skillsTags.innerHTML = "";
  jdInput.value = "";
  scoreOut.textContent = "";
});

setupNav();
bindQuickActions();
refreshLibraryAndStats().catch((e) => console.error(e));
refreshJdList().catch((e) => console.error(e));
loadSettingsFromFile();
applyHashRoute();
window.addEventListener("hashchange", applyHashRoute);

["tplEdu", "tplProject", "tplCert", "tplSelf", "tplName", "tplLength"].forEach((id) => {
  $(id)?.addEventListener("change", updateTemplatePreview);
});

