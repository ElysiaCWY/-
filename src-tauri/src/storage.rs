use crate::errors::AppError;
use crate::schema::{AppSettings, JdRecord, ParsedResultRecord, ResumeData, ResumeRecord};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

fn data_dir() -> Result<PathBuf, AppError> {
  let base = dirs::data_local_dir().ok_or_else(|| AppError::msg("无法获取本地数据目录"))?;
  let dir = base.join("resume-manager");
  if !dir.exists() {
    fs::create_dir_all(&dir)?;
  }
  Ok(dir)
}

fn resumes_path() -> Result<PathBuf, AppError> {
  Ok(data_dir()?.join("resumes.json"))
}

fn jd_path() -> Result<PathBuf, AppError> {
  Ok(data_dir()?.join("jds.json"))
}

fn parsed_results_dir() -> Result<PathBuf, AppError> {
  let dir = data_dir()?.join("parsed-results");
  if !dir.exists() {
    fs::create_dir_all(&dir)?;
  }
  Ok(dir)
}

fn parsed_index_path() -> Result<PathBuf, AppError> {
  Ok(parsed_results_dir()?.join("parsed-index.json"))
}

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

fn settings_path() -> Result<PathBuf, AppError> {
  if let Ok(exe) = std::env::current_exe() {
    if let Some(exe_dir) = exe.parent() {
      if let Some(found) = find_in_ancestors(exe_dir, "app-config.json", 5) {
        return Ok(found);
      }
    }
  }

  if let Ok(cwd) = std::env::current_dir() {
    if let Some(found) = find_in_ancestors(&cwd, "app-config.json", 3) {
      return Ok(found);
    }
    return Ok(cwd.join("app-config.json"));
  }

  Err(AppError::msg("无法定位 app-config.json"))
}

fn absolutize_config_path(raw: &str, base_dir: &Path) -> String {
  let trimmed = raw.trim();
  if trimmed.is_empty() {
    return String::new();
  }
  let p = Path::new(trimmed);
  if p.is_absolute() {
    return p.to_string_lossy().to_string();
  }
  base_dir.join(p).to_string_lossy().to_string()
}

fn is_http_url(s: &str) -> bool {
  let v = s.trim().to_ascii_lowercase();
  v.starts_with("http://") || v.starts_with("https://")
}

fn should_absolutize_llama_cli_path(raw: &str) -> bool {
  let v = raw.trim();
  if v.is_empty() || is_http_url(v) {
    return false;
  }
  let lower = v.to_ascii_lowercase();
  lower.ends_with(".exe") || v.contains('/') || v.contains('\\')
}

fn should_absolutize_model_path(raw: &str) -> bool {
  let v = raw.trim();
  if v.is_empty() {
    return false;
  }
  // Ollama 模型名（如 qwen2.5:3b）不应当被绝对路径化。
  // 仅对明显的本地模型文件路径（.gguf）做绝对路径展开。
  v.to_ascii_lowercase().ends_with(".gguf")
}

fn read_json_or_default<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T, AppError> {
  if !path.exists() {
    return Ok(T::default());
  }
  let s = fs::read_to_string(path)?;
  if s.trim().is_empty() {
    return Ok(T::default());
  }
  Ok(serde_json::from_str(&s)?)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
  let s = serde_json::to_string_pretty(value)?;
  fs::write(path, s)?;
  Ok(())
}

fn now_epoch() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

fn make_id(prefix: &str) -> String {
  format!("{prefix}-{}", now_epoch())
}

