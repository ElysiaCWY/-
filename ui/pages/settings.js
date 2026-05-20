import {
  appSettings,
  aiLlmProvider,
  aiModelPath,
  aiLlamaCliPath,
  aiLlmApiKey,
  aiThreads,
  aiTemperature,
  aiCloudMaxOutputTokens,
  aiDisableThinking,
  aiSettingsPathHint,
  aiSettingsSave,
  aiSettingsReload,
} from "../core/state.js";
import { invoke } from "../core/api.js";

function syncAiFormFromAppSettings() {
  const prov = String(appSettings.llm_provider || "ollama").toLowerCase();
  if (aiLlmProvider) {
    const opts = [...aiLlmProvider.options].map((o) => o.value);
    aiLlmProvider.value = opts.includes(prov) ? prov : "ollama";
  }
  if (aiModelPath) aiModelPath.value = appSettings.model_path || "";
  if (aiLlamaCliPath) aiLlamaCliPath.value = appSettings.llama_cli_path || "";
  if (aiLlmApiKey) aiLlmApiKey.value = appSettings.llm_api_key || "";
  if (aiThreads) aiThreads.value = String(appSettings.threads ?? 4);
  if (aiTemperature) aiTemperature.value = String(appSettings.temperature ?? 0.1);
  if (aiCloudMaxOutputTokens) {
    const v = appSettings.cloud_max_output_tokens;
    aiCloudMaxOutputTokens.value = v != null && Number.isFinite(Number(v)) ? String(v) : "";
  }
  if (aiDisableThinking) aiDisableThinking.checked = !!appSettings.disable_thinking;
}

function readAiSettingsFromForm() {
  const cloudRaw = (aiCloudMaxOutputTokens?.value || "").trim();
  let cloudMaxOutputTokens = null;
  if (cloudRaw) {
    const n = parseInt(cloudRaw, 10);
    if (Number.isFinite(n) && n >= 2048) cloudMaxOutputTokens = Math.min(n, 65536);
  }
  const threads = parseInt(aiThreads?.value || "4", 10);
  const temp = Number(aiTemperature?.value);
  return {
    llmProvider: ((aiLlmProvider?.value || "ollama").trim().toLowerCase() || "ollama"),
    modelPath: (aiModelPath?.value || "").trim(),
    llamaCliPath: (aiLlamaCliPath?.value || "").trim(),
    llmApiKey: (aiLlmApiKey?.value || "").trim(),
    threads: Math.min(64, Math.max(1, Number.isFinite(threads) ? threads : 4)),
    temperature: Math.min(2, Math.max(0, Number.isFinite(temp) ? temp : 0.1)),
    cloudMaxOutputTokens,
    disableThinking: !!aiDisableThinking?.checked,
  };
}

export async function loadSettingsFromFile() {
  try {
    const s = await invoke("load_app_settings");
    appSettings.llama_cli_path = (s.llamaCliPath || "").trim();
    appSettings.model_path = (s.modelPath || "").trim();
    appSettings.threads = Number(s.threads ?? 4);
    appSettings.temperature = Number(s.temperature ?? 0.1);
    appSettings.llm_provider = String(s.llmProvider || "ollama").trim().toLowerCase() || "ollama";
    appSettings.llm_api_key = String(s.llmApiKey ?? s.llm_api_key ?? "").trim();
    const v = s.cloudMaxOutputTokens ?? s.cloud_max_output_tokens;
    if (v == null || v === "") {
      appSettings.cloud_max_output_tokens = null;
    } else {
      const n = Number(v);
      if (!Number.isFinite(n) || n < 2048) {
        appSettings.cloud_max_output_tokens = null;
      } else {
        appSettings.cloud_max_output_tokens = Math.min(Math.floor(n), 65536);
      }
    }
    appSettings.disable_thinking = !!s.disableThinking;
  } catch (e) {
    appSettings.llama_cli_path = "";
    appSettings.model_path = "";
    appSettings.threads = 4;
    appSettings.temperature = 0.1;
    appSettings.llm_provider = "ollama";
    appSettings.llm_api_key = "";
    appSettings.cloud_max_output_tokens = null;
    appSettings.disable_thinking = false;
    alert(`加载配置失败：${String(e)}\n请检查项目根目录 app-config.json`);
  }
  syncAiFormFromAppSettings();
}

async function refreshAiSettingsPanel() {
  syncAiFormFromAppSettings();
  try {
    const p = await invoke("get_app_settings_path");
    if (aiSettingsPathHint) aiSettingsPathHint.textContent = p;
  } catch (e) {
    if (aiSettingsPathHint) aiSettingsPathHint.textContent = `无法解析配置路径：${String(e)}`;
  }
}

export function initSettingsPage() {
  aiSettingsSave?.addEventListener("click", async () => {
    try {
      const s = readAiSettingsFromForm();
      await invoke("save_app_settings", { settings: s });
      await loadSettingsFromFile();
      const p = await invoke("get_app_settings_path");
      if (aiSettingsPathHint) aiSettingsPathHint.textContent = p;
      alert(`已保存到：\n${p}`);
    } catch (e) {
      alert(String(e));
    }
  });
  aiSettingsReload?.addEventListener("click", async () => {
    try {
      await loadSettingsFromFile();
      await refreshAiSettingsPanel();
    } catch (e) {
      alert(String(e));
    }
  });
}
