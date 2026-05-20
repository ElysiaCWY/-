import {
  appSettings,
  libraryRecords,
  lastJdRankingRows,
  lastJdMatchCount,
  selectedTemplateKeys,
  resumeDetailBackTarget,
  jdRecords,
  jdInput,
  jdPositionInput,
  jdMinScore,
  jdResultTable,
  jdCompareBtn,
  jdCompareReportContent,
  jdList,
  btnNewJd,
  scoreOut,
  jdFilterProgressWrap,
  jdFilterProgressPhase,
  jdFilterProgressFrac,
  jdFilterProgressBar,
  tplCandidateList,
  tplName,
  tplGender,
  tplAge,
  tplContact,
  tplSkill,
  tplExportPdf,
  tplRegenerate,
  btnScore,
  btnImport,
  btnParse,
  btnExport,
  btnClear,
  updateStats,
} from "../core/state.js";
import { invoke, saveDialog } from "../core/api.js";
import {
  $,
  $$,
  escapeHtml,
  calcWorkYears,
  cleanInline,
  sanitizeFileName,
  splitPathForBatch,
  rowKeyForTemplate,
  formatJdFilterElapsed,
} from "../core/utils.js";
import { jumpToStandalonePage } from "../core/router.js";

// ── JD list ──

export function renderJdList(records) {
  if (!jdList) return;
  if (!records.length) {
    jdList.innerHTML = "<li>暂无 JD</li>";
    return;
  }
  jdList.innerHTML = records
    .slice(0, 20)
    .map((r) => `<li><strong>${r.title}</strong><br/><span class="small">${(r.text || "").slice(0, 80)}...</span></li>`)
    .join("");
}

export async function refreshJdList() {
  jdRecords.length = 0;
  const records = await invoke("list_jd_records");
  jdRecords.push(...records);
  renderJdList(jdRecords);
  updateStats();
}

// ── Template helpers ──

export function getTemplateExportOptions() {
  return {
    includeName: Boolean(tplName?.checked),
    includeGender: Boolean(tplGender?.checked),
    includeAge: Boolean(tplAge?.checked),
    includeContact: Boolean(tplContact?.checked),
    includeSkills: Boolean(tplSkill?.checked),
  };
}

