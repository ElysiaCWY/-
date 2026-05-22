use crate::errors::AppError;
use crate::export_pdf;
use crate::json_resume::{resume_data_to_json_resume, PdfExportOptions};
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

pub fn export_pdf_with_jsonresume(resume: &ResumeData, options: PdfExportOptions, out_path: &str) -> Result<(), AppError> {
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
  let theme_index = {
    let theme_dir = if options.theme_dir.trim().is_empty() {
      root.join("jsonresume-theme-local")
    } else {
      root.join(options.theme_dir.trim())
    };
    theme_dir.join("index.js")
  };
  let use_jsonresume = has_resume_cli && theme_index.is_file();

  if use_jsonresume {
    if let Some(program) = resolve_local_resume_cli(&root) {
      let tmp_dir = root.join(".tmp-jsonresume");
      if !tmp_dir.exists() {
        fs::create_dir_all(&tmp_dir)?;
      }
      let tmp_resume = tmp_dir.join(format!("resume-{}.json", now_millis()));
      let tmp_output = tmp_dir.join(format!("resume-{}.pdf", now_millis()));
      let theme_path = if options.theme_dir.trim().is_empty() {
        "./jsonresume-theme-local".to_string()
      } else {
        format!("./{}", options.theme_dir.trim().trim_start_matches("./"))
      };

      let resume_json = resume_data_to_json_resume(resume, &options);
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
          &theme_path,
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

  let content = export_pdf::resume_data_to_plain_pdf_content(resume, &options);
  export_pdf::write_resume_pdf(&content, out_path)?;
  Ok(())
}

/// 预览：将简历数据 + 主题渲染为 HTML 字符串，供前端新窗口展示。
pub fn preview_resume_html(resume: &ResumeData, options: &PdfExportOptions) -> Result<String, AppError> {
  let root = storage::project_root_dir()?;

  let theme_index = {
    let theme_dir = if options.theme_dir.trim().is_empty() {
      root.join("jsonresume-theme-local")
    } else {
      root.join(options.theme_dir.trim())
    };
    theme_dir.join("index.js")
  };

  let has_resume_cli = resolve_local_resume_cli(&root).is_some();
  if has_resume_cli && theme_index.is_file() {
    if let Some(program) = resolve_local_resume_cli(&root) {
      if let Err(e) = ensure_embedded_jsonresume_theme(&root) {
        log::warn!("json_resume: 预览无法释放内置主题：{}", e);
      }
      let tmp_dir = root.join(".tmp-jsonresume");
      if !tmp_dir.exists() {
        fs::create_dir_all(&tmp_dir)?;
      }
      let tmp_resume = tmp_dir.join(format!("preview-{}.json", now_millis()));
      let tmp_html = tmp_dir.join(format!("preview-{}.html", now_millis()));
      let theme_path = if options.theme_dir.trim().is_empty() {
        "./jsonresume-theme-local".to_string()
      } else {
        format!("./{}", options.theme_dir.trim().trim_start_matches("./"))
      };

      let resume_json = resume_data_to_json_resume(resume, options);
      fs::write(&tmp_resume, serde_json::to_string_pretty(&resume_json)?)?;

      let output = Command::new(&program)
        .current_dir(&root)
        .args([
          "export",
          &tmp_html.to_string_lossy(),
          "--resume",
          &tmp_resume.to_string_lossy(),
          "--theme",
          &theme_path,
          "--format",
          "html",
        ])
        .output()
        .map_err(|e| AppError::msg(format!("调用 resume-cli 预览失败：{}", e)))?;

      let _ = fs::remove_file(&tmp_resume);

      if output.status.success() && tmp_html.exists() {
        let html = fs::read_to_string(&tmp_html)?;
        let _ = fs::remove_file(&tmp_html);
        return Ok(html);
      }
      let _ = fs::remove_file(&tmp_html);
      let stderr = String::from_utf8_lossy(&output.stderr);
      log::warn!(
        "json_resume: resume-cli 预览未成功（{}），使用内置 HTML",
        stderr.trim().chars().take(200).collect::<String>()
      );
    }
  }

  // 兜底：内置简单 HTML
  Ok(builtin_preview_html(resume, options))
}

fn builtin_preview_html(resume: &ResumeData, options: &PdfExportOptions) -> String {
  let b = &resume.basic_info;
  let name = if options.include_name && !b.name.trim().is_empty() { b.name.as_str() } else { "候选人" };
  let gender = if options.include_gender { b.gender.as_str() } else { "" };
  let age = if options.include_age { b.age.as_str() } else { "" };
  let contact = if options.include_contact { b.contact.as_str() } else { "" };

  let edu = b.education.iter()
    .filter(|e| !e.school.trim().is_empty())
    .map(|e| format!("<div class='item'><b>{}</b> / {} / {}</div>", esc_html(&e.school), esc_html(&e.degree), esc_html(&e.major)))
    .collect::<Vec<_>>().join("");

  let work_html: Vec<String> = {
    let mut keys: Vec<String> = resume.work_experience.keys().cloned().collect();
    keys.sort_by(|a, b| a.parse::<i32>().unwrap_or(0).cmp(&b.parse::<i32>().unwrap_or(0)));
    keys.iter()
      .filter_map(|k| resume.work_experience.get(k))
      .filter(|w| !w.company.trim().is_empty() || !w.position.trim().is_empty())
      .map(|w| format!("<div class='item'><b>{}</b> — {} <span class='muted'>{}</span><br>{}</div>", esc_html(&w.company), esc_html(&w.position), esc_html(&w.period), esc_html(&w.description)))
      .collect()
  };

  let proj_html: Vec<String> = {
    let mut keys: Vec<String> = resume.project_experience.keys().cloned().collect();
    keys.sort_by(|a, b| a.parse::<i32>().unwrap_or(0).cmp(&b.parse::<i32>().unwrap_or(0)));
    keys.iter()
      .filter_map(|k| resume.project_experience.get(k))
      .filter(|p| !p.project_name.trim().is_empty())
      .map(|p| format!("<div class='item'><b>{}</b><br>{}<br><span class='muted'>项目成果：{}</span></div>", esc_html(&p.project_name), esc_html(&p.project_description), esc_html(&p.project_achievements)))
      .collect()
  };

  let skills_html = if options.include_skills && !b.skills.is_empty() {
    format!("<div class='tags'>{}</div>", b.skills.iter().map(|s| format!("<span class='tag'>{}</span>", esc_html(s))).collect::<Vec<_>>().join(""))
  } else { String::new() };

  let work_section = work_html.join("");
  let proj_section = proj_html.join("");

  format!(r#"<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:"Microsoft YaHei",sans-serif;color:#333;max-width:800px;margin:0 auto;line-height:1.8;position:relative;padding-top:60px;padding-bottom:40px}}
.header{{position:fixed;top:0;left:50%;transform:translateX(-50%);width:800px;height:44px;background:#1e3a5f;color:#fff;display:flex;align-items:center;justify-content:space-between;padding:0 24px;font-size:14px;z-index:100}}
.header-left{{font-weight:bold;letter-spacing:4px;font-size:16px}}
.header-right{{font-size:12px;opacity:0.85}}
.content{{padding:20px 24px 0}}
.footer{{position:fixed;bottom:0;left:50%;transform:translateX(-50%);width:800px;height:32px;background:#f0f2f5;color:#999;display:flex;align-items:center;justify-content:space-between;padding:0 24px;font-size:11px;z-index:100;border-top:1px solid #e0e0e0}}
h1{{font-size:28px;border-bottom:2px solid #2563eb;padding-bottom:8px}}
h2{{font-size:18px;color:#2563eb;margin-top:24px}}
.item{{margin:8px 0}}
.muted{{color:#888;font-size:13px}}
.tag{{display:inline-block;border:1px solid #ccc;padding:2px 10px;border-radius:12px;margin:4px;font-size:12px}}
.tags{{margin:8px 0}}
.watermark{{position:fixed;top:50%;left:50%;transform:translate(-50%,-50%) rotate(-30deg);font-size:120px;color:rgba(0,0,0,0.04);pointer-events:none;z-index:50;white-space:nowrap;font-weight:bold}}
@media print{{
  .header{{position:fixed;top:0}}
  .footer{{position:fixed;bottom:0}}
  body{{-webkit-print-color-adjust:exact;print-color-adjust:exact}}
  @page{{margin-top:50px;margin-bottom:35px}}
}}
</style></head><body>
<div class="header"><span class="header-left">大瀚</span><span class="header-right">人才简历</span></div>
<div class="content">
<h1>{name}</h1>
<p>{gender} | {age} | {contact}</p>
<h2>教育背景</h2>{edu}
<h2>工作经历</h2>{work_section}
<h2>项目经历</h2>{proj_section}
<h2>技能特长</h2>{skills_html}
</div>
<div class="footer"><span>大瀚</span><span>第 1 页</span></div>
<div class="watermark">大瀚</div>
</body></html>"#)
}

fn esc_html(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
