//! Windows：通过 PowerShell 调用 Word COM，将 `.doc` / `.docx` 转为 PDF（需安装 Microsoft Word）。

use crate::errors::AppError;
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::Manager;

const BATCH_PS1: &str = include_str!("word_pdf_batch.ps1");

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct WordToPdfSummary {
  pub converted: u32,
  pub skipped: u32,
  pub failed: u32,
  pub output_dir: String,
}

/// 默认与旧脚本一致：`<项目根>/test/word` → `<项目根>/test/pdf`
pub fn default_input_output(project_root: &Path) -> (PathBuf, PathBuf) {
  (
    project_root.join("test").join("word"),
    project_root.join("test").join("pdf"),
  )
}

fn collect_word_files(input_dir: &Path) -> Result<Vec<PathBuf>, AppError> {
  if !input_dir.is_dir() {
    return Err(AppError::msg(format!(
      "输入目录不存在或不是文件夹：{}",
      input_dir.display()
    )));
  }
  let mut v: Vec<PathBuf> = fs::read_dir(input_dir)?
    .filter_map(|e| e.ok())
    .map(|e| e.path())
    .filter(|p| {
      p.is_file()
        && matches!(
          p.extension()
            .and_then(|x| x.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
          Some("doc") | Some("docx")
        )
    })
    .collect();
  v.sort();
  Ok(v)
}

fn write_batch_json(paths: &[(PathBuf, PathBuf)]) -> Result<PathBuf, AppError> {
  let dir = std::env::temp_dir().join("resume-manager");
  fs::create_dir_all(&dir)?;
  let json_path = dir.join("word_to_pdf_batch.json");
  let items: Vec<_> = paths
    .iter()
    .map(|(src, dst)| {
      json!({
        "src": src.to_string_lossy(),
        "dst": dst.to_string_lossy(),
      })
    })
    .collect();
  fs::write(&json_path, serde_json::to_string_pretty(&items)?)?;
  Ok(json_path)
}

fn write_ps1_temp() -> Result<PathBuf, AppError> {
  let dir = std::env::temp_dir().join("resume-manager");
  fs::create_dir_all(&dir)?;
  let p = dir.join("word_pdf_batch.ps1");
  fs::write(&p, BATCH_PS1.as_bytes())?;
  Ok(p)
}

fn emit_line(app: &tauri::AppHandle, line: &str) {
  let v: serde_json::Value = match serde_json::from_str(line.trim()) {
    Ok(x) => x,
    Err(_) => return,
  };
  let _ = app.emit_all("word-to-pdf-progress", v);
}

/// 在阻塞线程中运行；通过 `word-to-pdf-progress` 推送进度事件。
pub fn run_word_to_pdf_batch(
  app: &tauri::AppHandle,
  input_dir: &Path,
  output_dir: &Path,
) -> Result<WordToPdfSummary, AppError> {
  fs::create_dir_all(output_dir)?;

  let sources = collect_word_files(input_dir)?;
  if sources.is_empty() {
    return Ok(WordToPdfSummary {
      converted: 0,
      skipped: 0,
      failed: 0,
      output_dir: output_dir.to_string_lossy().into_owned(),
    });
  }

  let pairs: Vec<(PathBuf, PathBuf)> = sources
    .into_iter()
    .map(|src| {
      let stem = src.file_stem().unwrap_or_default();
      let dst = output_dir.join(format!("{}.pdf", stem.to_string_lossy()));
      (src, dst)
    })
    .collect();

  let json_path = write_batch_json(&pairs)?;
  let ps1_path = write_ps1_temp()?;

  let mut child = Command::new("powershell")
    .args([
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      ps1_path.to_str().ok_or_else(|| AppError::msg("路径无效"))?,
      "-JsonPath",
      json_path.to_str().ok_or_else(|| AppError::msg("路径无效"))?,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|e| AppError::msg(format!("无法启动 PowerShell：{}", e)))?;

  let stdout = child.stdout.take().ok_or_else(|| AppError::msg("无法读取子进程输出"))?;
  let stderr = child.stderr.take();
  let err_handle = std::thread::spawn(move || {
    let mut s = String::new();
    if let Some(mut r) = stderr {
      use std::io::Read;
      let _ = r.read_to_string(&mut s);
    }
    s
  });

  let mut converted = 0u32;
  let mut skipped = 0u32;
  let mut failed = 0u32;

  for line in BufReader::new(stdout).lines() {
    let line = line.map_err(|e| AppError::msg(format!("读取输出失败：{}", e)))?;
    let trimmed = line.trim();
    if trimmed.is_empty() {
      continue;
    }
    emit_line(app, trimmed);

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
      if v.get("type").and_then(|t| t.as_str()) == Some("error") {
        let msg = v
          .get("message")
          .and_then(|m| m.as_str())
          .unwrap_or("Word 启动失败");
        return Err(AppError::msg(msg.to_string()));
      }
      if v.get("type").and_then(|t| t.as_str()) == Some("done") {
        converted = v.get("converted").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        skipped = v.get("skipped").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
        failed = v.get("failed").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
      }
    }
  }

  let status = child.wait().map_err(|e| AppError::msg(format!("等待子进程失败：{}", e)))?;
  let err_body = err_handle.join().unwrap_or_default();
  if !status.success() {
    return Err(AppError::msg(format!(
      "PowerShell 退出码 {:?}。{}",
      status.code(),
      err_body.trim()
    )));
  }

  Ok(WordToPdfSummary {
    converted,
    skipped,
    failed,
    output_dir: output_dir.to_string_lossy().into_owned(),
  })
}
