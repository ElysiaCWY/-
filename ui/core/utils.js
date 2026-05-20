export function $(id) {
  return document.getElementById(id);
}

export function $$(selector) {
  return [...document.querySelectorAll(selector)];
}

export function escapeHtml(v) {
  return String(v ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function formatDateFromEpoch(v) {
  const n = Number(v);
  if (!Number.isFinite(n) || n <= 0) return "-";
  const d = new Date(n * 1000);
  if (Number.isNaN(d.getTime())) return "-";
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

export function formatJdFilterElapsed(ms) {
  if (!Number.isFinite(ms) || ms < 0) return "—";
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} 秒`;
  const m = Math.floor(s / 60);
  const rs = Math.round(s - m * 60);
  return `${m} 分 ${rs} 秒`;
}

export function formatDurationMs(ms) {
  if (!Number.isFinite(ms) || ms < 0) return "-";
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = Math.round(ms / 100) / 10;
  if (totalSeconds < 60) return `${totalSeconds.toFixed(1)}s`;
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = (totalSeconds % 60).toFixed(1).padStart(4, "0");
  return `${minutes}m ${seconds}s`;
}

export function parsePeriodStart(period) {
  const text = String(period || "").trim();
  if (!text) return null;

  let m = text.match(/(\d{4})\s*[.\-/年]\s*(\d{1,2})/);
  if (m) {
    const y = Number(m[1]);
    const mon = Number(m[2]);
    if (Number.isFinite(y) && Number.isFinite(mon) && mon >= 1 && mon <= 12) {
      return { y, mon };
    }
  }

  m = text.match(/(\d{4})(\d{2})/);
  if (m) {
    const y = Number(m[1]);
    const mon = Number(m[2]);
    if (Number.isFinite(y) && Number.isFinite(mon) && mon >= 1 && mon <= 12) {
      return { y, mon };
    }
  }

  m = text.match(/(\d{4})/);
  if (m) {
    const y = Number(m[1]);
    if (Number.isFinite(y)) {
      return { y, mon: 1 };
    }
  }

  return null;
}

export function calcWorkYears(workExp) {
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

export function sanitizeFileName(name) {
  const cleaned = String(name || "")
    .replace(/[\\/:*?"<>|]/g, "_")
    .replace(/\s+/g, " ")
    .trim();
  return cleaned || "候选人";
}

export function splitPathForBatch(path) {
  const raw = String(path || "");
  const slashIdx = raw.lastIndexOf("/");
  const backslashIdx = raw.lastIndexOf("\\");
  const idx = Math.max(slashIdx, backslashIdx);
  if (idx < 0) return { dirPath: "", sep: "\\" };
  const sep = idx === backslashIdx ? "\\" : "/";
  return { dirPath: raw.slice(0, idx), sep };
}

export function cleanInline(v) {
  return String(v ?? "").replace(/\s+/g, " ").trim();
}

export function rowKeyForTemplate(row) {
  return String(row.resumeId || row.parsedId || row.sourceFile || row.candidateName || "").trim();
}

export function countImportedToday(records) {
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
