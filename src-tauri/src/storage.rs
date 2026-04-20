use crate::errors::AppError;
use crate::jd::{JdStructuredRequirement, ResumeStructuredForScore};
use crate::schema::{AppSettings, JdRecord, ParsedJdScoreRecord, ParsedResultRecord, ResumeData, ResumeRecord};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
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

fn resumes_db_path() -> Result<PathBuf, AppError> {
  let dir = project_root_dir()?.join("data");
  if !dir.exists() {
    fs::create_dir_all(&dir)?;
  }
  Ok(dir.join("resumes.db"))
}

fn legacy_resumes_db_path() -> Result<PathBuf, AppError> {
  Ok(data_dir()?.join("resumes.db"))
}

fn migrate_legacy_db_if_needed(new_db_path: &Path) -> Result<(), AppError> {
  if new_db_path.exists() {
    return Ok(());
  }
  let old_db_path = legacy_resumes_db_path()?;
  if !old_db_path.exists() {
    return Ok(());
  }
  fs::copy(&old_db_path, new_db_path)
    .map_err(|e| AppError::msg(format!("迁移旧 SQLite 数据库失败：{}", e)))?;
  Ok(())
}

fn open_resumes_db() -> Result<Connection, AppError> {
  let db_path = resumes_db_path()?;
  migrate_legacy_db_if_needed(&db_path)?;
  let conn = Connection::open(db_path).map_err(|e| AppError::msg(format!("打开 SQLite 失败：{}", e)))?;
  init_resumes_db(&conn)?;
  Ok(conn)
}

fn init_resumes_db(conn: &Connection) -> Result<(), AppError> {
  conn
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS parsed_resumes (
        parsed_id TEXT PRIMARY KEY,
        resume_id TEXT,
        source_file TEXT NOT NULL DEFAULT '',
        candidate_name TEXT NOT NULL DEFAULT '',
        age TEXT NOT NULL DEFAULT '',
        contact TEXT NOT NULL DEFAULT '',
        position TEXT NOT NULL DEFAULT '',
        degree TEXT NOT NULL DEFAULT '',
        work_years TEXT NOT NULL DEFAULT '',
        work_years_num REAL NOT NULL DEFAULT 0,
        skills_text TEXT NOT NULL DEFAULT '',
        skills_json TEXT NOT NULL DEFAULT '[]',
        work_text TEXT NOT NULL DEFAULT '',
        project_text TEXT NOT NULL DEFAULT '',
        json_path TEXT NOT NULL DEFAULT '',
        imported_date TEXT NOT NULL DEFAULT '',
        updated_at TEXT NOT NULL DEFAULT ''
      );
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_position ON parsed_resumes(position);
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_work_years_num ON parsed_resumes(work_years_num);
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_degree ON parsed_resumes(degree);",
    )
    .map_err(|e| AppError::msg(format!("初始化 SQLite 表失败：{}", e)))?;
  add_column_if_missing(conn, "parsed_resumes", "contact", "TEXT NOT NULL DEFAULT ''")?;
  Ok(())
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, def: &str) -> Result<(), AppError> {
  let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, def);
  match conn.execute(&sql, []) {
    Ok(_) => Ok(()),
    Err(e) => {
      let msg = e.to_string().to_ascii_lowercase();
      if msg.contains("duplicate column name") || msg.contains("already exists") {
        Ok(())
      } else {
        Err(AppError::msg(format!("迁移 SQLite 列失败：{}", e)))
      }
    }
  }
}

pub(crate) fn project_root_dir() -> Result<PathBuf, AppError> {
  let settings = settings_path()?;
  settings
    .parent()
    .map(|p| p.to_path_buf())
    .ok_or_else(|| AppError::msg("无法定位项目根目录"))
}

fn parsed_results_dir() -> Result<PathBuf, AppError> {
  let dir = project_root_dir()?.join("parsed-results");
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

static ID_SEQ: AtomicU64 = AtomicU64::new(0);

fn make_id(prefix: &str) -> String {
  let millis = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_millis())
    .unwrap_or(0);
  let seq = ID_SEQ.fetch_add(1, Ordering::Relaxed);
  format!("{prefix}-{millis}-{seq}")
}

fn today_ymd() -> String {
  let now = OffsetDateTime::now_utc();
  let d = now.date();
  format!("{:04}-{:02}-{:02}", d.year(), d.month() as u8, d.day())
}

fn normalize_contact(contact: &str) -> String {
  let digits = contact.chars().filter(|c| c.is_ascii_digit()).collect::<String>();
  if digits.len() >= 11 {
    digits[digits.len() - 11..].to_string()
  } else {
    digits
  }
}