fn today_ymd() -> String {
  let now = OffsetDateTime::now_utc();
  let d = now.date();
  format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

fn sanitize_filename(input: &str) -> String {
  let trimmed = input.trim();
  if trimmed.is_empty() {
    return "candidate".to_string();
  }

  let mut out = String::with_capacity(trimmed.len());
  for ch in trimmed.chars() {
    let bad = matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    if bad {
      out.push('_');
    } else {
      out.push(ch);
    }
  }

  let collapsed = out.trim_matches('.').trim();
  if collapsed.is_empty() {
    "candidate".to_string()
  } else {
    collapsed.to_string()
  }
}

fn unique_json_path(base_dir: &Path, base_name: &str) -> PathBuf {
  let direct = base_dir.join(format!("{}.json", base_name));
  if !direct.exists() {
    return direct;
  }

  for i in 2..10000 {
    let candidate = base_dir.join(format!("{}_{}.json", base_name, i));
    if !candidate.exists() {
      return candidate;
    }
  }

  base_dir.join(format!("{}_{}.json", base_name, now_epoch()))
}

pub fn list_resumes() -> Result<Vec<ResumeRecord>, AppError> {
  let path = resumes_path()?;
  let mut items: Vec<ResumeRecord> = read_json_or_default(&path)?;
  items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
  Ok(items)
}

pub fn save_resume(source_file: String, data: ResumeData) -> Result<ResumeRecord, AppError> {
  let path = resumes_path()?;
  let mut items: Vec<ResumeRecord> = read_json_or_default(&path)?;
  let record = ResumeRecord {
    id: make_id("resume"),
    created_at: now_epoch().to_string(),
    source_file,
    data,
  };
  items.push(record.clone());
  write_json(&path, &items)?;
  Ok(record)
}

pub fn delete_resume(id: String) -> Result<(), AppError> {
  let path = resumes_path()?;
  let mut items: Vec<ResumeRecord> = read_json_or_default(&path)?;
  let before = items.len();
  items.retain(|x| x.id != id);
  if items.len() == before {
    return Err(AppError::msg("未找到要删除的简历记录"));
  }
  write_json(&path, &items)?;
  Ok(())
}

pub fn list_jds() -> Result<Vec<JdRecord>, AppError> {
  let path = jd_path()?;
  let mut items: Vec<JdRecord> = read_json_or_default(&path)?;
  items.sort_by(|a, b| b.created_at.cmp(&a.created_at));
  Ok(items)
}

pub fn save_jd(title: String, text: String) -> Result<JdRecord, AppError> {
  let path = jd_path()?;
  let mut items: Vec<JdRecord> = read_json_or_default(&path)?;
  let record = JdRecord {
    id: make_id("jd"),
    created_at: now_epoch().to_string(),
    title,
    text,
  };
  items.push(record.clone());
  write_json(&path, &items)?;
  Ok(record)
}

pub fn load_settings() -> Result<AppSettings, AppError> {
  let path = settings_path()?;
  if !path.exists() {
    return Err(AppError::msg(format!(
      "未找到配置文件：{}。请在项目根目录创建 app-config.json",
      path.display()
    )));
  }
  let mut settings: AppSettings = read_json_or_default(&path)?;
  if let Some(base_dir) = path.parent() {
    if should_absolutize_llama_cli_path(&settings.llama_cli_path) {
      settings.llama_cli_path = absolutize_config_path(&settings.llama_cli_path, base_dir);
    }
    if should_absolutize_model_path(&settings.model_path) {
      settings.model_path = absolutize_config_path(&settings.model_path, base_dir);
    }
  }
  Ok(settings)
}

pub fn save_parsed_result_json(source_file: String, data: ResumeData) -> Result<ParsedResultRecord, AppError> {
  let dir = parsed_results_dir()?;
  let index_path = parsed_index_path()?;

  let candidate_name = data.basic_info.name.trim().to_string();
  let file_base = sanitize_filename(&candidate_name);
  let json_path = unique_json_path(&dir, &file_base);

  write_json(&json_path, &data)?;

  let mut items: Vec<ParsedResultRecord> = read_json_or_default(&index_path)?;
  let record = ParsedResultRecord {
    id: make_id("parsed"),
    created_at: now_epoch().to_string(),
    imported_date: today_ymd(),
    source_file,
    candidate_name,
    json_path: json_path.to_string_lossy().to_string(),
  };
  items.push(record.clone());
  write_json(&index_path, &items)?;
  Ok(record)
}

