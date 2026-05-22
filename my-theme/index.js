// ── 自定义中文简历主题 ──
// 水印开关：将 WATERMARK_TEXT 设为空字符串关闭，设为文字即可开启
const WATERMARK_TEXT = "内部资料";
const WATERMARK_OPACITY = 0.03;
const WATERMARK_FONT_SIZE = 64;
const WATERMARK_ROTATE = -28;

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

// 将 "1. xxx  2. yyy" 这类编号文本转为 <br> 分行
function formatNumbered(v) {
  const text = clean(v);
  if (!text) return "";
  const parts = text.split(/(?=(?:\d+[\.、．)）]|[①②③④⑤⑥⑦⑧⑨⑩]|[（(]\d+[)）])/);
  if (parts.length <= 1) return esc(text);
  return parts.map((s) => esc(s.trim())).filter(Boolean).join("<br>");
}

// ── 布局组件 ──
function infoRow(label, value) {
  if (isEmpty(value)) return "";
  return `<span class="info-item"><span class="info-k">${esc(label)}</span><span class="info-v">${esc(clean(value))}</span></span>`;
}

function section(title, body) {
  if (!body || !String(body).trim()) return "";
  return `<div class="sec">
    <h2>${esc(title)}</h2>
    <div class="sec-line"></div>
    ${body}
  </div>`;
}

function workItem(w) {
  const period = [w.startDate, w.endDate].filter(Boolean).join(" - ");
  const summary = w.summary ? formatNumbered(w.summary) : "";
  const highlights = (Array.isArray(w.highlights) ? w.highlights : [])
    .map((h) => formatNumbered(h))
    .filter(Boolean);
  return `<div class="item">
    <div class="item-head">
      <span class="item-title">${esc(w.name || "")}</span>
      ${period ? `<span class="item-period">${esc(period)}</span>` : ""}
    </div>
    ${w.position ? `<div class="item-sub">${esc(w.position)}</div>` : ""}
    ${summary ? `<div class="item-desc">${summary}</div>` : ""}
    ${highlights.length ? `<ul>${highlights.map((h) => `<li>${h}</li>`).join("")}</ul>` : ""}
  </div>`;
}

function eduItem(e) {
  const period = [e.startDate, e.endDate].filter(Boolean).join(" - ");
  return `<div class="item">
    <div class="item-head">
      <span class="item-title">${esc(e.institution || "")}</span>
      ${period ? `<span class="item-period">${esc(period)}</span>` : ""}
    </div>
    <div class="item-sub">${esc([e.studyType, e.area].filter(Boolean).join(" / "))}</div>
  </div>`;
}

function projItem(p) {
  const highlights = (Array.isArray(p.highlights) ? p.highlights : [])
    .map((h) => formatNumbered(h))
    .filter(Boolean);
  return `<div class="item">
    <div class="item-head">
      <span class="item-title">${esc(p.name || "")}</span>
    </div>
    ${p.description ? `<div class="item-desc">${formatNumbered(p.description)}</div>` : ""}
    ${highlights.length ? `<ul>${highlights.map((h) => `<li>${h}</li>`).join("")}</ul>` : ""}
  </div>`;
}

