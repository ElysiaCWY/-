const tauri = window.__TAURI__ || {};
export const invokeFn = tauri?.tauri?.invoke || tauri?.core?.invoke || tauri?.invoke || null;
const openFn = tauri?.dialog?.open || null;
const saveFn = tauri?.dialog?.save || null;

function ensureDesktopApi(name, fn) {
  if (typeof fn === "function") return fn;
  throw new Error(`Tauri API 不可用：${name}。请使用 npm run tauri:dev 启动应用。`);
}

export async function invoke(command, args = {}) {
  return ensureDesktopApi("invoke", invokeFn)(command, args);
}

export async function openDialog(options = {}) {
  return ensureDesktopApi("dialog.open", openFn)(options);
}

export async function saveDialog(options = {}) {
  return ensureDesktopApi("dialog.save", saveFn)(options);
}

/**
 * 同时输出到控制台并追加到本地 app.log。
 */
export function appLog(level, ...parts) {
  const msg = parts
    .map((x) => (typeof x === "string" ? x : JSON.stringify(x)))
    .join(" ");
  const fn = level === "error" ? console.error : level === "warn" ? console.warn : console.log;
  fn(`[app] ${msg}`);
  if (!invokeFn) return;
  const lvl = ["error", "warn", "debug", "info"].includes(level) ? level : "info";
  invoke("append_app_log", { level: lvl, message: msg }).catch(() => {});
}

if (invokeFn) {
  window.appLog = appLog;
  invoke("get_app_log_path")
    .then((p) => console.info("[resume-manager] 测试日志文件:", p))
    .catch(() => {});
  appLog("info", "前端已加载");
}
