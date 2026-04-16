mod errors;
mod export_js;
mod extract;
mod jd;
mod llm;
mod schema;
mod storage;
mod validate;

use crate::errors::AppError;
use crate::llm::LlmSettings;
use crate::schema::{AppSettings, JdRecord, JdScoreResult, ResumeData, ResumeRecord};

#[tauri::command]
async fn extract_text(file_path: String) -> Result<String, AppError> {
  tauri::async_runtime::spawn_blocking(move || {
    extract::extract_text_from_path(&file_path)
  }).await.map_err(|e| AppError::msg(e.to_string()))?
}

#[tauri::command]
async fn parse_resume(text: String, settings: LlmSettings) -> Result<ResumeData, AppError> {
  tauri::async_runtime::spawn_blocking(move || {
    let parsed = llm::parse_resume_with_llm(&text, &settings)?;
    Ok(validate::normalize_resume(parsed))
  }).await.map_err(|e| AppError::msg(e.to_string()))?
}

#[tauri::command]
fn export_js(resume_obj: ResumeData, out_path: String) -> Result<(), AppError> {
  export_js::write_resume_js(&resume_obj, &out_path)
}

#[tauri::command]
fn jd_score_v1(resume_obj: ResumeData, jd_text: String) -> Result<JdScoreResult, AppError> {
  Ok(jd::score_v1(&resume_obj, &jd_text))
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

fn main() {
  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![
      extract_text,
      parse_resume,
      export_js,
      jd_score_v1,
      save_resume_to_library,
      list_resume_library,
      save_jd_record,
      list_jd_records,
      load_app_settings
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}

