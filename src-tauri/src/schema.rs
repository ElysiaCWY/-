use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResumeData {
  pub basic_info: BasicInfo,
  pub work_experience: BTreeMap<String, WorkItem>,
  pub project_experience: BTreeMap<String, ProjectItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BasicInfo {
  pub name: String,
  pub age: String,
  pub gender: String,
  pub education: Vec<EducationItem>,
  pub skills: Vec<String>,
  pub certificates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EducationItem {
  pub school: String,
  pub major: String,
  pub degree: String,
  pub period: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkItem {
  pub company: String,
  pub position: String,
  pub period: String,
  pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectItem {
  pub project_name: String,
  pub project_description: String,
  pub project_achievements: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JdScoreResult {
  pub score: i32,
  pub matched_keywords: Vec<String>,
  pub total_keywords: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeRecord {
  pub id: String,
  pub created_at: String,
  pub source_file: String,
  pub data: ResumeData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JdRecord {
  pub id: String,
  pub created_at: String,
  pub title: String,
  pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedResultRecord {
  pub id: String,
  pub created_at: String,
  #[serde(default)]
  pub imported_date: String,
  pub source_file: String,
  pub candidate_name: String,
  pub json_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
  pub llama_cli_path: String,
  pub model_path: String,
  pub threads: i32,
  pub temperature: f32,
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      llama_cli_path: String::new(),
      model_path: String::new(),
      threads: 4,
      temperature: 0.0,
    }
  }
}