export function buildTemplateBlock(src) {
  const options = getTemplateExportOptions();
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
  lines.push(`# ${options.includeName ? (cleanInline(b.name) || "候选人姓名") : "候选人姓名"}`);
  const basicLines = [];
  if (options.includeGender) basicLines.push(`- 性别：${cleanInline(b.gender) || "-"}`);
  if (options.includeAge) basicLines.push(`- 年龄：${cleanInline(b.age) || "-"}`);
  if (options.includeContact) basicLines.push(`- 联系方式：${cleanInline(b.contact) || "-"}`);
  if (basicLines.length) {
    lines.push("## 基础信息");
    lines.push(...basicLines);
  }

  const eduRows = Array.isArray(b.education) ? b.education.filter((e) => e?.school || e?.degree || e?.major || e?.period || e?.graduationDate) : [];
  if (eduRows.length) {
    lines.push("## 教育背景");
    eduRows.forEach((e) => {
      const period = cleanInline(e.period || e.graduationDate || "-");
      lines.push(`- ${cleanInline(e.school) || "-"} / ${cleanInline(e.degree) || "-"} / ${cleanInline(e.major) || "-"} / ${period}`);
    });
  }

  if (options.includeSkills && Array.isArray(b.skills) && b.skills.length) {
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

export function getCurrentTemplateRows() {
  const minScore = Math.max(0, Math.min(100, Number(jdMinScore?.value) || 0));
  return (lastJdRankingRows || []).filter((r) => (r.score || 0) >= minScore);
}

export function getSelectedTemplateRecords() {
  const rows = getCurrentTemplateRows();
  return rows
    .filter((row) => selectedTemplateKeys.has(rowKeyForTemplate(row)))
    .map((row) => resolveResumeRecordFromJdRow(row))
    .filter(Boolean)
    .map((rec) => rec.data);
}

export function getSelectedTemplateResolvedRecords() {
  const selectedRows = getCurrentTemplateRows().filter((row) => selectedTemplateKeys.has(rowKeyForTemplate(row)));
  const selectedRecords = selectedRows
    .map((row) => ({ row, rec: resolveResumeRecordFromJdRow(row) }))
    .filter((x) => Boolean(x.rec))
    .map((x) => ({ row: x.row, data: x.rec.data }));
  return { selectedRows, selectedRecords };
}

export function renderTemplateCandidatePicker(rows) {
  if (!tplCandidateList) return;
  if (!rows.length) {
    tplCandidateList.innerHTML = '<span class="muted">请先在上方执行"计算匹配分"，此处将同步展示筛选名单。</span>';
    selectedTemplateKeys.clear();
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
  selectedTemplateKeys.clear();
  for (const k of nextSelected) selectedTemplateKeys.add(k);

  tplCandidateList.innerHTML = `<div class="tpl-grid">${rows
    .map((row) => {
      const key = rowKeyForTemplate(row);
      const selected = selectedTemplateKeys.has(key);
      const cls = selected ? "tpl-chip selected" : "tpl-chip";
      const name = row.candidateName || "-";
      const score = row.score != null ? row.score : "";
      return `<button type="button" class="${cls}" data-key="${escapeHtml(key)}" title="${escapeHtml(name)} · ${score}分">
        <span class="tpl-chip-name">${escapeHtml(name)}</span>
        ${score !== "" ? `<span class="tpl-chip-score">${score}</span>` : ""}
      </button>`;
    })
    .join("")}</div>`;

  // ── 点击 + 拖拽滑动多选 ──
  const grid = tplCandidateList.querySelector(".tpl-grid");
  if (!grid) { updateTemplatePreview(); return; }

  let dragMode = null; // "select" | "deselect" | null
  let dragTouched = new Set();
  let dragMoved = false;

  const chipFromPoint = (x, y) => {
    const el = document.elementFromPoint(x, y);
    if (!el) return null;
    return el.closest(".tpl-chip");
  };

  const toggleChip = (chip) => {
    const key = String(chip.dataset.key || "").trim();
    if (!key) return;
    if (selectedTemplateKeys.has(key)) {
      selectedTemplateKeys.delete(key);
      chip.classList.remove("selected");
    } else {
      selectedTemplateKeys.add(key);
      chip.classList.add("selected");
    }
  };

  const applyDragToChip = (chip) => {
    const key = String(chip.dataset.key || "").trim();
    if (!key || dragTouched.has(key)) return;
    dragTouched.add(key);
    if (dragMode === "select" && !selectedTemplateKeys.has(key)) {
      selectedTemplateKeys.add(key);
      chip.classList.add("selected");
    } else if (dragMode === "deselect" && selectedTemplateKeys.has(key)) {
      selectedTemplateKeys.delete(key);
      chip.classList.remove("selected");
    }
  };

  grid.addEventListener("pointerdown", (e) => {
    if (e.button !== 0) return;
    const chip = chipFromPoint(e.clientX, e.clientY);
    if (!chip) return;
    dragMode = chip.classList.contains("selected") ? "deselect" : "select";
    dragTouched = new Set();
    dragMoved = false;
    grid.setPointerCapture(e.pointerId);
  });

  grid.addEventListener("pointermove", (e) => {
    if (dragMode === null) return;
    const chip = chipFromPoint(e.clientX, e.clientY);
    if (!chip) return;
    if (!dragMoved) {
      dragMoved = true;
      applyDragToChip(chip);
    } else {
      applyDragToChip(chip);
    }
  });

  const endDrag = (e) => {
    if (dragMode === null) return;
    if (!dragMoved) {
      // 未移动 = 纯点击，直接 toggle
      const chip = chipFromPoint(e.clientX, e.clientY);
      if (chip) toggleChip(chip);
    }
    dragMode = null;
    dragTouched.clear();
    dragMoved = false;
    updateTemplatePreview();
    e.preventDefault(); // 阻止后续 click 事件，避免双击切换
  };

  grid.addEventListener("pointerup", endDrag);
  grid.addEventListener("pointerleave", endDrag);
  grid.addEventListener("pointercancel", endDrag);
}

export function updateTemplatePreview() {
  // 预览区域已移除，仅更新选中状态供导出使用
}

// ── Resolve resume from JD row ──

export function resolveResumeRecordFromJdRow(row) {
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

// ── JD ranking display ──

export function renderJdRanking(rows) {
  const minScore = Math.max(0, Math.min(100, Number(jdMinScore?.value) || 0));
  const visibleRows = (rows || []).filter((r) => (r.score || 0) >= minScore);
  if (!visibleRows.length) {
    jdResultTable.innerHTML = '<tr><td colspan="4" class="muted">未发现可用于筛选的本地解析结果</td></tr>';
    return;
  }
  jdResultTable.innerHTML = visibleRows
    .map((x, idx) => {
      return `<tr>
        <td>${x.candidateName || "-"}</td>
        <td>${(x.matchedKeywords || []).join(", ") || "-"}</td>
        <td>${x.score ?? "-"}</td>
        <td><button type="button" class="jd-detail-btn" data-idx="${idx}">查看详情</button></td>
      </tr>`;
    })
    .join("");

  $$(".jd-detail-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const idx = Number(btn.dataset.idx);
      if (!Number.isFinite(idx)) return;
      const row = rows[idx];
      const rec = resolveResumeRecordFromJdRow(row);
      if (!rec) {
        alert("未找到对应的简历详情，可能已被删除。请先到简历库确认数据。");
        return;
      }
      resumeDetailBackTarget.value = "jd";
      const { renderResumeDetailPage } = await import("./library.js");
      await renderResumeDetailPage(rec);
      jumpToStandalonePage("resumeDetail");
    });
  });
}

export function renderJdCompareReport(rows) {
  if (!jdCompareReportContent) return;
  const minScore = Math.max(0, Math.min(100, Number(jdMinScore?.value) || 0));
  const reportRows = (rows || []).filter((r) => (r.score || 0) >= minScore);
  if (!reportRows.length) {
    jdCompareReportContent.innerHTML = '<p class="muted">暂无可对比数据，请先在 JD 页面计算匹配分。</p>';
    return;
  }

  const reportRowsData = reportRows.map((row) => {
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

  jdCompareReportContent.innerHTML = `
    <div class="mini-card">
      <h3>报告摘要</h3>
      <p class="small muted">候选人数：${reportRowsData.length}，当前按"不低于 ${minScore} 分"筛选展示。</p>
    </div>
    <table class="table">
      <thead>
        <tr>
          <th>姓名</th>
          <th>学历</th>
          <th>工作年限</th>
          <th>状态</th>
          <th>匹配关键词</th>
        </tr>
      </thead>
      <tbody>
        ${reportRowsData.map((x) => `<tr>
          <td>${escapeHtml(x.name)}</td>
          <td>${escapeHtml(x.degree)}</td>
          <td>${escapeHtml(x.workYears)}</td>
          <td>${x.score}</td>
          <td>${escapeHtml(x.matchedKeywords)}</td>
        </tr>`).join("")}
      </tbody>
    </table>
  `;
}

// ── Page init ──

function setBusy(b) {
  if (btnImport) btnImport.disabled = b;
  if (btnParse) btnParse.disabled = b;
  if (btnExport) btnExport.disabled = b;
  if (btnScore) btnScore.disabled = b;
  if (btnClear) btnClear.disabled = b;
}

export function initJdPage({ onViewDetail } = {}) {
  // btnNewJd
  btnNewJd?.addEventListener("click", async () => {
    const title = window.prompt("请输入 JD 标题：");
    if (!title) return;
    const text = jdInput?.value?.trim() || window.prompt("请输入 JD 内容：") || "";
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

  // btnScore
  btnScore?.addEventListener("click", async () => {
    const listen = window.__TAURI__?.event?.listen;
    let unlisten = null;
    let filterT0 = null;
    try {
      setBusy(true);
      if (jdFilterProgressWrap) jdFilterProgressWrap.style.display = "block";
      if (jdFilterProgressPhase) jdFilterProgressPhase.textContent = "准备中…";
      if (jdFilterProgressFrac) jdFilterProgressFrac.textContent = "—";
      if (jdFilterProgressBar) {
        jdFilterProgressBar.style.width = "0%";
        jdFilterProgressBar.classList.remove("is-error");
      }
      if (typeof listen === "function") {
        unlisten = await listen("jd-filter-progress", (ev) => {
          const p = ev?.payload != null ? ev.payload : ev;
          const cur = Number(p?.current ?? 0);
          const tot = Number(p?.total ?? 0);
          const msg = p?.message || "";
          const done = Boolean(p?.done);
          if (jdFilterProgressPhase) jdFilterProgressPhase.textContent = msg;
          if (tot > 0) {
            if (jdFilterProgressFrac) jdFilterProgressFrac.textContent = `${cur} / ${tot}`;
            const pct = Math.min(100, Math.round((cur / tot) * 100));
            if (jdFilterProgressBar) jdFilterProgressBar.style.width = `${pct}%`;
          } else {
            if (jdFilterProgressFrac) jdFilterProgressFrac.textContent = "…";
            if (jdFilterProgressBar) jdFilterProgressBar.style.width = "6%";
          }
          if (done && jdFilterProgressBar) jdFilterProgressBar.style.width = "100%";
        });
      }
      const position = (jdPositionInput?.value || "").trim();
      const jd = jdInput?.value?.trim() || "";
      if (!jd) {
        alert("请粘贴 JD 文本。");
        return;
      }
      if (!position) {
        alert("请先输入岗位。");
        return;
      }

      const minScore = Math.max(0, Math.min(100, Number(jdMinScore?.value) || 0));
      filterT0 = Date.now();
      const rows = await invoke("jd_filter_by_keywords", {
        position,
        jdText: jd,
        limit: 200,
        rerankPool: 200,
      });
      const elapsedMs = Date.now() - filterT0;
      const timePart = `用时 ${formatJdFilterElapsed(elapsedMs)}`;

      if (!rows.length) {
        lastJdMatchCount.value = 0;
        lastJdRankingRows.length = 0;
        renderTemplateCandidatePicker([]);
        if (scoreOut) scoreOut.textContent = `已筛选 0 份（不低于 ${minScore} 分）· ${timePart}`;
        renderJdRanking([]);
        updateStats();
        return;
      }

      lastJdMatchCount.value = rows.length;
      lastJdRankingRows.length = 0;
      lastJdRankingRows.push(...rows);
      const filtered = getCurrentTemplateRows();
      renderTemplateCandidatePicker(filtered);
      if (scoreOut) {
        scoreOut.textContent = `已筛选 ${filtered.length} / ${rows.length} 份（不低于 ${minScore} 分）· ${timePart}`;
      }
      renderJdRanking(rows);
      renderJdCompareReport(rows);
      updateStats();
    } catch (e) {
      if (scoreOut && filterT0 != null) {
        const elapsedMs = Date.now() - filterT0;
        scoreOut.textContent = `筛选失败 · 用时 ${formatJdFilterElapsed(elapsedMs)}`;
      }
      alert(String(e));
    } finally {
      if (typeof unlisten === "function") unlisten();
      setBusy(false);
      if (jdFilterProgressWrap) jdFilterProgressWrap.style.display = "none";
      if (jdFilterProgressBar) jdFilterProgressBar.style.width = "0%";
    }
  });

  // Template regenerate & export
  tplRegenerate?.addEventListener("click", updateTemplatePreview);
  tplExportPdf?.addEventListener("click", async () => {
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
        const options = getTemplateExportOptions();
        const jsonPath = String(item.row?.jsonPath || "").trim();
        try {
          if (jsonPath) {
            await invoke("export_resume_pdf_from_json", { jsonPath, outPath: targetPath, options });
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

  // Compare report
  jdCompareBtn?.addEventListener("click", () => {
    renderJdCompareReport(lastJdRankingRows);
    jumpToStandalonePage("jdCompareReport");
  });

  // Change min score threshold
  const refreshJdViews = () => {
    renderJdRanking(lastJdRankingRows);
    renderTemplateCandidatePicker(getCurrentTemplateRows());
    renderJdCompareReport(lastJdRankingRows);
  };
  jdMinScore?.addEventListener("input", refreshJdViews);
  jdMinScore?.addEventListener("change", refreshJdViews);

  // Template option checkboxes
  ["tplName", "tplGender", "tplAge", "tplContact", "tplSkill"].forEach((id) => {
    $(id)?.addEventListener("change", updateTemplatePreview);
  });
}
