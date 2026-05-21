import {
  libraryRecords,
  filteredLibraryRecords,
  selectedLibraryIds,
  resumeDetailBackTarget,
  updateStats,
  libraryKeyword,
  libraryDegree,
  libraryYears,
  librarySkill,
  libraryFilterBtn,
  libraryResetBtn,
  libraryTableBody,
  libraryDeleteSelectedBtn,
  librarySelectAll,
  libraryTotalCount,
  recentList,
  resumeDetailContent,
  resumeDetailBasic,
  resumeDetailEduBody,
  resumeDetailWorkBody,
  resumeDetailProjectBody,
  resumeDetailJdSummary,
  basicInfoCard,
  skillsTags,
} from "../core/state.js";
import { invoke, invokeFn } from "../core/api.js";
import {
  $$,
  escapeHtml,
  formatDateFromEpoch,
  calcWorkYears,
  parsePeriodStart,
} from "../core/utils.js";
import { jumpToStandalonePage } from "../core/router.js";

export const libraryCallbacks = { onRefresh: null };

export function updateLibraryBulkDeleteState() {
  if (!libraryDeleteSelectedBtn) return;
  const selectedCount = selectedLibraryIds.size;
  libraryDeleteSelectedBtn.disabled = selectedCount === 0;
  libraryDeleteSelectedBtn.textContent = selectedCount > 0 ? `批量删除（${selectedCount}）` : "批量删除";
}

export function updateLibraryTotalCount() {
  if (libraryTotalCount) libraryTotalCount.textContent = String(libraryRecords.length);
}

export function syncLibrarySelectAllState(records = filteredLibraryRecords) {
  if (!librarySelectAll) return;
  const rows = records.filter((r) => !!String(r?.id || "").trim());
  const selectedCount = rows.filter((r) => selectedLibraryIds.has(String(r.id))).length;
  librarySelectAll.checked = rows.length > 0 && selectedCount === rows.length;
  librarySelectAll.indeterminate = selectedCount > 0 && selectedCount < rows.length;
}

export async function handleDeleteSelectedResumes() {
  const ids = Array.from(selectedLibraryIds);
  if (!ids.length) {
    alert("请先勾选要删除的简历。");
    return;
  }
  try {
    await invoke("delete_resume_records", { ids });
    selectedLibraryIds.clear();
    await refreshLibraryAndStats();
  } catch (e) {
    alert(String(e));
  }
}

