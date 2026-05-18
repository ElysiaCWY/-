#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Windows 发布构建使用 GUI 子系统，避免双击启动时额外弹出黑色命令行窗口（`tauri dev` 调试版仍为控制台子系统，便于看日志）。

#[macro_use]
mod app_log;
mod errors;
mod export_pdf;
mod export_js;
mod extract;
mod jd;
mod json_resume;
mod json_resume_renderer;
mod llm;
mod privacy_mask;
mod schema;
mod storage;
mod validate;
mod word_pdf;

use crate::errors::AppError;
use crate::json_resume::PdfExportOptions;
use crate::llm::LlmSettings;
use crate::schema::{
  AppSettings, JdRecord, JdScoreResult, JdScreeningIndex, ParsedJdScoreRecord, ParsedResultRecord, ResumeData,
  ResumeParseOutput, ResumeRecord,
};
use std::path::PathBuf;
use std::sync::Arc;
use tauri::Manager;

use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct BatchParseItem {
  pub file_path: String,
  pub text: String,
}

#[derive(Serialize)]
pub struct BatchParseResult {
  pub file_path: String,
  pub success: bool,
  pub current_progress: u32,
  pub error: Option<String>,
}

#[tauri::command]
async fn batch_parse_and_save(
  items: Vec<BatchParseItem>,
  settings: LlmSettings,
) -> Result<Vec<BatchParseResult>, AppError> {
  let mut results = Vec::new();
  // Here we can either process them in parallel using futures or sequentially.
  // Given local LLMs often crash on parallel inference, we run them sequentially but do it all in the backend to save IPC overhead and enable continuous processing.
  let is_parallel = items.len() > 1 && settings.llm_provider.contains("deepseek"); // simplistic heuristic or we just do sequentially
  
  if is_parallel {
      // For remote we can do it in parallel
      let mut handles = Vec::new();
      for item in items {
          let settings_clone = settings.clone();
          let fp = item.file_path.clone();
          let text = item.text.clone();
          handles.push(tauri::async_runtime::spawn_blocking(move || {
              let parsed = match llm::parse_resume_with_llm(&text, &settings_clone) {
                  Ok(p) => p,
                  Err(e) => return BatchParseResult { file_path: fp, success: false, current_progress: 100, error: Some(e.to_string()) },
              };
              let resume = validate::normalize_resume(parsed.resume);
              let _saved = match storage::save_resume(fp.clone(), resume.clone()) {
                  Ok(r) => r,
                  Err(e) => return BatchParseResult { file_path: fp, success: false, current_progress: 100, error: Some(e.to_string()) },
              };
              let _ = storage::save_parsed_result_json(fp.clone(), _saved.id, resume, parsed.jd_screening_index);
              BatchParseResult { file_path: fp, success: true, current_progress: 100, error: None }
          }));
      }
      for handle in handles {
          if let Ok(res) = handle.await {
              results.push(res);
          }
      }
  } else {
      for item in items {
          let fp = item.file_path.clone();
          let parsed = match tauri::async_runtime::spawn_blocking({
              let settings_clone = settings.clone();
              let text = item.text.clone();
              move || llm::parse_resume_with_llm(&text, &settings_clone)
          }).await {
              Ok(Ok(p)) => p,
              Ok(Err(e)) => { results.push(BatchParseResult { file_path: fp, success: false, current_progress: 100, error: Some(e.to_string()) }); continue; },
              Err(e) => { results.push(BatchParseResult { file_path: fp, success: false, current_progress: 100, error: Some(e.to_string()) }); continue; },
          };
          
          let resume = validate::normalize_resume(parsed.resume);
          let _saved = match storage::save_resume(fp.clone(), resume.clone()) {
              Ok(r) => r,
              Err(e) => { results.push(BatchParseResult { file_path: fp, success: false, current_progress: 100, error: Some(e.to_string()) }); continue; },
          };
          let _ = storage::save_parsed_result_json(fp.clone(), _saved.id, resume, parsed.jd_screening_index);
          results.push(BatchParseResult { file_path: fp, success: true, current_progress: 100, error: None });
      }
  }
  
  Ok(results)
}