fn identity_key(name: &str, age: &str, contact: &str) -> Option<String> {
  let n = name.trim().to_ascii_lowercase();
  let a = age.trim().to_ascii_lowercase();
  let c = normalize_contact(contact);
  if n.is_empty() || a.is_empty() || c.is_empty() {
    return None;
  }
  Some(format!("{}|{}|{}", n, a, c))
}

fn fallback_identity_key(name: &str, source_file: &str) -> Option<String> {
  let n = name.trim().to_ascii_lowercase();
  let sf = source_file.trim().to_ascii_lowercase();
  if n.is_empty() && sf.is_empty() {
    return None;
  }
  Some(format!("{}|{}", n, sf))
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

fn classify_position_folder(root_dir: &Path, data: &ResumeData, position: &str) -> String {
  let normalized = position
    .split_whitespace()
    .collect::<Vec<_>>()
    .join(" ")
    .trim()
    .to_string();

  if normalized.is_empty() {
    return "未分类".to_string();
  }

  let existing = list_existing_position_folders(root_dir);
  let summary = build_resume_summary_for_folder(data, &normalized);

  // 让 AI 判断归并目录（优先归并到已有目录，必要时新建目录）。
  if let Some(ai_folder) = ai_select_position_folder(&summary, &existing) {
    if let Some(found) = find_similar_existing_folder(&existing, &ai_folder) {
      return found;
    }
    return ai_folder;
  }

  // AI 不可用时，仅回退为岗位名目录，不再做规则化硬编码分类。
  if let Some(found) = find_similar_existing_folder(&existing, &normalized) {
    return found;
  }
  normalized
}

fn build_resume_summary_for_folder(data: &ResumeData, position: &str) -> String {
  let mut skills = data
    .basic_info
    .skills
    .iter()
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .take(10)
    .collect::<Vec<_>>();

  if skills.is_empty() {
    skills.push("(无)");
  }

  let latest_company = data
    .work_experience
    .get("1")
    .map(|w| w.company.trim())
    .filter(|s| !s.is_empty())
    .unwrap_or("(无)");

  format!(
    "岗位：{}\n最近公司：{}\n技能：{}",
    position,
    latest_company,
    skills.join("、")
  )
}

fn list_existing_position_folders(root_dir: &Path) -> Vec<String> {
  let mut out = Vec::new();
  let entries = match fs::read_dir(root_dir) {
    Ok(v) => v,
    Err(_) => return out,
  };

  for entry in entries.flatten() {
    let p = entry.path();
    if !p.is_dir() {
      continue;
    }
    if let Some(name) = p.file_name().and_then(|x| x.to_str()) {
      let n = name.trim();
      if !n.is_empty() {
        out.push(n.to_string());
      }
    }
  }
  out
}

fn normalize_compare_key(s: &str) -> String {
  s.chars()
    .filter(|c| !c.is_whitespace() && !matches!(c, '/' | '\\' | '-' | '_' | '（' | '）' | '(' | ')' | '【' | '】' | '[' | ']'))
    .flat_map(|c| c.to_lowercase())
    .collect::<String>()
}

fn find_similar_existing_folder(existing: &[String], target: &str) -> Option<String> {
  let t = normalize_compare_key(target);
  if t.is_empty() {
    return None;
  }

  for folder in existing {
    let f = normalize_compare_key(folder);
    if f == t {
      return Some(folder.clone());
    }
  }

  for folder in existing {
    let f = normalize_compare_key(folder);
    if f.contains(&t) || t.contains(&f) {
      return Some(folder.clone());
    }
  }

  None
}

fn normalize_ollama_base_url(input: &str) -> String {
  let raw = input.trim();
  if raw.is_empty() || raw.to_ascii_lowercase().ends_with(".exe") {
    return "http://127.0.0.1:11434".to_string();
  }

  if raw.starts_with("http://") || raw.starts_with("https://") {
    return raw.trim_end_matches('/').to_string();
  }

  format!("http://{}", raw.trim_end_matches('/'))
}

fn extract_json_object(s: &str) -> Option<String> {
  let t = s.trim();
  let start = t.find('{')?;
  let end = t.rfind('}')?;
  if end <= start {
    return None;
  }
  Some(t[start..=end].to_string())
}

fn ai_select_position_folder(summary: &str, existing: &[String]) -> Option<String> {
  let settings = load_settings().ok()?;
  if settings.model_path.trim().is_empty() {
    return None;
  }

  let base_url = normalize_ollama_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/api/generate", base_url);
  let existing_text = if existing.is_empty() {
    "（无已有目录）".to_string()
  } else {
    existing.join(" | ")
  };

  let prompt = format!(
    "你是岗位目录归并助手。\n已有目录：{existing_text}\n简历摘要：\n{summary}\n"
  ) +
  "规则：\n1. 若与已有目录语义相近，必须直接复用已有目录名。\n2. 仅在确实无法归并时，新建简短目录名（2-12字）。\n3. 不要输出级别、年限、公司名。\n4. 仅返回 JSON：{\"folder\":\"目录名\"}，不要解释。";

  let body = json!({
    "model": settings.model_path.trim(),
    "prompt": prompt,
    "format": "json",
    "stream": false,
    "options": {
      "temperature": 0.0,
      "num_ctx": 1024,
      "num_thread": settings.threads,
    }
  });

  let resp = ureq::post(&endpoint)
    .set("Content-Type", "application/json")
    .send_json(body)
    .ok()?;

  let payload: Value = resp.into_json().ok()?;
  let raw_text = payload.get("response")?.as_str()?;
  let raw_json = extract_json_object(raw_text)?;
  let v: Value = serde_json::from_str(&raw_json).ok()?;
  let folder = v.get("folder")?.as_str()?.trim();
  if folder.is_empty() {
    return None;
  }
  Some(folder.to_string())
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
  let source_file_trimmed = source_file.trim().to_string();
  let name_trimmed = data.basic_info.name.trim().to_string();
  let new_key = identity_key(&data.basic_info.name, &data.basic_info.age, &data.basic_info.contact);
  if let Some(key) = new_key {
    items.retain(|x| {
      identity_key(&x.data.basic_info.name, &x.data.basic_info.age, &x.data.basic_info.contact)
        .map(|k| k != key)
        .unwrap_or(true)
    });
  } else if let Some(key) = fallback_identity_key(&name_trimmed, &source_file_trimmed) {
    items.retain(|x| {
      fallback_identity_key(&x.data.basic_info.name, &x.source_file)
        .map(|k| k != key)
        .unwrap_or(true)
    });
  }
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
  let target = items.iter().find(|x| x.id == id).cloned();
  let Some(target) = target else {
    return Err(AppError::msg("未找到要删除的简历记录"));
  };

  items.retain(|x| x.id != id);
  write_json(&path, &items)?;

  let index_path = parsed_index_path()?;
  if index_path.exists() {
    let parsed_items: Vec<ParsedResultRecord> = read_json_or_default(&index_path)?;
    let mut kept = Vec::with_capacity(parsed_items.len());
    let mut removed = Vec::new();
    for record in parsed_items {
      if parsed_record_matches_resume(&record, &target) {
        if !record.json_path.trim().is_empty() {
          let _ = fs::remove_file(&record.json_path);
        }
        removed.push(record);
        continue;
      }
      kept.push(record);
    }
    write_json(&index_path, &kept)?;
    delete_parsed_rows_from_sqlite(&removed, &target)?;
  }

  Ok(())
}

fn delete_parsed_rows_from_sqlite(removed: &[ParsedResultRecord], resume: &ResumeRecord) -> Result<(), AppError> {
  if removed.is_empty() {
    return Ok(());
  }
  let conn = open_resumes_db()?;
  let tx = conn
    .unchecked_transaction()
    .map_err(|e| AppError::msg(format!("开启 SQLite 删除事务失败：{}", e)))?;

  for record in removed {
    tx
      .execute("DELETE FROM parsed_resumes WHERE parsed_id = ?1", params![record.id])
      .map_err(|e| AppError::msg(format!("按 parsed_id 删除 SQLite 记录失败：{}", e)))?;

    if let Some(resume_id) = record.resume_id.as_deref() {
      if !resume_id.trim().is_empty() {
        tx
          .execute("DELETE FROM parsed_resumes WHERE resume_id = ?1", params![resume_id])
          .map_err(|e| AppError::msg(format!("按 resume_id 删除 SQLite 记录失败：{}", e)))?;
      }
    }

    tx
      .execute(
        "DELETE FROM parsed_resumes
         WHERE source_file = ?1
           AND candidate_name = ?2
           AND (?3 = '' OR resume_id IS NULL OR resume_id = ?3)",
        params![
          record.source_file.trim(),
          record.candidate_name.trim(),
          resume.id.trim(),
        ],
      )
      .map_err(|e| AppError::msg(format!("按 source_file/candidate_name 删除 SQLite 记录失败：{}", e)))?;
  }

  tx.commit()
    .map_err(|e| AppError::msg(format!("提交 SQLite 删除事务失败：{}", e)))?;
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

fn parsed_record_matches_resume(record: &ParsedResultRecord, resume: &ResumeRecord) -> bool {
  if record.resume_id.as_deref() == Some(resume.id.as_str()) {
    return true;
  }

  if record.source_file.trim() != resume.source_file.trim() {
    return false;
  }

  let candidate_name = resume.data.basic_info.name.trim();
  if candidate_name.is_empty() {
    return true;
  }

  record.candidate_name.trim() == candidate_name
}

fn parsed_record_identity_key(record: &ParsedResultRecord) -> Option<String> {
  identity_key(&record.candidate_name, &record.age, &record.contact)
}

fn extract_primary_position(data: &ResumeData) -> String {
  if let Some(first) = data.work_experience.get("1") {
    let p = first.position.trim();
    if !p.is_empty() {
      return p.to_string();
    }
  }

  for item in data.work_experience.values() {
    let p = item.position.trim();
    if !p.is_empty() {
      return p.to_string();
    }
  }
  String::new()
}

fn extract_skills(data: &ResumeData) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for s in &data.basic_info.skills {
    let text = s.trim();
    if text.is_empty() {
      continue;
    }
    if !out.iter().any(|x| x == text) {
      out.push(text.to_string());
    }
  }
  out
}

fn parse_period_start(period: &str) -> Option<(i32, i32)> {
  let chars: Vec<char> = period.chars().collect();
  if chars.len() < 4 {
    return None;
  }

  for i in 0..=(chars.len() - 4) {
    let year_text: String = chars[i..i + 4].iter().collect();
    if !year_text.chars().all(|c| c.is_ascii_digit()) {
      continue;
    }
    let year = year_text.parse::<i32>().ok()?;
    if !(1950..=2100).contains(&year) {
      continue;
    }

    let mut month = 1;
    let mut j = i + 4;
    while j < chars.len() && !chars[j].is_ascii_digit() {
      j += 1;
    }
    if j < chars.len() {
      let mut month_digits = String::new();
      while j < chars.len() && chars[j].is_ascii_digit() && month_digits.len() < 2 {
        month_digits.push(chars[j]);
        j += 1;
      }
      if let Ok(m) = month_digits.parse::<i32>() {
        if (1..=12).contains(&m) {
          month = m;
        }
      }
    }
    return Some((year, month));
  }

  None
}

fn calc_work_years(data: &ResumeData) -> String {
  let mut starts: Vec<(i32, i32)> = Vec::new();
  for item in data.work_experience.values() {
    if let Some((y, m)) = parse_period_start(&item.period) {
      starts.push((y, m));
    }
  }

  if starts.is_empty() {
    return String::new();
  }

  starts.sort_unstable();
  let (start_y, start_m) = starts[0];
  let now = OffsetDateTime::now_utc();
  let now_y = now.year();
  let now_m = now.month() as i32;
  let months = (now_y - start_y) * 12 + (now_m - start_m);
  let years = (months.max(0) as f32) / 12.0;
  if years < 1.0 {
    return "1年以下".to_string();
  }
  format!("{:.1}年", years)
}

fn extract_top_degree(data: &ResumeData) -> String {
  let mut best_rank = -1;
  let mut best_degree = String::new();
  for edu in &data.basic_info.education {
    let d = edu.degree.trim();
    if d.is_empty() {
      continue;
    }
    let rank = crate::jd::degree_rank(d);
    if rank > best_rank {
      best_rank = rank;
      best_degree = d.to_string();
    }
  }
  best_degree
}

fn build_work_text(data: &ResumeData) -> String {
  let mut parts = Vec::new();
  for item in data.work_experience.values() {
    parts.push(item.company.trim());
    parts.push(item.position.trim());
    parts.push(item.description.trim());
  }
  parts.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

fn build_project_text(data: &ResumeData) -> String {
  let mut parts = Vec::new();
  for item in data.project_experience.values() {
    parts.push(item.project_name.trim());
    parts.push(item.project_description.trim());
    parts.push(item.project_achievements.trim());
  }
  parts.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n")
}

fn upsert_parsed_resume_sqlite(record: &ParsedResultRecord, data: &ResumeData) -> Result<(), AppError> {
  let conn = open_resumes_db()?;
  let skills_text = record.skills.join(" ").to_ascii_lowercase();
  let skills_json = serde_json::to_string(&record.skills).unwrap_or_else(|_| "[]".to_string());
  let work_text = build_work_text(data).to_ascii_lowercase();
  let project_text = build_project_text(data).to_ascii_lowercase();
  let work_years_num = crate::jd::candidate_work_years_num(&record.work_years).unwrap_or(0.0);
  let updated_at = now_epoch().to_string();
  conn
    .execute(
      "INSERT INTO parsed_resumes (
        parsed_id, resume_id, source_file, candidate_name, age, contact, position, degree, work_years, work_years_num,
        skills_text, skills_json, work_text, project_text, json_path, imported_date, updated_at
      ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
      ON CONFLICT(parsed_id) DO UPDATE SET
        resume_id=excluded.resume_id,
        source_file=excluded.source_file,
        candidate_name=excluded.candidate_name,
        age=excluded.age,
        contact=excluded.contact,
        position=excluded.position,
        degree=excluded.degree,
        work_years=excluded.work_years,
        work_years_num=excluded.work_years_num,
        skills_text=excluded.skills_text,
        skills_json=excluded.skills_json,
        work_text=excluded.work_text,
        project_text=excluded.project_text,
        json_path=excluded.json_path,
        imported_date=excluded.imported_date,
        updated_at=excluded.updated_at",
      params![
        record.id,
        record.resume_id,
        record.source_file,
        record.candidate_name,
        record.age,
        record.contact,
        record.position,
        record.degree,
        record.work_years,
        work_years_num,
        skills_text,
        skills_json,
        work_text,
        project_text,
        record.json_path,
        record.imported_date,
        updated_at
      ],
    )
    .map_err(|e| AppError::msg(format!("写入 SQLite 失败：{}", e)))?;
  Ok(())
}

pub fn save_parsed_result_json(source_file: String, resume_id: String, data: ResumeData) -> Result<ParsedResultRecord, AppError> {
  let root_dir = parsed_results_dir()?;
  let index_path = parsed_index_path()?;

  let candidate_name = data.basic_info.name.trim().to_string();
  let age = data.basic_info.age.trim().to_string();
  let contact = data.basic_info.contact.trim().to_string();
  let position = extract_primary_position(&data);
  let degree = extract_top_degree(&data);
  let work_years = calc_work_years(&data);
  let skills = extract_skills(&data);
  let folder_name = classify_position_folder(&root_dir, &data, &position);
  let dir = root_dir.join(sanitize_filename(&folder_name));
  if !dir.exists() {
    fs::create_dir_all(&dir)?;
  }
  let file_base = sanitize_filename(&candidate_name);
  let json_path = unique_json_path(&dir, &file_base);

  let mut items: Vec<ParsedResultRecord> = read_json_or_default(&index_path)?;
  let new_identity = identity_key(&candidate_name, &age, &contact);
  let fallback_identity = fallback_identity_key(&candidate_name, &source_file);
  let has_resume_id = !resume_id.trim().is_empty();
  let mut removed: Vec<ParsedResultRecord> = Vec::new();
  items.retain(|x| {
    let matched_resume_id = has_resume_id && x.resume_id.as_deref() == Some(resume_id.as_str());
    let matched_identity = new_identity
      .as_ref()
      .map(|key| parsed_record_identity_key(x).map(|k| &k == key).unwrap_or(false))
      .unwrap_or(false);
    let matched_fallback = fallback_identity
      .as_ref()
      .map(|key| fallback_identity_key(&x.candidate_name, &x.source_file).map(|k| &k == key).unwrap_or(false))
      .unwrap_or(false);
    if matched_resume_id || matched_identity || matched_fallback {
      removed.push(x.clone());
      return false;
    }
    true
  });
  for old in &removed {
    if !old.json_path.trim().is_empty() {
      let _ = fs::remove_file(&old.json_path);
    }
  }

  write_json(&json_path, &data)?;
  let record = ParsedResultRecord {
    id: make_id("parsed"),
    created_at: now_epoch().to_string(),
    imported_date: today_ymd(),
    resume_id: if resume_id.trim().is_empty() { None } else { Some(resume_id.clone()) },
    source_file,
    candidate_name,
    age,
    contact,
    position,
    degree,
    work_years,
    skills,
    json_path: json_path.to_string_lossy().to_string(),
  };
  items.push(record.clone());
  write_json(&index_path, &items)?;
  if !removed.is_empty() {
    let synthetic_resume = ResumeRecord {
      id: resume_id.clone(),
      created_at: record.created_at.clone(),
      source_file: record.source_file.clone(),
      data: data.clone(),
    };
    delete_parsed_rows_from_sqlite(&removed, &synthetic_resume)?;
  }
  upsert_parsed_resume_sqlite(&record, &data)?;
  Ok(record)
}

pub fn jd_score_from_local_parsed(jd_text: String) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let index_path = parsed_index_path()?;
  let items: Vec<ParsedResultRecord> = read_json_or_default(&index_path)?;

  let mut out: Vec<ParsedJdScoreRecord> = Vec::new();
  for item in items {
    if item.json_path.trim().is_empty() {
      continue;
    }

    let resume_path = PathBuf::from(item.json_path.clone());
    if !resume_path.exists() {
      continue;
    }

    let resume: ResumeData = match read_json_or_default(&resume_path) {
      Ok(v) => v,
      Err(_) => continue,
    };

    let score = crate::jd::score_v1(&resume, &jd_text);
    let candidate_name = if item.candidate_name.trim().is_empty() {
      resume.basic_info.name.trim().to_string()
    } else {
      item.candidate_name.clone()
    };

    out.push(ParsedJdScoreRecord {
      parsed_id: item.id,
      resume_id: item.resume_id,
      candidate_name,
      source_file: item.source_file,
      age: item.age,
      contact: item.contact,
      position: item.position,
      degree: item.degree,
      work_years: item.work_years,
      skills: item.skills,
      json_path: item.json_path,
      score: score.score,
      score_breakdown: crate::schema::JdScoreBreakdown::default(),
      matched_keywords: score.matched_keywords,
      total_keywords: score.total_keywords,
    });
  }

  out.sort_by(|a, b| b.score.cmp(&a.score));
  Ok(out)
}

fn position_match(target: &str, query: &str) -> bool {
  let q = query.trim().to_ascii_lowercase();
  if q.is_empty() {
    return true;
  }
  let t = target.trim().to_ascii_lowercase();
  if t.is_empty() {
    return false;
  }
  t.contains(&q) || q.contains(&t)
}

fn parent_folder_name(path: &str) -> String {
  let p = PathBuf::from(path);
  p.parent()
    .and_then(|x| x.file_name())
    .and_then(|x| x.to_str())
    .unwrap_or("")
    .trim()
    .to_string()
}

fn parsed_item_matches_position(item: &ParsedResultRecord, query: &str) -> bool {
  if query.trim().is_empty() {
    return true;
  }

  if position_match(&item.position, query) {
    return true;
  }

  let folder = parent_folder_name(&item.json_path);
  if position_match(&folder, query) {
    return true;
  }

  false
}

fn clamp_limit(limit: i32) -> usize {
  let n = if limit <= 0 { 10 } else { limit };
  (n as usize).min(200)
}

fn ai_extract_jd_requirements(position: &str, jd_text: &str, settings: &AppSettings) -> Result<JdStructuredRequirement, AppError> {
  let base_url = normalize_ollama_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/api/generate", base_url);
  let prompt = format!(
    "你是 JD 结构化提取助手。请根据岗位名和JD，提取筛选所需结构化要求。\n"
  ) +
  "只返回 JSON，不要解释。JSON 格式：\n"
  + "{\"minDegreeRank\":0-4,\"minWorkYears\":number,\"requiredSkills\":[\"...\"],\"preferredSkills\":[\"...\"],\"workKeywords\":[\"...\"],\"projectKeywords\":[\"...\"]}\n"
  + "其中 minDegreeRank 映射：0不限,1大专,2本科,3硕士,4博士。\n"
  + "岗位："
  + position
  + "\nJD：\n"
  + jd_text;

  let body = json!({
    "model": settings.model_path.trim(),
    "prompt": prompt,
    "format": "json",
    "stream": false,
    "options": {
      "temperature": 0.0,
      "num_ctx": 4096,
      "num_thread": settings.threads,
    }
  });

  let resp = ureq::post(&endpoint)
    .set("Content-Type", "application/json")
    .send_json(body)
    .map_err(|e| AppError::msg(format!("提取 JD 结构化要求失败：{}", e)))?;
  let payload: Value = resp
    .into_json()
    .map_err(|e| AppError::msg(format!("解析 JD 结构化响应失败：{}", e)))?;
  let raw = payload
    .get("response")
    .and_then(|v| v.as_str())
    .ok_or_else(|| AppError::msg("JD 结构化响应缺少 response"))?;
  let json_text = extract_json_object(raw).ok_or_else(|| AppError::msg("JD 结构化响应不是合法 JSON"))?;
  let mut req: JdStructuredRequirement = serde_json::from_str(&json_text)
    .map_err(|e| AppError::msg(format!("JD 结构化 JSON 反序列化失败：{}", e)))?;

  req.min_degree_rank = req.min_degree_rank.clamp(0, 4);
  req.min_work_years = req.min_work_years.max(0.0);
  Ok(req)
}

pub fn jd_filter_by_keywords_from_index(position: String, jd_text: String, limit: i32) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let top_n = clamp_limit(limit);
  let settings = load_settings()?;
  let req = ai_extract_jd_requirements(&position, &jd_text, &settings)?;
  let valid_resume_ids: HashSet<String> = list_resumes()?
    .into_iter()
    .map(|x| x.id.trim().to_string())
    .filter(|x| !x.is_empty())
    .collect();
  let conn = open_resumes_db()?;

  let mut out: Vec<ParsedJdScoreRecord> = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();
  let mut stmt = conn
    .prepare(
      "SELECT
        parsed_id, resume_id, source_file, candidate_name, age, contact, position, degree,
        work_years, skills_json, work_text, project_text, json_path
      FROM parsed_resumes
      WHERE (?1 = '' OR lower(position) LIKE '%' || lower(?1) || '%')
        AND (?2 <= 0 OR work_years_num >= ?2)
        AND (?3 <= 0 OR
          CASE
            WHEN lower(degree) LIKE '%博士%' OR lower(degree) LIKE '%phd%' THEN 4
            WHEN lower(degree) LIKE '%硕士%' OR lower(degree) LIKE '%研究生%' OR lower(degree) LIKE '%master%' THEN 3
            WHEN lower(degree) LIKE '%本科%' OR lower(degree) LIKE '%学士%' OR lower(degree) LIKE '%bachelor%' THEN 2
            WHEN lower(degree) LIKE '%大专%' OR lower(degree) LIKE '%专科%' OR lower(degree) LIKE '%college%' THEN 1
            ELSE 0
          END >= ?3
        )",
    )
    .map_err(|e| AppError::msg(format!("准备 SQLite 查询失败：{}", e)))?;

  let rows = stmt
    .query_map(
      params![position.trim(), req.min_work_years, req.min_degree_rank],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, Option<String>>(1)?,
          row.get::<_, String>(2)?,
          row.get::<_, String>(3)?,
          row.get::<_, String>(4)?,
          row.get::<_, String>(5)?,
          row.get::<_, String>(6)?,
          row.get::<_, String>(7)?,
          row.get::<_, String>(8)?,
          row.get::<_, String>(9)?,
          row.get::<_, String>(10)?,
          row.get::<_, String>(11)?,
          row.get::<_, String>(12)?,
        ))
      },
    )
    .map_err(|e| AppError::msg(format!("执行 SQLite 查询失败：{}", e)))?;

  for row in rows {
    let (
      parsed_id,
      resume_id,
      source_file,
      candidate_name,
      age,
      contact,
      db_position,
      degree,
      work_years,
      skills_json,
      work_text,
      project_text,
      json_path,
    ) = row.map_err(|e| AppError::msg(format!("读取 SQLite 记录失败：{}", e)))?;

    let skills: Vec<String> = serde_json::from_str(&skills_json).unwrap_or_default();
    let resume_struct = ResumeStructuredForScore {
      degree: degree.clone(),
      work_years: work_years.clone(),
      skills: skills.clone(),
      work_text,
      project_text,
    };
    let score = crate::jd::score_structured_resume(&req, &resume_struct);
    if let Some(id) = resume_id.as_deref() {
      let rid = id.trim();
      if !rid.is_empty() && !valid_resume_ids.contains(rid) {
        continue;
      }
    }
    let dedupe_key = resume_id
      .as_deref()
      .map(|x| x.trim().to_string())
      .filter(|x| !x.is_empty())
      .or_else(|| identity_key(&candidate_name, &age, &contact))
      .or_else(|| fallback_identity_key(&candidate_name, &source_file))
      .unwrap_or_else(|| parsed_id.clone());
    if seen.contains(&dedupe_key) {
      continue;
    }
    seen.insert(dedupe_key);

    out.push(ParsedJdScoreRecord {
      parsed_id,
      resume_id,
      candidate_name,
      source_file,
      age,
      contact,
      position: db_position,
      degree,
      work_years,
      skills,
      json_path,
      score: score.total_score,
      score_breakdown: score.breakdown,
      matched_keywords: score.matched_keywords,
      total_keywords: score.total_keywords,
    });
  }

  out.sort_by(|a, b| b.score.cmp(&a.score));
  if out.len() > top_n {
    out.truncate(top_n);
  }
  Ok(out)
}