export function renderLibraryTable(records) {
  if (!records.length) {
    libraryTableBody.innerHTML = '<tr><td colspan="11" class="muted">暂无简历数据</td></tr>';
    syncLibrarySelectAllState(records);
    updateLibraryBulkDeleteState();
    return;
  }
  libraryTableBody.innerHTML = records
    .map((r) => {
      const b = r.data?.basicInfo || {};
      const edu = b.education?.[0]?.degree || "";
      const latestWork = r.data?.workExperience?.["1"] || {};
      const workYears = calcWorkYears(r.data?.workExperience || {});
      const importedDate = formatDateFromEpoch(r.createdAt || r.created_at || "");
      const checked = selectedLibraryIds.has(String(r.id)) ? "checked" : "";
      const fileName = r.fileName || r.sourceFile?.split(/[/\\]/).pop() || "-";
      return `<tr>
        <td><input type="checkbox" class="library-row-check" data-id="${r.id}" ${checked} /></td>
        <td title="${escapeHtml(r.sourceFile || '')}">${escapeHtml(fileName)}</td>
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

  $$(".library-row-check").forEach((input) => {
    input.addEventListener("change", () => {
      const id = String(input.dataset.id || "").trim();
      if (!id) return;
      if (input.checked) selectedLibraryIds.add(id);
      else selectedLibraryIds.delete(id);
      syncLibrarySelectAllState(records);
      updateLibraryBulkDeleteState();
    });
  });

  $$(".library-detail-btn").forEach((btn) => {
    btn.addEventListener("click", async () => {
      const id = btn.dataset.id;
      const rec = libraryRecords.find((x) => x.id === id);
      if (!rec) return;
      resumeDetailBackTarget.value = "library";
      await renderResumeDetailPage(rec);
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
        selectedLibraryIds.delete(String(id));
        await refreshLibraryAndStats();
      } catch (e) {
        alert(String(e));
      }
    });
  });
  syncLibrarySelectAllState(records);
  updateLibraryBulkDeleteState();
}

export function applyLibraryFilters() {
  const keyword = (libraryKeyword?.value || "").trim().toLowerCase();
  const degree = (libraryDegree?.value || "").trim().toLowerCase();
  const skill = (librarySkill?.value || "").trim().toLowerCase();

  const filtered = libraryRecords.filter((r) => {
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
  filteredLibraryRecords.length = 0;
  filteredLibraryRecords.push(...filtered);
  const visibleIds = new Set(filteredLibraryRecords.map((r) => String(r.id)));
  for (const id of [...selectedLibraryIds]) {
    if (!visibleIds.has(id)) selectedLibraryIds.delete(id);
  }
  renderLibraryTable(filteredLibraryRecords);
  updateLibraryTotalCount();
}

export function renderRecent(records) {
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

export function formatJdScreeningIndexHtml(idx) {
  if (idx == null) {
    return '<p class="muted">暂无解析阶段生成的 JD 筛选索引（请先完成简历解析入库，或索引文件已缺失）。</p>';
  }
  const summary = String(idx.summaryForJd ?? "").trim();
  const skills = idx.skillTags ?? [];
  const roles = idx.roleTags ?? [];
  const domains = idx.domainTags ?? [];
  const workBullets = String(idx.workBullets ?? "").trim();
  const projectBullets = String(idx.projectBullets ?? "").trim();

  const hasTags = (arr) => Array.isArray(arr) && arr.some((s) => String(s).trim());
  if (
    !summary &&
    !hasTags(skills) &&
    !hasTags(roles) &&
    !hasTags(domains) &&
    !workBullets &&
    !projectBullets
  ) {
    return '<p class="muted">索引存在但各字段为空（可能为旧版解析结果）。</p>';
  }

  const tagRow = (label, arr) => {
    if (!hasTags(arr)) return "";
    const chips = arr
      .map((t) => String(t).trim())
      .filter(Boolean)
      .map((t) => `<span class="tag">${escapeHtml(t)}</span>`)
      .join("");
    return `<div class="detail-item" style="grid-column: 1 / -1"><span class="detail-k">${escapeHtml(label)}</span><div class="tags detail-v">${chips}</div></div>`;
  };

  const bulletBlock = (label, text) => {
    if (!text) return "";
    return `<section style="margin-top:0.75rem"><h3 class="muted" style="font-size:0.95em;margin:0 0 0.35rem">${escapeHtml(
      label
    )}</h3><div class="jd-summary-pre">${escapeHtml(text)}</div></section>`;
  };

  const summaryBlock = summary
    ? `<section><h3 class="muted" style="font-size:0.95em;margin:0 0 0.35rem">职业摘要</h3><p style="margin:0;line-height:1.55;white-space:pre-wrap">${escapeHtml(
        summary
      )}</p></section>`
    : "";

  const tagRows = [tagRow("技能标签", skills), tagRow("岗位标签", roles), tagRow("领域标签", domains)].join("");
  const gridBlock = tagRows.trim()
    ? `<div class="detail-grid" style="margin-top:0.75rem">${tagRows}</div>`
    : "";

  return `${summaryBlock}
    ${gridBlock}
    ${bulletBlock("工作要点", workBullets)}
    ${bulletBlock("项目要点", projectBullets)}`.trim();
}

export async function renderResumeDetailPage(rec) {
  if (!resumeDetailContent || !resumeDetailBasic || !resumeDetailEduBody || !resumeDetailWorkBody || !resumeDetailProjectBody) return;

  const b = rec?.data?.basicInfo || {};
  const edu = b.education?.[0] || {};
  const work = rec?.data?.workExperience || {};
  const proj = rec?.data?.projectExperience || {};

  const fileName = rec.fileName || (rec.sourceFile || "").split(/[/\\]/).pop() || "-";
  const basicItems = [
    ["简历文件", fileName],
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

  const eduList = Array.isArray(b.education) ? b.education : [];
  if (!eduList.length) {
    resumeDetailEduBody.innerHTML = '<tr><td colspan="4" class="muted">暂无教育经历</td></tr>';
  } else {
    resumeDetailEduBody.innerHTML = eduList
      .map((e) => `<tr>
        <td>${escapeHtml(e.period || "-")}</td>
        <td>${escapeHtml(e.school || "-")}</td>
        <td>${escapeHtml(e.major || "-")}</td>
        <td>${escapeHtml(e.degree || "-")}</td>
      </tr>`)
      .join("");
  }

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

  if (resumeDetailJdSummary) {
    resumeDetailJdSummary.innerHTML = '<p class="muted">加载中…</p>';
    const id = String(rec?.id || "").trim();
    if (!invokeFn) {
      resumeDetailJdSummary.innerHTML = '<p class="muted">当前环境无法调用后端，无法加载 JD 筛选索引。</p>';
    } else if (!id) {
      resumeDetailJdSummary.innerHTML = '<p class="muted">无简历 ID，无法加载 JD 筛选索引。</p>';
    } else {
      try {
        const idx = await invoke("get_jd_screening_index_for_resume", { resumeId: id });
        resumeDetailJdSummary.innerHTML = formatJdScreeningIndexHtml(idx);
      } catch (e) {
        resumeDetailJdSummary.innerHTML = `<p class="muted">加载失败：${escapeHtml(String(e))}</p>`;
      }
    }
  }
}

export function renderStructuredCards(resume) {
  const b = resume?.basicInfo || {};
  if (basicInfoCard) {
    basicInfoCard.textContent = [
      `姓名：${b.name || ""}`,
      `年龄：${b.age || ""}`,
      `性别：${b.gender || ""}`,
      `学历：${b.education?.[0]?.degree || ""}`,
    ].join(" | ");
  }

  if (skillsTags) {
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
}

export async function refreshLibraryAndStats() {
  libraryRecords.length = 0;
  const records = await invoke("list_resume_library");
  libraryRecords.push(...records);
  filteredLibraryRecords.length = 0;
  filteredLibraryRecords.push(...libraryRecords);
  const validIds = new Set(libraryRecords.map((r) => String(r.id)));
  for (const id of [...selectedLibraryIds]) {
    if (!validIds.has(id)) selectedLibraryIds.delete(id);
  }
  renderLibraryTable(filteredLibraryRecords);
  renderRecent(libraryRecords);
  updateLibraryTotalCount();
  updateStats();
  if (libraryCallbacks.onRefresh) libraryCallbacks.onRefresh();
}

export function initLibraryPage() {
  libraryFilterBtn?.addEventListener("click", applyLibraryFilters);
  libraryResetBtn?.addEventListener("click", () => {
    if (libraryKeyword) libraryKeyword.value = "";
    if (libraryDegree) libraryDegree.value = "";
    if (libraryYears) libraryYears.value = "";
    if (librarySkill) librarySkill.value = "";
    filteredLibraryRecords.length = 0;
    filteredLibraryRecords.push(...libraryRecords);
    selectedLibraryIds.clear();
    renderLibraryTable(filteredLibraryRecords);
  });
  libraryDeleteSelectedBtn?.addEventListener("click", handleDeleteSelectedResumes);
  librarySelectAll?.addEventListener("change", () => {
    const currentRows = filteredLibraryRecords.filter((r) => !!String(r?.id || "").trim());
    if (librarySelectAll.checked) {
      currentRows.forEach((r) => selectedLibraryIds.add(String(r.id)));
    } else {
      currentRows.forEach((r) => selectedLibraryIds.delete(String(r.id)));
    }
    renderLibraryTable(filteredLibraryRecords);
  });
}
