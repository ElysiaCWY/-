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

function isOneDigitMarkerStart(text, i) {
  if (i < 0 || i + 1 >= text.length) return false;
  const d = text[i];
  const p = text[i + 1];
  if (!/[1-9]/.test(d)) return false;
  if (!/[、.．]/.test(p)) return false;
  // 只在前一个字符不是数字时认定为新编号，避免匹配年份/小数。
  if (i > 0 && /\d/.test(text[i - 1])) return false;
  return true;
}

function formatNumberedText(v) {
  const text = clean(v);
  if (!text) return "";

  const starts = [0];
  for (let i = 1; i < text.length; i += 1) {
    if (isOneDigitMarkerStart(text, i)) {
      starts.push(i);
    }
  }
  if (starts.length <= 1) return esc(text);

  const parts = [];
  for (let i = 0; i < starts.length; i += 1) {
    const s = starts[i];
    const e = i + 1 < starts.length ? starts[i + 1] : text.length;
    const seg = text.slice(s, e).trim();
    if (seg) parts.push(esc(seg));
  }
  return parts.join("<br>");
}

function row(label, value) {
  const text = String(value ?? "").trim();
  if (!text) return "";
  return `<div class="info-item"><span class="k">${esc(label)}</span><span class="v">${esc(text)}</span></div>`;
}

function section(title, inner) {
  if (!inner || !String(inner).trim()) return "";
  const slug = String(title || "")
    .toLowerCase()
    .replace(/[^a-z0-9\u4e00-\u9fa5]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return `<section class="sec sec-${slug}"><h2>${esc(title)}</h2><div class="line"></div>${inner}</section>`;
}

module.exports = {
  pdfRenderOptions: {
    format: "A4",
    printBackground: true,
    margin: { top: "14mm", right: "12mm", bottom: "14mm", left: "12mm" },
  },
  render(resume = {}) {
    const basics = resume.basics || {};
    const education = Array.isArray(resume.education) ? resume.education : [];
    const work = Array.isArray(resume.work) ? resume.work : [];
    const skills = Array.isArray(resume.skills) ? resume.skills : [];
    const projects = Array.isArray(resume.projects) ? resume.projects : [];

    const contact = [basics.phone, basics.email].map((x) => clean(x)).filter(Boolean).join(" / ");
    const baseInfo = [
      row("性别", basics.xGender || ""),
      row("年龄", basics.xAge || ""),
      row("联系方式", contact || basics.phone || basics.email || ""),
    ].join("");

    const eduHtml = education.map((e) => {
      const period = [e.startDate, e.endDate].filter(Boolean).join(" - ");
      return `<div class="item"><div class="head">${esc(e.institution || "")}</div>
        <div class="sub">${esc([e.studyType, e.area, period].filter(Boolean).join(" / "))}</div>
      </div>`;
    }).join("");

    const workHtml = work.map((w) => {
      const period = [w.startDate, w.endDate].filter(Boolean).join(" - ");
      const hs = Array.isArray(w.highlights) ? w.highlights : [];
      return `<div class="item">
        <div class="head">${esc(w.name || "")}</div>
        <div class="sub">${esc([w.position, period].filter(Boolean).join(" / "))}</div>
        ${w.summary ? `<div class="desc">${formatNumberedText(w.summary)}</div>` : ""}
        ${hs.length ? `<ul>${hs.map((x) => `<li>${formatNumberedText(x)}</li>`).join("")}</ul>` : ""}
      </div>`;
    }).join("");

    const skillsHtml = skills.length
      ? `<div class="tags">${skills.map((s) => `<span class="tag">${esc(s.name || "")}</span>`).join("")}</div>`
      : "";

    const projectHtml = projects.map((p) => {
      const hs = Array.isArray(p.highlights) ? p.highlights : [];
      return `<div class="item">
        <div class="head">${esc(p.name || "")}</div>
        ${p.description ? `<div class="desc">${formatNumberedText(p.description)}</div>` : ""}
        ${hs.length ? `<ul>${hs.map((x) => `<li>${formatNumberedText(x)}</li>`).join("")}</ul>` : ""}
      </div>`;
    }).join("");

    return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <title>${esc(basics.name || "简历")}</title>
  <style>
    body{font-family:"Microsoft YaHei","PingFang SC",Arial,sans-serif;color:#1f2937;margin:0}
    .wrap{padding:0 2mm}
    h1{font-size:32px;margin:0 0 6px}
    .line{height:1px;background:#9ca3af;margin:6px 0 10px}
    .sec{margin-top:12px}
    .sec{break-inside:auto;page-break-inside:auto}
    h2{font-size:22px;margin:0}
    .info-item{display:inline-flex;gap:6px;margin-right:18px;font-size:14px}
    .k{color:#6b7280}
    .item{margin:0 0 10px}
    .head{font-size:16px;font-weight:600}
    .sub{font-size:13px;color:#4b5563;margin-top:2px}
    .desc{font-size:13px;line-height:1.6;margin-top:4px}
    ul{margin:6px 0 0 20px;padding:0}
    li{font-size:13px;line-height:1.6}
    .tags{display:flex;flex-wrap:wrap;gap:8px}
    .tag{font-size:12px;border:1px solid #cbd5e1;padding:2px 8px;border-radius:12px}
    /* 防止“技能特长”板块在分页处被切开（标题在上一页、标签在下一页） */
    .sec-sec-技能特长{break-inside:avoid-page;page-break-inside:avoid}
    .sec-sec-技能特长 .line{break-after:avoid-page;page-break-after:avoid}
    .sec-sec-技能特长 .tags{break-inside:avoid-page;page-break-inside:avoid}
    .sec-sec-技能特长 .tag{break-inside:avoid;page-break-inside:avoid}
  </style>
</head>
<body><main class="wrap">
  <h1>${esc(basics.name || "候选人")}</h1>
  <div class="line"></div>
  ${baseInfo}
  ${section("教育背景", eduHtml)}
  ${section("工作经历", workHtml)}
  ${section("技能特长", skillsHtml)}
  ${section("项目经历", projectHtml)}
</main></body></html>`;
  },
};