#[tauri::command]
async fn extract_text(file_path: String) -> Result<String, AppError> {
  tauri::async_runtime::spawn_blocking(move || {
    extract::extract_text_from_path(&file_path)
  }).await.map_err(|e| AppError::msg(e.to_string()))?
}

#[tauri::command]
async fn parse_resume(text: String, settings: LlmSettings) -> Result<ResumeParseOutput, AppError> {
  let text_len = text.chars().count();
  resume_parse_log!(debug, "parse_resume: invoke text_chars={}", text_len);
  let out = tauri::async_runtime::spawn_blocking(move || {
    let parsed = llm::parse_resume_with_llm(&text, &settings)?;
    let resume = validate::normalize_resume(parsed.resume);
    Ok(ResumeParseOutput {
      resume,
      jd_screening_index: parsed.jd_screening_index,
    })
  })
  .await
  .map_err(|e| {
    resume_parse_log!(error, "parse_resume: 后台任务失败: {}", e);
    AppError::msg(e.to_string())
  })?;

  match &out {
    Ok(_) => resume_parse_log!(debug, "parse_resume: 成功 text_chars={}", text_len),
    Err(e) => resume_parse_log!(
      error,
      "parse_resume: 失败 text_chars={} err={}",
      text_len,
      e
    ),
  }
  out
}

#[tauri::command]
fn export_js(resume_obj: ResumeData, out_path: String) -> Result<(), AppError> {
  export_js::write_resume_js(&resume_obj, &out_path)
}

#[tauri::command]
fn export_resume_pdf(content: String, out_path: String) -> Result<(), AppError> {
  export_pdf::write_resume_pdf(&content, &out_path)
}

#[tauri::command]
fn export_resume_pdf_from_json(
  json_path: String,
  out_path: String,
  options: Option<PdfExportOptions>,
) -> Result<(), AppError> {
  let content = std::fs::read_to_string(&json_path)?;
  let resume: ResumeData = serde_json::from_str(&content)?;
  json_resume_renderer::export_pdf_with_jsonresume(&resume, options.unwrap_or_default(), &out_path)
}

#[tauri::command]
fn jd_score_v1(resume_obj: ResumeData, jd_text: String) -> Result<JdScoreResult, AppError> {
  Ok(jd::score_v1(&resume_obj, &jd_text))
}

#[tauri::command]
async fn jd_score_from_local_parsed(jd_text: String) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  tauri::async_runtime::spawn_blocking(move || storage::jd_score_from_local_parsed(jd_text))
    .await
    .map_err(|e| AppError::msg(format!("JD 评分任务异常: {}", e)))?
}

#[tauri::command]
async fn jd_filter_by_keywords(
  app: tauri::AppHandle,
  position: String,
  jd_text: String,
  limit: i32,
  rerank_pool: i32,
) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let app_emit = app.clone();
  tauri::async_runtime::spawn_blocking(move || {
    let app_emit = app_emit.clone();
    let progress: Arc<dyn Fn(storage::JdFilterProgressEvent) + Send + Sync> =
      Arc::new(move |ev: storage::JdFilterProgressEvent| {
        let _ = app_emit.emit_all("jd-filter-progress", &ev);
      });
    storage::jd_filter_by_keywords_from_index(position, jd_text, limit, rerank_pool, Some(progress))
  })
  .await
  .map_err(|e| AppError::msg(format!("JD 筛选任务异常: {}", e)))?
}

#[tauri::command]
async fn jd_filter_by_model(
  position: String,
  jd_text: String,
  limit: i32,
) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  tauri::async_runtime::spawn_blocking(move || {
    storage::jd_filter_by_model_from_parsed(position, jd_text, limit)
  })
  .await
  .map_err(|e| AppError::msg(format!("JD 模型筛选任务异常: {}", e)))?
}

#[tauri::command]
fn save_resume_to_library(source_file: String, resume_obj: ResumeData) -> Result<ResumeRecord, AppError> {
  storage::save_resume(source_file, resume_obj)
}

#[tauri::command]
fn list_resume_library() -> Result<Vec<ResumeRecord>, AppError> {
  storage::list_resumes()
}