fn ai_score_resume_match(resume: &ResumeData, jd_text: &str, settings: &AppSettings) -> Result<(i32, Vec<String>, usize), AppError> {
  let base_url = normalize_ollama_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/api/generate", base_url);

  let resume_json = serde_json::to_string(resume)
    .map_err(|e| AppError::msg(format!("序列化简历失败：{}", e)))?;

  let prompt = format!(
    "你是简历匹配评估助手。请根据岗位JD与候选人简历打分。\n"
  ) +
  "输出要求：只返回 JSON，格式为 {\"score\":0-100,\"matchedKeywords\":[\"...\"],\"totalKeywords\":N}。\n"
  + "评分标准：技能匹配、项目相关性、工作经历相关性。\n"
  + &format!("JD:\n{}\n", jd_text)
  + &format!("简历(JSON):\n{}\n", resume_json);

  let body = json!({
    "model": settings.model_path.trim(),
    "prompt": prompt,
    "format": "json",
    "stream": false,
    "options": {
      "temperature": 0.0,
      "num_ctx": 4096,
      "num_thread": settings.threads,
    }
  });

  let resp = ureq::post(&endpoint)
    .set("Content-Type", "application/json")
    .send_json(body)
    .map_err(|e| AppError::msg(format!("调用模型评分失败：{}", e)))?;

  let payload: Value = resp
    .into_json()
    .map_err(|e| AppError::msg(format!("解析模型评分响应失败：{}", e)))?;
  let raw = payload
    .get("response")
    .and_then(|v| v.as_str())
    .ok_or_else(|| AppError::msg("模型评分响应缺少 response"))?;
  let json_text = extract_json_object(raw).ok_or_else(|| AppError::msg("模型评分响应不是合法 JSON"))?;
  let v: Value = serde_json::from_str(&json_text)
    .map_err(|e| AppError::msg(format!("模型评分 JSON 解析失败：{}", e)))?;

  let score = v.get("score").and_then(|x| x.as_i64()).unwrap_or(0).clamp(0, 100) as i32;
  let matched_keywords = v
    .get("matchedKeywords")
    .and_then(|x| x.as_array())
    .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<_>>())
    .unwrap_or_default();
  let total_keywords = v.get("totalKeywords").and_then(|x| x.as_u64()).unwrap_or(0) as usize;

  Ok((score, matched_keywords, total_keywords))
}

