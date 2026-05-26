// ── 大瀚简历主题 — 简约型 ──
const WATERMARK_TEXT = "";
const WATERMARK_OPACITY = 0.02;
const WATERMARK_FONT_SIZE = 72;
const WATERMARK_ROTATE = -20;

// ── 工具函数 ──
function esc(v) {
  return String(v ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function clean(v) {
  return String(v ?? "").replace(/\s+/g, " ").trim();
}

function isEmpty(v) {
  return !v || !String(v).trim();
}

// ── 布局组件 ──
function section(title, body) {
  if (!body || !String(body).trim()) return "";
  return `<div class="sec">
    <h2>${esc(title)}</h2>
    ${body}
  </div>`;
}

function workItem(w) {
  const period = [w.startDate, w.endDate].filter(Boolean).join(" — ");
  const summary = w.summary ? clean(w.summary) : "";
  const highlights = (Array.isArray(w.highlights) ? w.highlights : [])
    .map((h) => clean(h))
    .filter(Boolean);
  return `<div class="item">
    <div class="item-row">
      <span class="item-name">${esc(w.name || "")}</span>
      <span class="item-meta">${esc(w.position || "")}${period ? `&nbsp;·&nbsp;${esc(period)}` : ""}</span>
    </div>
    ${summary ? `<div class="item-text">${esc(summary)}</div>` : ""}
    ${highlights.length ? `<ul>${highlights.map((h) => `<li>${esc(h)}</li>`).join("")}</ul>` : ""}
  </div>`;
}

function eduItem(e) {
  const period = [e.startDate, e.endDate].filter(Boolean).join(" — ");
  const detail = [e.studyType, e.area].filter(Boolean).join(" / ");
  return `<div class="item">
    <div class="item-row">
      <span class="item-name">${esc(e.institution || "")}</span>
      <span class="item-meta">${esc(detail)}${period ? `&nbsp;·&nbsp;${esc(period)}` : ""}</span>
    </div>
  </div>`;
}

function projItem(p) {
  const highlights = (Array.isArray(p.highlights) ? p.highlights : [])
    .map((h) => clean(h))
    .filter(Boolean);
  return `<div class="item">
    <div class="item-row">
      <span class="item-name">${esc(p.name || "")}</span>
    </div>
    ${p.description ? `<div class="item-text">${esc(clean(p.description))}</div>` : ""}
    ${highlights.length ? `<ul>${highlights.map((h) => `<li>${esc(h)}</li>`).join("")}</ul>` : ""}
  </div>`;
}

// ── 主题入口 ──
module.exports = {
  pdfRenderOptions: {
    format: "A4",
    printBackground: true,
    margin: { top: "16mm", right: "14mm", bottom: "14mm", left: "14mm" },
  },

  render(resume = {}) {
    const basics = resume.basics || {};
    const work = Array.isArray(resume.work) ? resume.work : [];
    const education = Array.isArray(resume.education) ? resume.education : [];
    const skills = Array.isArray(resume.skills) ? resume.skills : [];
    const projects = Array.isArray(resume.projects) ? resume.projects : [];
    const certificates = Array.isArray(resume.certificates) ? resume.certificates : [];

    const name = clean(basics.name) || "候选人";

    // 联系方式
    const contactItems = [
      basics.email, basics.phone, basics.xGender, basics.xAge,
    ].map(clean).filter(Boolean);
    const contactLine = contactItems.length ? `<div class="contact">${contactItems.map((c) => esc(c)).join("&nbsp;·&nbsp;")}</div>` : "";

    // 教育
    const eduBody = education.map(eduItem).join("");

    // 工作
    const workBody = work.map(workItem).join("");

    // 项目
    const projBody = projects.map(projItem).join("");

    // 技能
    const skillsBody = skills.length
      ? `<div class="tags">${skills.map((s) => {
          const n = typeof s === "string" ? s : (s.name || s.level ? `${s.name || ""} ${s.level || ""}` : "");
          return isEmpty(n) ? "" : `<span>${esc(n.trim())}</span>`;
        }).filter(Boolean).join("")}</div>`
      : "";

    // 证书
    const certBody = certificates.length
      ? `<div class="tags">${certificates.map((c) => {
          const n = typeof c === "string" ? c : (c.name || "");
          return isEmpty(n) ? "" : `<span>${esc(n)}</span>`;
        }).filter(Boolean).join("")}</div>`
      : "";

    return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <title>${esc(name)}</title>
  <style>
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: "Microsoft YaHei", "PingFang SC", "Noto Sans SC", sans-serif;
      font-size: 13px;
      color: #333;
      line-height: 1.7;
    }

    /* ── 头部 ── */
    h1 {
      font-size: 26px;
      font-weight: 600;
      letter-spacing: 3px;
      color: #1e293b;
      margin-bottom: 6px;
    }
    .contact {
      font-size: 12.5px;
      color: #666;
      margin-bottom: 4px;
    }
    .head-line {
      height: 2px;
      background: #2563eb;
      margin: 10px 0 18px;
    }

    /* ── 板块 ── */
    .sec { margin-bottom: 16px; }
    h2 {
      font-size: 15px;
      font-weight: 600;
      color: #2563eb;
      padding-bottom: 5px;
      border-bottom: 1px solid #e5e7eb;
      margin-bottom: 10px;
    }

    /* ── 条目 ── */
    .item {
      margin-bottom: 10px;
      break-inside: avoid;
    }
    .item-row {
      display: flex;
      justify-content: space-between;
      align-items: baseline;
      flex-wrap: wrap;
      gap: 6px 12px;
    }
    .item-name {
      font-size: 13.5px;
      font-weight: 600;
      color: #1e293b;
    }
    .item-meta {
      font-size: 11.5px;
      color: #888;
      white-space: nowrap;
    }
    .item-text {
      font-size: 12.5px;
      color: #555;
      margin-top: 3px;
    }

    /* ── 列表 ── */
    ul {
      margin: 4px 0 0 16px;
      padding: 0;
    }
    li {
      font-size: 12.5px;
      color: #555;
      line-height: 1.7;
    }
    li::marker { color: #93c5fd; }

    /* ── 标签 ── */
    .tags {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }
    .tags span {
      font-size: 11.5px;
      color: #475569;
      background: #f1f5f9;
      padding: 2px 10px;
      border-radius: 3px;
    }

    /* ── 打印 ── */
    @media print {
      body { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
    }
  </style>
</head>
<body>

<h1>${esc(name)}</h1>
${contactLine}
<div class="head-line"></div>

${section("教育背景", eduBody)}
${section("工作经历", workBody)}
${section("项目经历", projBody)}
${section("技能特长", skillsBody)}
${certBody ? section("证书资质", certBody) : ""}

</body></html>`;
  },
};
