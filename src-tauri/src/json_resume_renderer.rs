use crate::errors::AppError;
use crate::json_resume::resume_data_to_json_resume;
use crate::schema::ResumeData;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn find_in_ancestors(start: &Path, file_name: &str, max_depth: usize) -> Option<PathBuf> {
  let mut current = Some(start);
  for _ in 0..=max_depth {
    if let Some(dir) = current {
      let candidate = dir.join(file_name);
      if candidate.exists() {
        return Some(candidate);
      }
      current = dir.parent();
    } else {
      break;
    }
  }
  None
}

fn project_root_dir() -> Result<PathBuf, AppError> {
  if let Ok(exe) = std::env::current_exe() {
    if let Some(exe_dir) = exe.parent() {
      if let Some(found) = find_in_ancestors(exe_dir, "package.json", 6) {
        return found.parent().map(|p| p.to_path_buf()).ok_or_else(|| AppError::msg("无法定位项目根目录"));
      }
    }
  }
  if let Ok(cwd) = std::env::current_dir() {
    if let Some(found) = find_in_ancestors(&cwd, "package.json", 4) {
      return found.parent().map(|p| p.to_path_buf()).ok_or_else(|| AppError::msg("无法定位项目根目录"));
    }
  }
  Err(AppError::msg("无法定位 package.json，请在项目根目录运行应用"))
}

fn now_millis() -> u128 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0)
}

fn resolve_resume_cli(root: &Path) -> Option<(String, Vec<String>)> {
  // 1) 优先使用项目本地安装的 resume-cli
  let local_cmd = root.join("node_modules").join(".bin").join("resume.cmd");
  if local_cmd.exists() {
    return Some((local_cmd.to_string_lossy().to_string(), Vec::new()));
  }
  let local_sh = root.join("node_modules").join(".bin").join("resume");
  if local_sh.exists() {
    return Some((local_sh.to_string_lossy().to_string(), Vec::new()));
  }

  // 2) 回退到 npx（Windows 常见为 npx.cmd）
  Some(("npx.cmd".to_string(), vec!["resume".to_string()]))
}

pub fn export_pdf_with_jsonresume(resume: &ResumeData, include_skills: bool, out_path: &str) -> Result<(), AppError> {
  let root = project_root_dir()?;
  let target_path = PathBuf::from(out_path);
  if let Some(parent) = target_path.parent() {
    if !parent.as_os_str().is_empty() && !parent.exists() {
      fs::create_dir_all(parent)?;
    }
  }
  let tmp_dir = root.join(".tmp-jsonresume");
  if !tmp_dir.exists() {
    fs::create_dir_all(&tmp_dir)?;
  }

  let tmp_resume = tmp_dir.join(format!("resume-{}.json", now_millis()));
  let tmp_output = tmp_dir.join(format!("resume-{}.pdf", now_millis()));
  let resume_json = resume_data_to_json_resume(resume, include_skills);
  fs::write(&tmp_resume, serde_json::to_string_pretty(&resume_json)?)?;

  let tmp_resume_str = tmp_resume.to_string_lossy().to_string();
  let (program, mut prefix_args) = resolve_resume_cli(&root)
    .ok_or_else(|| AppError::msg("未找到 JSON Resume CLI，请先在项目根目录执行 npm install"))?;
  prefix_args.extend_from_slice(&[
    "export".to_string(),
    tmp_output.to_string_lossy().to_string(),
    "--resume".to_string(),
    tmp_resume_str.clone(),
    "--theme".to_string(),
    "./jsonresume-theme-local".to_string(),
    "--format".to_string(),
    "pdf".to_string(),
  ]);

  let output = Command::new(&program)
    .current_dir(&root)
    .args(prefix_args)
    .output()
    .map_err(|e| AppError::msg(format!("调用 resume-cli 失败：{}。请确认已执行 npm install 且本地存在 node_modules/.bin/resume.cmd", e)))?;

  let _ = fs::remove_file(&tmp_resume);

  if !output.status.success() {
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    return Err(AppError::msg(format!(
      "JSON Resume 导出失败：{}\n{}",
      stderr.trim(),
      stdout.trim()
    )));
  }
  let stdout = String::from_utf8_lossy(&output.stdout).to_string();
  let stderr = String::from_utf8_lossy(&output.stderr).to_string();
  if stdout.contains("You have to install this theme relative to the folder")
    || stderr.contains("You have to install this theme relative to the folder")
  {
    return Err(AppError::msg(format!(
      "JSON Resume 主题加载失败。stdout: {}\nstderr: {}",
      stdout.trim(),
      stderr.trim()
    )));
  }

  // 部分环境在中文路径下由 puppeteer 直接落盘会失败，先导出到英文临时路径再拷贝。
  if !tmp_output.exists() {
    return Err(AppError::msg(format!(
      "JSON Resume 导出未生成临时文件：{}",
      tmp_output.to_string_lossy()
    )));
  }

  fs::copy(&tmp_output, &target_path)
    .map_err(|e| AppError::msg(format!("移动导出 PDF 到目标路径失败：{}", e)))?;
  let _ = fs::remove_file(&tmp_output);

  if !target_path.exists() {
    return Err(AppError::msg(format!(
      "JSON Resume 导出未生成目标文件：{}",
      out_path
    )));
  }
  Ok(())
}
