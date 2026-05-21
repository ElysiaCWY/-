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
  #[serde(default)]
  pub contact: String,
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
  #[serde(default)]
  pub file_name: String,
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

/// 与 `resume` 同轮生成，供 JD 筛选时压缩送入模型的索引（落盘为 `*.jd-screening.json`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JdScreeningIndex {
  #[serde(default)]
  pub summary_for_jd: String,
  #[serde(default)]
  pub skill_tags: Vec<String>,
  #[serde(default)]
  pub role_tags: Vec<String>,
  #[serde(default)]
  pub domain_tags: Vec<String>,
  /// 工作要点，换行分隔
  #[serde(default)]
  pub work_bullets: String,
  /// 项目要点，换行分隔
  #[serde(default)]
  pub project_bullets: String,
}

/// 解析接口返回：完整简历 + JD 筛选索引。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeParseOutput {
  pub resume: ResumeData,
  pub jd_screening_index: JdScreeningIndex,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedResultRecord {
  pub id: String,
  pub created_at: String,
  #[serde(default)]
  pub imported_date: String,
  #[serde(default)]
  pub resume_id: Option<String>,
  pub source_file: String,
  pub candidate_name: String,
  #[serde(default)]
  pub age: String,
  #[serde(default)]
  pub contact: String,
  #[serde(default)]
  pub position: String,
  #[serde(default)]
  pub degree: String,
  #[serde(default)]
  pub work_years: String,
  #[serde(default)]
  pub skills: Vec<String>,
  pub json_path: String,
  /// `*.jd-screening.json` 绝对路径；旧数据为空。
  #[serde(default)]
  pub jd_screening_json_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedJdScoreRecord {
  pub parsed_id: String,
  #[serde(default)]
  pub resume_id: Option<String>,
  pub candidate_name: String,
  pub source_file: String,
  #[serde(default)]
  pub age: String,
  #[serde(default)]
  pub contact: String,
  #[serde(default)]
  pub position: String,
  #[serde(default)]
  pub degree: String,
  #[serde(default)]
  pub work_years: String,
  #[serde(default)]
  pub skills: Vec<String>,
  #[serde(default)]
  pub json_path: String,
  #[serde(default)]
  pub jd_screening_json_path: String,
  pub score: i32,
  #[serde(default)]
  pub score_breakdown: JdScoreBreakdown,
  pub matched_keywords: Vec<String>,
  pub total_keywords: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct JdScoreBreakdown {
  pub skill_score: i32,
  pub years_score: i32,
  pub degree_score: i32,
  pub work_score: i32,
  pub project_score: i32,
  /// 模型给出的简要评估理由（150 字以内）
  #[serde(default)]
  pub rationale: String,
}

fn default_llm_provider() -> String {
  "ollama".to_string()
}

fn default_settings_threads() -> i32 {
  4
}

fn default_settings_temperature() -> f32 {
  0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
  #[serde(default)]
  pub llama_cli_path: String,
  #[serde(default)]
  pub model_path: String,
  #[serde(default = "default_settings_threads")]
  pub threads: i32,
  #[serde(default = "default_settings_temperature")]
  pub temperature: f32,
  /// `ollama`（默认）、`lmstudio`（OpenAI 兼容本地）、`deepseek`（DeepSeek 官方）、`dashscope` 或 `qwen`（阿里云 DashScope OpenAI 兼容）、`doubao` / `ark` / `volcengine`（火山方舟 OpenAI 兼容）
  #[serde(default = "default_llm_provider")]
  pub llm_provider: String,
  /// 云端 API 密钥（DeepSeek / DashScope 等）；也可仅用环境变量，如 `DEEPSEEK_API_KEY`、`DASHSCOPE_API_KEY`。
  #[serde(default)]
  pub llm_api_key: String,
  /// 云端 OpenAI 兼容请求的 `max_tokens` 上限（可选）。不填则用内置上限；填较小值可缩短「模型允许写出的最长回复」从而往往加快尾段生成，但过长简历 JSON 可能被截断。建议从 8192～12288 试起（≥2048 才生效）。
  #[serde(default)]
  pub cloud_max_output_tokens: Option<u32>,
  /// 禁用云端模型的思考/推理模式（DeepSeek R1、Qwen3 等）。关闭后可显著加速响应，适合结构化 JSON 提取等不需要推理链的任务。
  #[serde(default)]
  pub disable_thinking: bool,
}

impl Default for AppSettings {
  fn default() -> Self {
    Self {
      llama_cli_path: String::new(),
      model_path: String::new(),
      threads: 4,
      temperature: 0.0,
      llm_provider: default_llm_provider(),
      llm_api_key: String::new(),
      cloud_max_output_tokens: None,
      disable_thinking: false,
    }
  }
}