#[tauri::command]
fn delete_resume_record(id: String) -> Result<(), AppError> {
  storage::delete_resume(id)
}

#[tauri::command]
fn delete_resume_records(ids: Vec<String>) -> Result<usize, AppError> {
  storage::delete_resumes(ids)
}

#[tauri::command]
fn save_jd_record(title: String, text: String) -> Result<JdRecord, AppError> {
  storage::save_jd(title, text)
}

#[tauri::command]
fn list_jd_records() -> Result<Vec<JdRecord>, AppError> {
  storage::list_jds()
}

#[tauri::command]
fn load_app_settings() -> Result<AppSettings, AppError> {
  storage::load_settings()
}

#[tauri::command]
fn save_app_settings(settings: AppSettings) -> Result<(), AppError> {
  storage::save_settings(&settings)
}

#[tauri::command]
fn get_app_settings_path() -> Result<String, AppError> {
  storage::app_settings_file_path()
}

#[tauri::command]
fn save_parsed_result_json(
  source_file: String,
  resume_id: String,
  resume_obj: ResumeData,
  jd_screening_index: JdScreeningIndex,
) -> Result<ParsedResultRecord, AppError> {
  storage::save_parsed_result_json(source_file, resume_id, resume_obj, jd_screening_index)
}

/// 测试/调试：写入 `<项目根>/logs/app.log`（与控制台 RUST_LOG 互补）
#[tauri::command]
fn append_app_log(level: String, message: String) -> Result<(), AppError> {
  let lvl = level.to_lowercase();
  let line = if lvl == "error" {
    log::error!("{}", message);
    "ERROR"
  } else if lvl == "warn" {
    log::warn!("{}", message);
    "WARN"
  } else if lvl == "debug" {
    log::debug!("{}", message);
    "DEBUG"
  } else {
    log::info!("{}", message);
    "INFO"
  };
  app_log::append_line(line, &message)
}

#[tauri::command]
fn get_app_log_path() -> Result<String, AppError> {
  app_log::log_file_path()
    .map(|p| p.to_string_lossy().into_owned())
}

/// Word → PDF（Windows + 本机 Microsoft Word，PowerShell COM）
#[tauri::command]
async fn word_to_pdf_convert(
  app: tauri::AppHandle,
  input_dir: String,
  output_dir: String,
) -> Result<word_pdf::WordToPdfSummary, AppError> {
  let input = PathBuf::from(input_dir);
  let output = PathBuf::from(output_dir);
  let app2 = app.clone();
  tauri::async_runtime::spawn_blocking(move || word_pdf::run_word_to_pdf_batch(&app2, &input, &output))
    .await
    .map_err(|e| AppError::msg(e.to_string()))?
}

#[tauri::command]
fn word_to_pdf_default_dirs() -> Result<(String, String), AppError> {
  let root = storage::project_root_dir()?;
  let (a, b) = word_pdf::default_input_output(&root);
  Ok((
    a.to_string_lossy().into_owned(),
    b.to_string_lossy().into_owned(),
  ))
}

#[tauri::command]
fn analyze_resumes_db() -> Result<(), AppError> {
  storage::analyze_parsed_resumes_db()
}

#[tauri::command]
fn get_jd_screening_index_for_resume(resume_id: String) -> Result<Option<JdScreeningIndex>, AppError> {
  storage::get_jd_screening_index_for_resume(&resume_id)
}

fn main() {
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("resume_manager=info"))
    .format_timestamp_millis()
    .init();

  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      extract_text,
      batch_parse_and_save,
      parse_resume,
      export_js,
      export_resume_pdf,
      export_resume_pdf_from_json,
      jd_score_v1,
      jd_score_from_local_parsed,
      jd_filter_by_keywords,
      jd_filter_by_model,
      save_resume_to_library,
      list_resume_library,
      delete_resume_record,
      delete_resume_records,
      save_jd_record,
      list_jd_records,
      load_app_settings,
      save_app_settings,
      get_app_settings_path,
      save_parsed_result_json,
      append_app_log,
      get_app_log_path,
      word_to_pdf_convert,
      word_to_pdf_default_dirs,
      analyze_resumes_db,
      get_jd_screening_index_for_resume
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

