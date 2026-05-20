import {
  word2pdfInputDir,
  word2pdfOutputDir,
  word2pdfBrowseIn,
  word2pdfBrowseOut,
  word2pdfStart,
  word2pdfDefaults,
  word2pdfPhase,
  word2pdfFraction,
  word2pdfBar,
  word2pdfLog,
  word2pdfSummary,
} from "../core/state.js";
import { invoke, openDialog, invokeFn } from "../core/api.js";

export function initWord2PdfPage() {
  if (!word2pdfInputDir || !word2pdfOutputDir || !word2pdfStart || !word2pdfPhase || !word2pdfFraction || !word2pdfBar || !word2pdfLog || !word2pdfSummary) return;

  let running = false;

  async function fillDefaults() {
    if (!invokeFn) return;
    const [a, b] = await invoke("word_to_pdf_default_dirs");
    word2pdfInputDir.value = a || "";
    word2pdfOutputDir.value = b || "";
  }

  word2pdfBrowseIn?.addEventListener("click", async () => {
    try {
      const sel = await openDialog({ directory: true, multiple: false });
      if (sel && typeof sel === "string") word2pdfInputDir.value = sel;
    } catch (e) {
      alert(String(e));
    }
  });
  word2pdfBrowseOut?.addEventListener("click", async () => {
    try {
      const sel = await openDialog({ directory: true, multiple: false });
      if (sel && typeof sel === "string") word2pdfOutputDir.value = sel;
    } catch (e) {
      alert(String(e));
    }
  });

  word2pdfDefaults?.addEventListener("click", async () => {
    try {
      await fillDefaults();
    } catch (e) {
      alert(String(e));
    }
  });

  function appendLine(text) {
    const t = new Date().toLocaleTimeString();
    word2pdfLog.textContent = (word2pdfLog.textContent ? `${word2pdfLog.textContent}\n` : "") + `[${t}] ${text}`;
    word2pdfLog.scrollTop = word2pdfLog.scrollHeight;
  }

  word2pdfStart.addEventListener("click", async () => {
    if (running) return;
    const inputDir = word2pdfInputDir.value.trim();
    const outputDir = word2pdfOutputDir.value.trim();
    if (!inputDir || !outputDir) {
      alert("请先选择输入与输出文件夹，或点击「填入默认路径」。");
      return;
    }
    running = true;
    word2pdfStart.disabled = true;
    if (word2pdfDefaults) word2pdfDefaults.disabled = true;
    word2pdfPhase.textContent = "准备中…";
    word2pdfFraction.textContent = "0 / 0";
    word2pdfBar.style.width = "0%";
    word2pdfBar.classList.remove("is-error");
    word2pdfLog.textContent = "";
    word2pdfSummary.textContent = "";

    const listen = window.__TAURI__?.event?.listen;
    let unlisten = null;
    try {
      if (typeof listen === "function") {
        unlisten = await listen("word-to-pdf-progress", (ev) => {
          const p = ev?.payload != null ? ev.payload : ev;
          const t = p?.type;
          const total = Number(p?.total || 0);
          const idx = Number(p?.index || 0);
          const name = p?.name || "";
          const pct = total > 0 ? Math.min(100, Math.round((idx / total) * 100)) : 0;
          if (t === "skip" || t === "convert" || t === "ok" || t === "fail") {
            word2pdfFraction.textContent = `${idx} / ${total}`;
            word2pdfBar.style.width = `${pct}%`;
          }
          if (t === "convert") {
            word2pdfPhase.textContent = "正在转换";
            appendLine(`转换：${name}`);
          } else if (t === "skip") {
            word2pdfPhase.textContent = "跳过（已存在）";
            appendLine(`跳过：${name}`);
          } else if (t === "ok") {
            word2pdfPhase.textContent = "进行中";
            appendLine(`完成：${name}`);
          } else if (t === "fail") {
            word2pdfPhase.textContent = "有失败项";
            word2pdfBar.classList.add("is-error");
            appendLine(`失败：${name} — ${p?.error || ""}`);
          } else if (t === "done") {
            word2pdfPhase.textContent = "批次结束";
          }
        });
      } else {
        appendLine("（未检测到进度事件 API，仅显示最终结果）");
      }

      const sum = await invoke("word_to_pdf_convert", { inputDir, outputDir });
      word2pdfSummary.textContent = `转换 ${sum.converted} 个，跳过 ${sum.skipped} 个，失败 ${sum.failed} 个。输出目录：${sum.outputDir || outputDir}`;
      word2pdfPhase.textContent = "完成";
    } catch (e) {
      alert(String(e));
      word2pdfPhase.textContent = "出错";
      appendLine(String(e));
    } finally {
      running = false;
      word2pdfStart.disabled = false;
      if (word2pdfDefaults) word2pdfDefaults.disabled = false;
      if (typeof unlisten === "function") unlisten();
    }
  });
}
