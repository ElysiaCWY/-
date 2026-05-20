import { tokenStatsTotal, tokenStatsDaily, tokenStatsModel, tokenStatsDays } from "../core/state.js";
import { invoke } from "../core/api.js";
import { escapeHtml } from "../core/utils.js";

function formatTokens(n) {
  if (n == null || !Number.isFinite(n)) return "0";
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(1) + "M";
  if (n >= 1_000) return (n / 1_000).toFixed(1) + "K";
  return String(n);
}

function formatCount(n) {
  return n != null && Number.isFinite(n) ? String(n) : "0";
}

export async function loadTokenStatsPage() {
  if (!tokenStatsTotal || !tokenStatsDaily) return;

  try {
    const s = await invoke("get_token_stats", { limit: 50 });
    if (s.callCount === 0) {
      tokenStatsTotal.innerHTML = '<span class="muted">暂无 Token 消耗记录。</span>';
      if (tokenStatsModel) tokenStatsModel.innerHTML = '<span class="muted">暂无数据。</span>';
      tokenStatsDaily.innerHTML = '<span class="muted">暂无数据。</span>';
      return;
    }
    const byProv = s.byProvider
      .map((p) => `${escapeHtml(p.provider)}: ${formatTokens(p.totalTokens)} (${p.callCount}次)`)
      .join(" &nbsp;|&nbsp; ");
    tokenStatsTotal.innerHTML = `
      <div class="stats" style="grid-template-columns:repeat(4,1fr);margin-bottom:12px">
        <div class="stat"><div class="k">总调用次数</div><div class="v" style="font-size:22px">${formatCount(s.callCount)}</div></div>
        <div class="stat"><div class="k">总 Token 消耗</div><div class="v" style="font-size:22px">${formatTokens(s.totalTokens)}</div></div>
        <div class="stat"><div class="k">输入 Token</div><div class="v" style="font-size:22px">${formatTokens(s.totalPromptTokens)}</div></div>
        <div class="stat"><div class="k">输出 Token</div><div class="v" style="font-size:22px">${formatTokens(s.totalCompletionTokens)}</div></div>
      </div>
      <div class="muted small">${byProv}</div>`;

    await loadModelStats();
    const days = parseInt(tokenStatsDays?.value || "30", 10);
    await loadDailyStats(days);
  } catch (e) {
    tokenStatsTotal.innerHTML = `<span class="muted">加载失败：${escapeHtml(String(e))}</span>`;
  }
}

async function loadModelStats() {
  if (!tokenStatsModel) return;
  try {
    const rows = await invoke("get_token_model_stats");
    if (!rows.length) {
      tokenStatsModel.innerHTML = '<span class="muted">暂无数据。</span>';
      return;
    }
    let maxTokens = 0;
    for (const r of rows) {
      if (r.totalTokens > maxTokens) maxTokens = r.totalTokens;
    }
    let html = '<table class="table"><thead><tr><th>模型</th><th>提供商</th><th style="text-align:right">调用次数</th><th style="text-align:right">Prompt</th><th style="text-align:right">Completion</th><th style="text-align:right">合计</th><th style="width:35%">占比</th></tr></thead><tbody>';
    for (const r of rows) {
      const pct = maxTokens > 0 ? (r.totalTokens / maxTokens * 100) : 0;
      const barColor = pct > 70 ? "#3b82f6" : pct > 30 ? "#93c5fd" : "#bfdbfe";
      html += `<tr>
        <td><strong>${escapeHtml(r.model)}</strong></td>
        <td>${escapeHtml(r.provider)}</td>
        <td style="text-align:right">${formatCount(r.callCount)}</td>
        <td style="text-align:right">${formatTokens(r.promptTokens)}</td>
        <td style="text-align:right">${formatTokens(r.completionTokens)}</td>
        <td style="text-align:right"><strong>${formatTokens(r.totalTokens)}</strong></td>
        <td><div style="height:16px;border-radius:8px;background:${barColor};width:${pct.toFixed(1)}%;min-width:2px;transition:width .2s"></div></td>
      </tr>`;
    }
    html += '</tbody></table>';
    tokenStatsModel.innerHTML = html;
  } catch (e) {
    tokenStatsModel.innerHTML = `<span class="muted">加载失败：${escapeHtml(String(e))}</span>`;
  }
}

async function loadDailyStats(days) {
  if (!tokenStatsDaily) return;
  try {
    const rows = await invoke("get_token_daily_stats", { days });
    if (!rows.length) {
      tokenStatsDaily.innerHTML = '<span class="muted">该时间段暂无 Token 消耗记录。</span>';
      return;
    }
    let maxTokens = 0;
    for (const r of rows) {
      if (r.totalTokens > maxTokens) maxTokens = r.totalTokens;
    }
    let html = '<table class="table"><thead><tr><th>日期</th><th>调用次数</th><th>Prompt</th><th>Completion</th><th>合计</th><th style="width:40%">占比</th></tr></thead><tbody>';
    for (const r of rows) {
      const pct = maxTokens > 0 ? (r.totalTokens / maxTokens * 100) : 0;
      const barColor = pct > 70 ? "#3b82f6" : pct > 30 ? "#93c5fd" : "#bfdbfe";
      html += `<tr>
        <td>${escapeHtml(r.date)}</td>
        <td style="text-align:right">${formatCount(r.callCount)}</td>
        <td style="text-align:right">${formatTokens(r.promptTokens)}</td>
        <td style="text-align:right">${formatTokens(r.completionTokens)}</td>
        <td style="text-align:right"><strong>${formatTokens(r.totalTokens)}</strong></td>
        <td><div style="height:16px;border-radius:8px;background:${barColor};width:${pct.toFixed(1)}%;min-width:2px;transition:width .2s"></div></td>
      </tr>`;
    }
    html += '</tbody></table>';
    tokenStatsDaily.innerHTML = html;
  } catch (e) {
    tokenStatsDaily.innerHTML = `<span class="muted">加载失败：${escapeHtml(String(e))}</span>`;
  }
}

export function initTokenStatsPage() {
  tokenStatsDays?.addEventListener("change", () => {
    const days = parseInt(tokenStatsDays.value || "30", 10);
    loadDailyStats(days);
  });
}