pub fn jd_filter_by_model_from_parsed(position: String, jd_text: String, limit: i32) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let index_path = parsed_index_path()?;
  let items: Vec<ParsedResultRecord> = read_json_or_default(&index_path)?;
  let settings = load_settings()?;
  let top_n = clamp_limit(limit);

  let mut out: Vec<ParsedJdScoreRecord> = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();
  for item in items {
    if !parsed_item_matches_position(&item, &position) {
      continue;
    }
    if item.json_path.trim().is_empty() {
      continue;
    }

    let resume_path = PathBuf::from(item.json_path.clone());
    if !resume_path.exists() {
      continue;
    }

    let resume: ResumeData = match read_json_or_default(&resume_path) {
      Ok(v) => v,
      Err(_) => continue,
    };

    let (score, matched_keywords, total_keywords) = match ai_score_resume_match(&resume, &jd_text, &settings) {
      Ok(v) => v,
      Err(_) => continue,
    };
    let dedupe_key = item
      .resume_id
      .as_deref()
      .map(|x| x.trim().to_string())
      .filter(|x| !x.is_empty())
      .or_else(|| identity_key(&item.candidate_name, &item.age, &item.contact))
      .or_else(|| fallback_identity_key(&item.candidate_name, &item.source_file))
      .unwrap_or_else(|| item.id.clone());
    if seen.contains(&dedupe_key) {
      continue;
    }
    seen.insert(dedupe_key);

    out.push(ParsedJdScoreRecord {
      parsed_id: item.id,
      resume_id: item.resume_id,
      candidate_name: item.candidate_name,
      source_file: item.source_file,
      age: item.age,
      contact: item.contact,
      position: item.position,
      degree: item.degree,
      work_years: item.work_years,
      skills: item.skills,
      json_path: item.json_path,
      score,
      score_breakdown: crate::schema::JdScoreBreakdown::default(),
      matched_keywords,
      total_keywords,
    });
  }

  out.sort_by(|a, b| b.score.cmp(&a.score));
  if out.len() > top_n {
    out.truncate(top_n);
  }
  Ok(out)
}

