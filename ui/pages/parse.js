import {
  appSettings,
  importQueue,
  importedPath,
  selectedQueueIndex,
  lastResumeObj,
  lastJdMatchCount,
  lastJdRankingRows,
  selectedTemplateKeys,
  importInProgress,
  currentFile,
  parseProgressText,
  parseQueue,
  jdInput,
  scoreOut,
  btnImport,
  btnParse,
  btnExport,
  btnScore,
  btnClear,
  useBatchSwitch,
  settings,
  updateStats,
} from "../core/state.js";
import { invoke, openDialog, saveDialog } from "../core/api.js";
import { formatDurationMs } from "../core/utils.js";

// ── Busy state ──

function setBusy(b) {
  if (btnImport) btnImport.disabled = b;
  if (btnParse) btnParse.disabled = b;
  if (btnExport) btnExport.disabled = b;
  if (btnScore) btnScore.disabled = b;
  if (btnClear) btnClear.disabled = b;
}

// ── Queue rendering ──

export function renderQueueTable() {
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

export function clearProgressTimer(item) {
  if (item && item.progressTimer) {
    clearInterval(item.progressTimer);
    item.progressTimer = null;
  }
}

export function animateProgressTo(item, target, stepMs = 24) {
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

// ── Import ──

export async function handleImportClick() {
  if (importInProgress.value) return;
  try {
    importInProgress.value = true;
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
    importQueue.length = 0;
    importQueue.push(...files.map((filePath) => ({
      filePath,
      fileName: filePath.split(/[\\/]/).pop() || filePath,
      status: "抽取中",
      progress: 0,
      progressTimer: null,
      parseElapsedMs: null,
      text: "",
      error: "",
    })));
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
      selectedQueueIndex.value = firstOk;
      importedPath.value = importQueue[firstOk].filePath;
      currentFile.textContent = importedPath.value;
    } else {
      importedPath.value = null;
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
    importInProgress.value = false;
  }
}

function bindImportTriggers() {
  btnImport?.addEventListener("click", (event) => {
    event.preventDefault();
    handleImportClick();
  });

  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target.closest("button") : null;
    if (!target || target.id !== "btnImport") return;
    event.preventDefault();
    handleImportClick();
  });
}

// ── Page init ──

