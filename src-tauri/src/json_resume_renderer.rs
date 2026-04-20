use crate::errors::AppError;
use crate::export_pdf;
use crate::json_resume::resume_data_to_json_resume;
use crate::schema::ResumeData;
use crate::storage;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 构建时嵌入 `jsonresume-theme-local`；当检测到本地已有 `resume` CLI 且项目根缺少主题文件时自动释放。
const EMBEDDED_JSONRESUME_THEME_INDEX_JS: &str =
  include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../jsonresume-theme-local/index.js"));
const EMBEDDED_JSONRESUME_THEME_PACKAGE_JSON: &str =
  include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../jsonresume-theme-local/package.json"));

fn now_millis() -> u128 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0)
}

/// 仅当项目根存在本地 `node_modules/.bin/resume` 时使用 JSON Resume 主题导出（需主题目录等）。
fn resolve_local_resume_cli(root: &Path) -> Option<PathBuf> {
  let win = root.join("node_modules").join(".bin").join("resume.cmd");
  if win.is_file() {
    return Some(win);
  }
  let unix = root.join("node_modules").join(".bin").join("resume");
  if unix.is_file() {
    return Some(unix);
  }
  None
}

/// 若项目根尚无 `jsonresume-theme-local/index.js`，写入内置主题（不覆盖已有文件）。
fn ensure_embedded_jsonresume_theme(root: &Path) -> Result<(), AppError> {
  let theme_dir = root.join("jsonresume-theme-local");
  let index_path = theme_dir.join("index.js");
  if index_path.is_file() {
    return Ok(());
  }
  fs::create_dir_all(&theme_dir).map_err(|e| {
    AppError::msg(format!("创建 jsonresume-theme-local 目录失败：{}", e))
  })?;
  fs::write(&index_path, EMBEDDED_JSONRESUME_THEME_INDEX_JS).map_err(|e| {
    AppError::msg(format!("写入内置 PDF 主题 index.js 失败：{}", e))
  })?;
  fs::write(
    theme_dir.join("package.json"),
    EMBEDDED_JSONRESUME_THEME_PACKAGE_JSON,
  )
  .map_err(|e| AppError::msg(format!("写入内置 PDF 主题 package.json 失败：{}", e)))?;
  log::info!(
    "json_resume: 已释放内置 jsonresume-theme-local（项目根缺少主题时自动创建）"
  );
  Ok(())
}

pub fn export_pdf_with_jsonresume(resume: &ResumeData, include_skills: bool, out_path: &str) -> Result<(), AppError> {
  let root = storage::project_root_dir()?;
  let target_path = PathBuf::from(out_path);
  if let Some(parent) = target_path.parent() {
    if !parent.as_os_str().is_empty() && !parent.exists() {
      fs::create_dir_all(parent)?;
    }
  }

  let has_resume_cli = resolve_local_resume_cli(&root).is_some();
  if has_resume_cli {
    if let Err(e) = ensure_embedded_jsonresume_theme(&root) {
      log::warn!("json_resume: 无法释放内置 PDF 主题：{}", e);
    }
  }
  let theme_index = root.join("jsonresume-theme-local").join("index.js");
  let use_jsonresume = has_resume_cli && theme_index.is_file();

  if use_jsonresume {
    if let Some(program) = resolve_local_resume_cli(&root) {
      let tmp_dir = root.join(".tmp-jsonresume");
      if !tmp_dir.exists() {
        fs::create_dir_all(&tmp_dir)?;
      }
      let tmp_resume = tmp_dir.join(format!("resume-{}.json", now_millis()));
      let tmp_output = tmp_dir.join(format!("resume-{}.pdf", now_millis()));
      let resume_json = resume_data_to_json_resume(resume, include_skills);
      fs::write(&tmp_resume, serde_json::to_string_pretty(&resume_json)?)?;

      let tmp_resume_str = tmp_resume.to_string_lossy().to_string();
      let output = Command::new(&program)
        .current_dir(&root)
        .args([
          "export",
          &tmp_output.to_string_lossy(),
          "--resume",
          &tmp_resume_str,
          "--theme",
          "./jsonresume-theme-local",
          "--format",
          "pdf",
        ])
        .output()
        .map_err(|e| {
          AppError::msg(format!(
            "调用 resume-cli 失败：{}。将尝试内置 PDF。",
            e
          ))
        });

      let _ = fs::remove_file(&tmp_resume);

      match output {
        Ok(out) if out.status.success() && tmp_output.exists() => {
          fs::copy(&tmp_output, &target_path)
            .map_err(|e| AppError::msg(format!("复制 PDF 到目标路径失败：{}", e)))?;
          let _ = fs::remove_file(&tmp_output);
          if target_path.exists() {
            log::info!("json_resume: 已使用本地 resume-cli + 主题导出 PDF");
            return Ok(());
          }
        }
        Ok(out) => {
          let stderr = String::from_utf8_lossy(&out.stderr);
          log::warn!(
            "json_resume: resume-cli 未成功（{}），改用内置 PDF",
            stderr.trim().chars().take(200).collect::<String>()
          );
        }
        Err(_) => {}
      }
      let _ = fs::remove_file(&tmp_output);
    }
  } else {
    log::info!("json_resume: 未检测到 node_modules/resume 与 jsonresume-theme-local，使用内置文本 PDF");
  }

  let content = export_pdf::resume_data_to_plain_pdf_content(resume, include_skills);
  export_pdf::write_resume_pdf(&content, out_path)?;
  Ok(())
}
