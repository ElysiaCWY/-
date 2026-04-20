use crate::errors::AppError;
use crate::storage;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

static APP_LOG_MUTEX: Mutex<()> = Mutex::new(());

fn logs_dir() -> Result<PathBuf, AppError> {
  let dir = storage::project_root_dir()?.join("logs");
  if !dir.exists() {
    fs::create_dir_all(&dir)?;
  }
  Ok(dir)
}

/// 应用级文本日志（与 env_logger 控制台输出独立，便于测试时持久化前端/步骤记录）
/// 路径：`<项目根>/logs/app.log`（项目根与 `app-config.json` 所在目录一致）
pub fn log_file_path() -> Result<PathBuf, AppError> {
  Ok(logs_dir()?.join("app.log"))
}

pub fn append_line(level: &str, message: &str) -> Result<(), AppError> {
  let path = log_file_path()?;
  let ts = OffsetDateTime::now_utc()
    .format(&Rfc3339)
    .map_err(|e| AppError::msg(format!("时间格式化失败：{}", e)))?;
  let line = format!("[{}] [{}] {}\n", ts, level, message.replace('\n', " "));
  let _guard = APP_LOG_MUTEX
    .lock()
    .map_err(|e| AppError::msg(format!("日志锁 poison：{}", e)))?;
  let mut f = OpenOptions::new()
    .create(true)
    .append(true)
    .open(&path)
    .map_err(|e| AppError::msg(format!("打开日志文件失败：{}", e)))?;
  f.write_all(line.as_bytes())
    .map_err(|e| AppError::msg(format!("写入日志失败：{}", e)))?;
  Ok(())
}

/// 简历解析专用：同时写 `log`（控制台，受 `RUST_LOG` 控制）与 `<项目根>/logs/app.log`。
/// 用法：`resume_parse_log!(info, "resume_parse: ... {}", x);` 第二参数起与 `format!` 相同。
/// 通过 `main.rs` 中 `#[macro_use] mod app_log` 对后续模块可见。
macro_rules! resume_parse_log {
  (error, $($arg:tt)*) => {{
    let __m = format!($($arg)*);
    log::error!("{}", __m);
    let _ = $crate::app_log::append_line("ERROR", &__m);
  }};
  (warn, $($arg:tt)*) => {{
    let __m = format!($($arg)*);
    log::warn!("{}", __m);
    let _ = $crate::app_log::append_line("WARN", &__m);
  }};
  (info, $($arg:tt)*) => {{
    let __m = format!($($arg)*);
    log::info!("{}", __m);
    let _ = $crate::app_log::append_line("INFO", &__m);
  }};
  (debug, $($arg:tt)*) => {{
    let __m = format!($($arg)*);
    log::debug!("{}", __m);
    let _ = $crate::app_log::append_line("DEBUG", &__m);
  }};
}