export function initParsePage() {
  bindImportTriggers();

  btnParse?.addEventListener("click", async () => {
    try {
      setBusy(true);
      const prov = (appSettings.llm_provider || "").toLowerCase();
      const isLm = prov === "lmstudio" || prov === "lm-studio" || prov === "lm_studio";
      const isDeep = prov === "deepseek";
      const isDash = prov === "dashscope" || prov === "qwen";
      const isVolc =
        prov === "doubao" || prov === "ark" || prov === "volcengine" || prov === "volc";
      const missingBase =
        !isLm && !isDeep && !isDash && !isVolc && !appSettings.llama_cli_path.trim();
      const missingModel = !appSettings.model_path.trim();
      if (missingBase || missingModel) {
        alert(
          isDash
            ? "请先在左侧「AI 配置」或 app-config.json 中配置：llmProvider 为 dashscope 或 qwen，modelPath 填控制台中的模型名（如 qwen3.5-flash-2026-02-23）；llmApiKey 或环境变量 DASHSCOPE_API_KEY；llamaCliPath 可留空（默认北京 compatible-mode）。国际区请在 llamaCliPath 填写官方国际 compatible-mode 地址。"
            : isDeep
              ? "请先在左侧「AI 配置」或 app-config.json 中配置：llmProvider 为 deepseek，modelPath 为 deepseek-v4-flash（或你的模型名）；llamaCliPath 可留空（默认官方 https://api.deepseek.com）；API Key 填 llmApiKey 或设置环境变量 DEEPSEEK_API_KEY。"
              : isVolc
                ? "请先在左侧「AI 配置」或 app-config.json 中配置：llmProvider 为 doubao（或 ark / volcengine），modelPath 填方舟模型名（如 doubao-seed-2-0-mini-260428）；llmApiKey 或环境变量 ARK_API_KEY；llamaCliPath 可留空（默认 https://ark.cn-beijing.volces.com/api/v3）。"
              : isLm
                ? "请先在左侧「AI 配置」或 app-config.json 中配置 modelPath（LM Studio 下 llamaCliPath 可留空，默认 http://127.0.0.1:1234/v1）"
                : "请先在左侧「AI 配置」或 app-config.json 中配置 llamaCliPath 和 modelPath"
        );
        return;
      }
      const pending = importQueue.filter((x) => !x.error && x.text && x.status !== "已完成");
      if (!pending.length) {
        alert("没有可解析任务。请先导入并抽取文本。");
        return;
      }

      let okCount = 0;
      let failCount = 0;

      const useBatch = useBatchSwitch && useBatchSwitch.checked;
      if (useBatch) {
        for (const item of pending) {
          item.status = "解析中";
          animateProgressTo(item, 80, 18);
        }
        renderQueueTable();

        try {
          const batchItems = pending.map(x => ({ file_path: x.filePath || "manual-input", text: x.text }));
          const parseStartedAt = Date.now();
          const results = await invoke("batch_parse_and_save", { items: batchItems, settings: settings() });

          for (const res of results) {
            const item = importQueue.find(x => (x.filePath || "manual-input") === res.file_path);
            if (item) {
              clearProgressTimer(item);
              if (res.success) {
                item.status = "已完成";
                animateProgressTo(item, 100, 10);
                okCount++;
              } else {
                item.status = "需修正";
                item.error = res.error;
                animateProgressTo(item, 100, 10);
                failCount++;
              }
              item.parseElapsedMs = Date.now() - parseStartedAt;
            }
          }
          const { refreshLibraryAndStats } = await import("./library.js");
          await refreshLibraryAndStats();
          renderQueueTable();
        } catch (err) {
          console.error("批处理失败：", err);
          alert("整个批处理任务失败：" + err);
          for (const item of pending) {
             item.status = "解析失败";
             item.error = String(err);
          }
          renderQueueTable();
          return;
        }
      } else {
        for (const item of importQueue) {
          if (item.error || !item.text || item.status === "已完成") continue;
          item.status = "解析中";
          animateProgressTo(item, 60, 18);
          const parseStartedAt = Date.now();
          renderQueueTable();
          currentFile.textContent = item.filePath;

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
            lastResumeObj.value = parsed.resume;

            const savedRecord = await invoke("save_resume_to_library", {
              sourceFile: item.filePath || "manual-input",
              resumeObj: parsed.resume,
            });

            await invoke("save_parsed_result_json", {
              sourceFile: item.filePath || "manual-input",
              resumeId: savedRecord?.id || "",
              resumeObj: parsed.resume,
              jdScreeningIndex: parsed.jdScreeningIndex,
            });

            const { refreshLibraryAndStats } = await import("./library.js");
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
      }

      if (okCount > 0) {
        try {
          await invoke("analyze_resumes_db");
        } catch (e) {
          console.warn("ANALYZE parsed_resumes 失败（可忽略）:", e);
        }
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
      if (!lastResumeObj.value) {
        alert("没有可导出的结构化结果。请先解析。");
        return;
      }
      const outPath = await saveDialog({
        defaultPath: "resume_data.js",
        filters: [{ name: "JavaScript", extensions: ["js"] }],
      });
      if (!outPath) return;
      await invoke("export_js", { resumeObj: lastResumeObj.value, outPath });
      alert("导出成功：" + outPath);
    } catch (e) {
      alert(String(e));
    } finally {
      setBusy(false);
    }
  });

  btnClear?.addEventListener("click", async () => {
    importQueue.forEach((item) => clearProgressTimer(item));
    importQueue.length = 0;
    importedPath.value = null;
    selectedQueueIndex.value = -1;
    lastResumeObj.value = null;
    lastJdMatchCount.value = 0;
    lastJdRankingRows.length = 0;
    selectedTemplateKeys.clear();
    const { renderTemplateCandidatePicker } = await import("./jd.js");
    renderTemplateCandidatePicker([]);
    currentFile.textContent = "未选择";
    renderQueueTable();
    if (jdInput) jdInput.value = "";
    if (scoreOut) scoreOut.textContent = "";
  });
}
