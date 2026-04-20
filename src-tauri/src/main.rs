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
mod schema;
mod storage;
mod validate;

use crate::errors::AppError;
use crate::llm::LlmSettings;
use crate::schema::{AppSettings, JdRecord, JdScoreResult, ParsedJdScoreRecord, ParsedResultRecord, ResumeData, ResumeRecord};

#[tauri::command]
async fn extract_text(file_path: String) -> Result<String, AppError> {
  tauri::async_runtime::spawn_blocking(move || {
    extract::extract_text_from_path(&file_path)
  }).await.map_err(|e| AppError::msg(e.to_string()))?
}

#[tauri::command]
async fn parse_resume(text: String, settings: LlmSettings) -> Result<ResumeData, AppError> {
  let text_len = text.chars().count();
  resume_parse_log!(debug, "parse_resume: invoke text_chars={}", text_len);
  let out = tauri::async_runtime::spawn_blocking(move || {
    let parsed = llm::parse_resume_with_llm(&text, &settings)?;
    Ok(validate::normalize_resume(parsed))
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
fn export_resume_pdf_from_json(json_path: String, out_path: String, include_skills: bool) -> Result<(), AppError> {
  let content = std::fs::read_to_string(&json_path)?;
  let resume: ResumeData = serde_json::from_str(&content)?;
  json_resume_renderer::export_pdf_with_jsonresume(&resume, include_skills, &out_path)
}

#[tauri::command]
fn jd_score_v1(resume_obj: ResumeData, jd_text: String) -> Result<JdScoreResult, AppError> {
  Ok(jd::score_v1(&resume_obj, &jd_text))
}

#[tauri::command]
fn jd_score_from_local_parsed(jd_text: String) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  storage::jd_score_from_local_parsed(jd_text)
}

#[tauri::command]
fn jd_filter_by_keywords(position: String, jd_text: String, limit: i32) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  storage::jd_filter_by_keywords_from_index(position, jd_text, limit)
}

#[tauri::command]
fn jd_filter_by_model(position: String, jd_text: String, limit: i32) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  storage::jd_filter_by_model_from_parsed(position, jd_text, limit)
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
fn save_parsed_result_json(source_file: String, resume_id: String, resume_obj: ResumeData) -> Result<ParsedResultRecord, AppError> {
  storage::save_parsed_result_json(source_file, resume_id, resume_obj)
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

fn main() {
  env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("resume_manager=info"))
    .format_timestamp_millis()
    .init();

  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      extract_text,
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
      save_jd_record,
      list_jd_records,
      load_app_settings,
      save_parsed_result_json,
      append_app_log,
      get_app_log_path
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