// ── 主题入口 ──
module.exports = {
  pdfRenderOptions: {
    format: "A4",
    printBackground: true,
    margin: { top: "14mm", right: "14mm", bottom: "14mm", left: "14mm" },
  },

  render(resume = {}) {
    const basics = resume.basics || {};
    const work = Array.isArray(resume.work) ? resume.work : [];
    const education = Array.isArray(resume.education) ? resume.education : [];
    const skills = Array.isArray(resume.skills) ? resume.skills : [];
    const projects = Array.isArray(resume.projects) ? resume.projects : [];
    const certificates = Array.isArray(resume.certificates) ? resume.certificates : [];

    // ── 基础信息行 ──
    const contactParts = [basics.phone, basics.email].map(clean).filter(Boolean);
    const infoRows = [
      infoRow("性别", basics.xGender),
      infoRow("年龄", basics.xAge),
      infoRow("联系方式", contactParts.join(" / ")),
    ].join("");

    // ── 教育 ──
    const eduBody = education.map(eduItem).join("");

    // ── 工作经历 ──
    const workBody = work.map(workItem).join("");

    // ── 项目 ──
    const projBody = projects.map(projItem).join("");

    // ── 技能 ──
    const skillsBody = skills.length
      ? `<div class="tags">${skills.map((s) => `<span class="tag">${esc(s.name || "")}</span>`).join("")}</div>`
      : "";

    // ── 证书 ──
    const certBody = certificates.length
      ? `<div class="tags">${certificates.map((c) => {
          const name = typeof c === "string" ? c : (c.name || "");
          return isEmpty(name) ? "" : `<span class="tag tag-cert">${esc(name)}</span>`;
        }).filter(Boolean).join("")}</div>`
      : "";

    // ── 水印 ──
    const watermarkStyle = WATERMARK_TEXT ? `
      body::after {
        content: "${WATERMARK_TEXT}";
        position: fixed;
        top: 50%; left: 50%;
        transform: translate(-50%, -50%) rotate(${WATERMARK_ROTATE}deg);
        font-size: ${WATERMARK_FONT_SIZE}px;
        color: rgba(0, 0, 0, ${WATERMARK_OPACITY});
        white-space: nowrap;
        pointer-events: none;
        z-index: 9999;
      }
    ` : "";

    return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <title>${esc(basics.name || "简历")}</title>
  <style>
    /* ── 全局 ── */
    * { margin: 0; padding: 0; box-sizing: border-box; }
    body {
      font-family: "Microsoft YaHei", "PingFang SC", "Noto Sans SC", "Source Han Sans CN", sans-serif;
      color: #1e293b;
      line-height: 1.6;
    }
    .wrap { padding: 0 2mm; }

    /* ── 姓名 ── */
    h1 {
      font-size: 30px;
      font-weight: 700;
      letter-spacing: 2px;
      margin: 0 0 8px 0;
    }

    /* ── 基础信息 ── */
    .info-row {
      display: flex;
      flex-wrap: wrap;
      gap: 2px 20px;
      margin-bottom: 4px;
    }
    .info-item {
      font-size: 13px;
      display: inline-flex;
      gap: 4px;
    }
    .info-k { color: #64748b; }
    .info-v { color: #334155; }

    /* ── 分割线 ── */
    .header-line {
      height: 2px;
      background: linear-gradient(90deg, #2563eb, #93c5fd);
      margin: 4px 0 10px 0;
    }

    /* ── 板块 ── */
    .sec {
      margin-top: 14px;
      break-inside: auto;
      page-break-inside: auto;
    }
    h2 {
      font-size: 17px;
      font-weight: 700;
      color: #1e40af;
      letter-spacing: 1px;
    }
    .sec-line {
      height: 1px;
      background: #cbd5e1;
      margin: 3px 0 8px 0;
    }

    /* ── 条目 ── */
    .item {
      margin-bottom: 10px;
      break-inside: avoid;
      page-break-inside: avoid;
    }
    .item-head {
      display: flex;
      justify-content: space-between;
      align-items: baseline;
      flex-wrap: wrap;
      gap: 8px;
    }
    .item-title {
      font-size: 15px;
      font-weight: 600;
    }
    .item-period {
      font-size: 12px;
      color: #64748b;
      white-space: nowrap;
    }
    .item-sub {
      font-size: 13px;
      color: #475569;
      margin-top: 1px;
    }
    .item-desc {
      font-size: 13px;
      line-height: 1.7;
      margin-top: 4px;
      color: #334155;
    }

    /* ── 列表 ── */
    ul {
      margin: 4px 0 0 18px;
      padding: 0;
      list-style-type: disc;
    }
    li {
      font-size: 13px;
      line-height: 1.7;
      color: #334155;
    }

    /* ── 技能/证书标签 ── */
    .tags {
      display: flex;
      flex-wrap: wrap;
      gap: 8px;
    }
    .tag {
      font-size: 12px;
      border: 1px solid #cbd5e1;
      padding: 3px 12px;
      border-radius: 14px;
      color: #334155;
      background: #f8fafc;
    }
    .tag-cert {
      border-color: #93c5fd;
      background: #eff6ff;
      color: #1e40af;
    }

    /* ── 技能板块不分页 ── */
    .sec-skill { break-inside: avoid-page; page-break-inside: avoid; }
    .sec-skill .sec-line { break-after: avoid-page; page-break-after: avoid; }
    .sec-skill .tags { break-inside: avoid-page; page-break-inside: avoid; }

    /* ── 水印 ── */
    ${watermarkStyle}
  </style>
</head>
<body>
<main class="wrap">

  <h1>${esc(basics.name || "候选人")}</h1>
  <div class="header-line"></div>
  <div class="info-row">${infoRows}</div>

  ${section("教育背景", eduBody)}
  ${section("工作经历", workBody)}
  ${section("项目经历", projBody)}
  ${section("技能特长", skillsBody)}
  ${certBody ? section("证书资质", certBody) : ""}

</main>
</body></html>`;
  },
};
