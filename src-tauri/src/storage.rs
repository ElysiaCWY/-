use crate::errors::AppError;
use crate::jd::{JdStructuredRequirement, ResumeStructuredForScore};
use crate::llm::{complete_json_prompt, drain_token_usage_log, JsonPromptParams, LlmSettings, TokenUsageRecord};
use crate::privacy_mask::{mask_sensitive_segments, unmask_sensitive_segments, LLM_PLACEHOLDER_GUARD};
use crate::schema::{AppSettings, JdRecord, JdScreeningIndex, ParsedJdScoreRecord, ParsedResultRecord, ResumeData, ResumeRecord};
use rusqlite::{params, Connection};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
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
  apply_sqlite_pragmas(&conn)?;
  init_resumes_db(&conn)?;
  finalize_migration_v3();
  Ok(conn)
}

/// 桌面单机场景：WAL + 适度同步 + 更大页缓存与 mmap，减轻 JD 预筛与批量写入抖动。
fn apply_sqlite_pragmas(conn: &Connection) -> Result<(), AppError> {
  let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
  conn
    .execute_batch(
      "PRAGMA synchronous=NORMAL;
       PRAGMA cache_size=-64000;
       PRAGMA mmap_size=268435456;",
    )
    .map_err(|e| AppError::msg(format!("设置 SQLite PRAGMA 失败：{}", e)))?;
  Ok(())
}

const SCHEMA_USER_VERSION_V2: i32 = 2;
const SCHEMA_USER_VERSION_V3: i32 = 3;

/// 回填 `degree_rank`、创建 JD 预筛复合索引，并执行一次 ANALYZE（仅升级时跑一次）。
fn migrate_parsed_resumes_to_v2(conn: &Connection) -> Result<(), AppError> {
  let ver: i32 = conn
    .query_row("PRAGMA user_version", [], |r| r.get(0))
    .unwrap_or(0);
  if ver >= SCHEMA_USER_VERSION_V2 {
    return Ok(());
  }
  conn
    .execute(
      "UPDATE parsed_resumes SET degree_rank = CASE
        WHEN lower(degree) LIKE '%博士%' OR lower(degree) LIKE '%phd%' THEN 4
        WHEN lower(degree) LIKE '%硕士%' OR lower(degree) LIKE '%研究生%' OR lower(degree) LIKE '%master%' THEN 3
        WHEN lower(degree) LIKE '%本科%' OR lower(degree) LIKE '%学士%' OR lower(degree) LIKE '%bachelor%' THEN 2
        WHEN lower(degree) LIKE '%大专%' OR lower(degree) LIKE '%专科%' OR lower(degree) LIKE '%college%' THEN 1
        ELSE 0
      END",
      [],
    )
    .map_err(|e| AppError::msg(format!("回填 degree_rank 失败：{}", e)))?;
  conn
    .execute(
      "CREATE INDEX IF NOT EXISTS idx_parsed_resumes_jd_prefilter ON parsed_resumes(work_years_num, degree_rank)",
      [],
    )
    .map_err(|e| AppError::msg(format!("创建 JD 预筛复合索引失败：{}", e)))?;
  conn
    .pragma_update(None, "user_version", SCHEMA_USER_VERSION_V2)
    .map_err(|e| AppError::msg(format!("更新 user_version 失败：{}", e)))?;
  conn
    .execute("ANALYZE parsed_resumes", [])
    .map_err(|e| AppError::msg(format!("ANALYZE parsed_resumes 失败：{}", e)))?;
  log::info!("SQLite 已迁移至 schema v2：degree_rank + idx_parsed_resumes_jd_prefilter，并已 ANALYZE");
  Ok(())
}

/// 将 JSON 文件数据迁入 SQLite（v2 → v3），幂等安全。
fn migrate_to_v3(conn: &Connection) -> Result<(), AppError> {
  let ver: i32 = conn
    .query_row("PRAGMA user_version", [], |r| r.get(0))
    .unwrap_or(0);
  if ver >= SCHEMA_USER_VERSION_V3 {
    return Ok(());
  }

  // 迁移 resumes.json → resume_library
  if let Ok(path) = resumes_path() {
    if path.exists() {
      if let Ok(items) = read_json_file::<Vec<ResumeRecord>>(&path) {
        for item in &items {
          let data_json = serde_json::to_string(&item.data).unwrap_or_default();
          let _ = conn.execute(
            "INSERT OR IGNORE INTO resume_library (id, created_at, source_file, data_json) VALUES (?1,?2,?3,?4)",
            params![item.id, item.created_at, item.source_file, data_json],
          );
        }
      }
    }
  }

  // 迁移 jds.json → jd_records
  if let Ok(path) = jd_path() {
    if path.exists() {
      if let Ok(items) = read_json_file::<Vec<JdRecord>>(&path) {
        for item in &items {
          let _ = conn.execute(
            "INSERT OR IGNORE INTO jd_records (id, created_at, title, text) VALUES (?1,?2,?3,?4)",
            params![item.id, item.created_at, item.title, item.text],
          );
        }
      }
    }
  }

  // 迁移 parsed-index.json → parsed_resumes (填充 data_json, jd_screening_index_json)
  if let Ok(index_path) = parsed_index_path() {
    if index_path.exists() {
      if let Ok(items) = read_json_file::<Vec<ParsedResultRecord>>(&index_path) {
        for item in &items {
          let data_json_str = load_json_file_content(&item.json_path);
          let jd_idx_str = if !item.jd_screening_json_path.trim().is_empty() {
            load_json_file_content(&item.jd_screening_json_path)
          } else {
            String::new()
          };

          let _ = conn.execute(
            "INSERT INTO parsed_resumes (
              parsed_id, resume_id, source_file, candidate_name, age, contact,
              position, degree, work_years, work_years_num, degree_rank,
              skills_text, skills_json, work_text, project_text,
              json_path, jd_screening_json_path, imported_date, updated_at,
              created_at, data_json, jd_screening_index_json
            ) VALUES (
              ?1,?2,?3,?4,?5,?6,?7,?8,?9,0,0,?10,?11,'','',?12,?13,?14,?15,?16,?17,?18
            ) ON CONFLICT(parsed_id) DO UPDATE SET
              created_at=excluded.created_at,
              data_json=excluded.data_json,
              jd_screening_index_json=excluded.jd_screening_index_json,
              resume_id=COALESCE(excluded.resume_id, parsed_resumes.resume_id),
              json_path=COALESCE(excluded.json_path, parsed_resumes.json_path),
              jd_screening_json_path=COALESCE(excluded.jd_screening_json_path, parsed_resumes.jd_screening_json_path)",
            params![
              item.id,
              item.resume_id,
              item.source_file,
              item.candidate_name,
              item.age,
              item.contact,
              item.position,
              item.degree,
              item.work_years,
              item.skills.join(" ").to_ascii_lowercase(),
              serde_json::to_string(&item.skills).unwrap_or_else(|_| "[]".to_string()),
              item.json_path,
              item.jd_screening_json_path,
              item.imported_date,
              now_epoch().to_string(),
              item.created_at,
              data_json_str,
              jd_idx_str,
            ],
          );
        }
      }
    }
  }

  conn
    .pragma_update(None, "user_version", SCHEMA_USER_VERSION_V3)
    .map_err(|e| AppError::msg(format!("更新 user_version 至 v3 失败：{}", e)))?;
  conn
    .execute("ANALYZE", [])
    .map_err(|e| AppError::msg(format!("ANALYZE v3 失败：{}", e)))?;
  log::info!("SQLite 已迁移至 schema v3：纯 SQLite 存储");
  Ok(())
}

/// 读取 JSON 文件内容字符串；文件不存在或无效时返回空字符串。
fn load_json_file_content(path_str: &str) -> String {
  let p = path_str.trim();
  if p.is_empty() {
    return String::new();
  }
  let path = Path::new(p);
  if !path.exists() {
    return String::new();
  }
  match fs::read_to_string(path) {
    Ok(s) => {
      if s.trim().is_empty() {
        String::new()
      } else {
        s
      }
    }
    Err(_) => String::new(),
  }
}

/// 读取 JSON 文件并反序列化（用于迁移阶段读取旧文件）。
fn read_json_file<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, AppError> {
  let s = fs::read_to_string(path)?;
  if s.trim().is_empty() {
    return serde_json::from_str("null").map_err(AppError::from);
  }
  serde_json::from_str(&s).map_err(AppError::from)
}

/// 迁移完成后将旧 JSON 文件重命名为 .bak（best-effort）。
fn finalize_migration_v3() {
  if let Ok(path) = resumes_path() {
    if path.exists() {
      let bak = path.with_extension("json.bak");
      let _ = fs::rename(&path, &bak);
    }
  }
  if let Ok(path) = jd_path() {
    if path.exists() {
      let bak = path.with_extension("json.bak");
      let _ = fs::rename(&path, &bak);
    }
  }
  if let Ok(path) = parsed_index_path() {
    if path.exists() {
      let bak = path.with_extension("json.bak");
      let _ = fs::rename(&path, &bak);
    }
  }
}

/// 在批量导入/解析结束后调用，更新查询计划统计信息。
pub fn analyze_parsed_resumes_db() -> Result<(), AppError> {
  let conn = open_resumes_db()?;
  conn
    .execute("ANALYZE parsed_resumes", [])
    .map_err(|e| AppError::msg(format!("ANALYZE parsed_resumes 失败：{}", e)))?;
  Ok(())
}

fn log_jd_prefilter_query_plan(conn: &Connection, min_work_years: f32, min_degree_rank: i32) {
  if !log::log_enabled!(log::Level::Debug) {
    return;
  }
  let sql = "EXPLAIN QUERY PLAN SELECT parsed_id FROM parsed_resumes \
    WHERE (?1 <= 0 OR work_years_num >= ?1) \
      AND (?2 <= 0 OR degree_rank >= ?2)";
  let mut stmt = match conn.prepare(sql) {
    Ok(s) => s,
    Err(e) => {
      log::debug!("jd_prefilter EXPLAIN prepare 失败: {}", e);
      return;
    }
  };
  let rows = match stmt.query_map(params![min_work_years, min_degree_rank], |row| {
    Ok((
      row.get::<_, i64>(0)?,
      row.get::<_, i64>(1)?,
      row.get::<_, i64>(2)?,
      row.get::<_, String>(3)?,
    ))
  }) {
    Ok(r) => r,
    Err(e) => {
      log::debug!("jd_prefilter EXPLAIN query_map 失败: {}", e);
      return;
    }
  };
  for row in rows.flatten() {
    let (id, parent, notused, detail) = row;
    log::debug!(
      "jd_prefilter EXPLAIN QUERY PLAN: id={} parent={} notused={} detail={}",
      id,
      parent,
      notused,
      detail
    );
  }
}

fn init_resumes_db(conn: &Connection) -> Result<(), AppError> {
  conn
    .execute_batch(
      "CREATE TABLE IF NOT EXISTS resume_library (
        id TEXT PRIMARY KEY,
        created_at TEXT NOT NULL,
        source_file TEXT NOT NULL,
        data_json TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS jd_records (
        id TEXT PRIMARY KEY,
        created_at TEXT NOT NULL,
        title TEXT NOT NULL,
        text TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS parsed_resumes (
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
      CREATE TABLE IF NOT EXISTS token_usage (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        created_at TEXT NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        label TEXT NOT NULL,
        prompt_tokens INTEGER NOT NULL DEFAULT 0,
        completion_tokens INTEGER NOT NULL DEFAULT 0,
        total_tokens INTEGER NOT NULL DEFAULT 0
      );
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_position ON parsed_resumes(position);
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_work_years_num ON parsed_resumes(work_years_num);
      CREATE INDEX IF NOT EXISTS idx_parsed_resumes_degree ON parsed_resumes(degree);",
    )
    .map_err(|e| AppError::msg(format!("初始化 SQLite 表失败：{}", e)))?;
  add_column_if_missing(conn, "parsed_resumes", "contact", "TEXT NOT NULL DEFAULT ''")?;
  add_column_if_missing(
    conn,
    "parsed_resumes",
    "degree_rank",
    "INTEGER NOT NULL DEFAULT 0",
  )?;
  add_column_if_missing(
    conn,
    "parsed_resumes",
    "jd_screening_json_path",
    "TEXT NOT NULL DEFAULT ''",
  )?;
  migrate_parsed_resumes_to_v2(conn)?;
  add_column_if_missing(conn, "parsed_resumes", "created_at", "TEXT NOT NULL DEFAULT ''")?;
  add_column_if_missing(conn, "parsed_resumes", "data_json", "TEXT NOT NULL DEFAULT ''")?;
  add_column_if_missing(
    conn,
    "parsed_resumes",
    "jd_screening_index_json",
    "TEXT NOT NULL DEFAULT ''",
  )?;
  migrate_to_v3(conn)?;
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
  let parent = settings
    .parent()
    .map(|p| p.to_path_buf())
    .ok_or_else(|| AppError::msg("无法定位项目根目录"))?;
  // 安装包内默认配置在 exe 同级的 resources/app-config.json，数据目录仍放在安装根（与 exe 并列），而非 resources 子目录内。
  if parent.file_name().and_then(|n| n.to_str()) == Some("resources") {
    if let Ok(exe) = std::env::current_exe() {
      if let Some(exe_dir) = exe.parent() {
        if settings.starts_with(exe_dir) {
          return Ok(exe_dir.to_path_buf());
        }
      }
    }
  }
  Ok(parent)
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
      let beside = exe_dir.join("app-config.json");
      if beside.exists() {
        return Ok(beside);
      }
      // 调试构建下 exe 在 target/debug，内置默认在 resources/；若先选 resources，会覆盖仓库根目录的 app-config.json。
      if let Some(found) = find_in_ancestors(exe_dir, "app-config.json", 8) {
        return Ok(found);
      }
      let bundled = exe_dir.join("resources").join("app-config.json");
      if bundled.exists() {
        return Ok(bundled);
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

/// 检查两份简历是否姓名相同且至少有一条教育经历重叠（学校+专业相同）。
fn name_education_overlap(a: &ResumeData, b: &ResumeData) -> bool {
  let na = a.basic_info.name.trim().to_ascii_lowercase();
  let nb = b.basic_info.name.trim().to_ascii_lowercase();
  if na.is_empty() || nb.is_empty() || na != nb {
    return false;
  }
  for ea in &a.basic_info.education {
    let sa = ea.school.trim().to_ascii_lowercase();
    if sa.is_empty() {
      continue;
    }
    for eb in &b.basic_info.education {
      let sb = eb.school.trim().to_ascii_lowercase();
      if sb.is_empty() || sa != sb {
        continue;
      }
      let ma = ea.major.trim().to_ascii_lowercase();
      let mb = eb.major.trim().to_ascii_lowercase();
      if ma.is_empty() && mb.is_empty() {
        return true;
      }
      if ma == mb {
        return true;
      }
    }
  }
  false
}

/// 融合两份简历：以 new 为底，old 中非空字段补入，列表字段取并集去重。
fn merge_resume_data(old: &ResumeData, new: &ResumeData) -> ResumeData {
  let mut merged = new.clone();

  // BasicInfo — 补齐 old 中有而 new 中为空的字段
  if merged.basic_info.name.trim().is_empty() {
    merged.basic_info.name = old.basic_info.name.clone();
  }
  if merged.basic_info.age.trim().is_empty() {
    merged.basic_info.age = old.basic_info.age.clone();
  }
  if merged.basic_info.contact.trim().is_empty() {
    merged.basic_info.contact = old.basic_info.contact.clone();
  }
  if merged.basic_info.gender.trim().is_empty() {
    merged.basic_info.gender = old.basic_info.gender.clone();
  }

  // Education — 按 (school, major, degree, period) 去重合并
  {
    let mut seen = std::collections::HashSet::new();
    let mut all: Vec<crate::schema::EducationItem> = Vec::new();
    for e in old.basic_info.education.iter().chain(new.basic_info.education.iter()) {
      let key = format!(
        "{}|{}|{}|{}",
        e.school.trim().to_ascii_lowercase(),
        e.major.trim().to_ascii_lowercase(),
        e.degree.trim().to_ascii_lowercase(),
        e.period.trim().to_ascii_lowercase()
      );
      if seen.insert(key) {
        all.push(e.clone());
      }
    }
    merged.basic_info.education = all;
  }

  // Skills — 并集（忽略大小写去重）
  {
    let mut seen = std::collections::HashSet::new();
    let mut all: Vec<String> = Vec::new();
    for s in old.basic_info.skills.iter().chain(new.basic_info.skills.iter()) {
      let lower = s.trim().to_ascii_lowercase();
      if !lower.is_empty() && seen.insert(lower) {
        all.push(s.trim().to_string());
      }
    }
    merged.basic_info.skills = all;
  }

  // Certificates — 并集（忽略大小写去重）
  {
    let mut seen = std::collections::HashSet::new();
    let mut all: Vec<String> = Vec::new();
    for c in old.basic_info.certificates.iter().chain(new.basic_info.certificates.iter()) {
      let lower = c.trim().to_ascii_lowercase();
      if !lower.is_empty() && seen.insert(lower) {
        all.push(c.trim().to_string());
      }
    }
    merged.basic_info.certificates = all;
  }

  // WorkExperience — 合并去重，按 (company, position) 判同，保留描述更长的
  {
    let mut dedup: std::collections::BTreeMap<String, crate::schema::WorkItem> = std::collections::BTreeMap::new();
    for item in old.work_experience.values().chain(new.work_experience.values()) {
      let key = format!(
        "{}|{}",
        item.company.trim().to_ascii_lowercase(),
        item.position.trim().to_ascii_lowercase()
      );
      let entry = dedup.entry(key).or_insert_with(|| item.clone());
      if item.description.trim().len() > entry.description.trim().len() {
        *entry = item.clone();
      }
    }
    let mut renumbered = std::collections::BTreeMap::new();
    for (i, (_, v)) in dedup.into_iter().enumerate() {
      renumbered.insert(format!("{}", i + 1), v);
    }
    merged.work_experience = renumbered;
  }

  // ProjectExperience — 合并去重，按 project_name 判同，保留描述更长的
  {
    let mut dedup: std::collections::BTreeMap<String, crate::schema::ProjectItem> = std::collections::BTreeMap::new();
    for item in old.project_experience.values().chain(new.project_experience.values()) {
      let key = item.project_name.trim().to_ascii_lowercase();
      let entry = dedup.entry(key).or_insert_with(|| item.clone());
      let old_len = entry.project_description.trim().len() + entry.project_achievements.trim().len();
      let new_len = item.project_description.trim().len() + item.project_achievements.trim().len();
      if new_len > old_len {
        *entry = item.clone();
      }
    }
    let mut renumbered = std::collections::BTreeMap::new();
    for (i, (_, v)) in dedup.into_iter().enumerate() {
      renumbered.insert(format!("{}", i + 1), v);
    }
    merged.project_experience = renumbered;
  }

  merged
}

/// 与 `*.json` 简历文件同目录、同主文件名，扩展名为 `.jd-screening.json`。
fn jd_screening_sibling_path(resume_json_path: &Path) -> PathBuf {
  let stem = resume_json_path
    .file_stem()
    .and_then(|s| s.to_str())
    .unwrap_or("candidate");
  resume_json_path
    .parent()
    .unwrap_or_else(|| Path::new("."))
    .join(format!("{}.jd-screening.json", stem))
}

/// 按简历库 `resume_id` 加载解析阶段生成的 JD 筛选索引，供详情页展示。
pub fn get_jd_screening_index_for_resume(resume_id: &str) -> Result<Option<JdScreeningIndex>, AppError> {
  let rid = resume_id.trim();
  if rid.is_empty() {
    return Ok(None);
  }

  let conn = open_resumes_db()?;
  match conn.query_row(
    "SELECT jd_screening_index_json FROM parsed_resumes \
     WHERE trim(coalesce(resume_id,'')) = ?1 \
     ORDER BY updated_at DESC LIMIT 1",
    params![rid],
    |row| row.get::<_, String>(0),
  ) {
    Ok(json_str) => {
      if json_str.trim().is_empty() {
        return Ok(None);
      }
      match serde_json::from_str::<JdScreeningIndex>(&json_str) {
        Ok(idx) => Ok(Some(idx)),
        Err(_) => Ok(None),
      }
    }
    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
    Err(e) => Err(AppError::msg(format!("查询解析索引失败：{}", e))),
  }
}

pub fn list_resumes() -> Result<Vec<ResumeRecord>, AppError> {
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare("SELECT id, created_at, source_file, data_json FROM resume_library ORDER BY created_at DESC")
    .map_err(|e| AppError::msg(format!("查询简历库失败：{}", e)))?;
  let rows = stmt
    .query_map([], |row| {
      Ok(ResumeRecord {
        id: row.get(0)?,
        created_at: row.get(1)?,
        source_file: row.get(2)?,
        data: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or_default(),
      })
    })
    .map_err(|e| AppError::msg(format!("读取简历库失败：{}", e)))?;
  let items: Vec<ResumeRecord> = rows.filter_map(|r| r.ok()).collect();
  Ok(items)
}

pub fn save_resume(source_file: String, data: ResumeData) -> Result<ResumeRecord, AppError> {
  let conn = open_resumes_db()?;
  let source_file_trimmed = source_file.trim().to_string();
  let name_trimmed = data.basic_info.name.trim().to_string();
  let new_key = identity_key(&data.basic_info.name, &data.basic_info.age, &data.basic_info.contact);
  let fallback = fallback_identity_key(&name_trimmed, &source_file_trimmed);

  let existing: Vec<(String, String, ResumeData)> = {
    let mut stmt = conn
      .prepare("SELECT id, source_file, data_json FROM resume_library")
      .map_err(|e| AppError::msg(format!("查询简历库失败：{}", e)))?;
    let result = stmt
      .query_map([], |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          serde_json::from_str::<ResumeData>(&row.get::<_, String>(2)?).unwrap_or_default(),
        ))
      })
      .map_err(|e| AppError::msg(format!("读取简历库失败：{}", e)))?
      .filter_map(|r| r.ok())
      .collect();
    result
  };

  let tx = conn
    .unchecked_transaction()
    .map_err(|e| AppError::msg(format!("开启事务失败：{}", e)))?;

  let mut merged_data: Option<ResumeData> = None;

  for (id, sf, d) in &existing {
    // 姓名相同 + 教育经历有交集 → 融合两份简历
    if name_education_overlap(&data, d) {
      if merged_data.is_none() {
        merged_data = Some(merge_resume_data(d, &data));
      }
      tx
        .execute("DELETE FROM resume_library WHERE id = ?1", params![id])
        .map_err(|e| AppError::msg(format!("去重删除失败：{}", e)))?;
      continue;
    }

    let matched = new_key
      .as_ref()
      .and_then(|key| identity_key(&d.basic_info.name, &d.basic_info.age, &d.basic_info.contact).map(|k| &k == key))
      .unwrap_or(false);
    let fallback_matched = if !matched {
      fallback
        .as_ref()
        .and_then(|key| fallback_identity_key(&d.basic_info.name, sf).map(|k| &k == key))
        .unwrap_or(false)
    } else {
      false
    };
    if matched || fallback_matched {
      tx
        .execute("DELETE FROM resume_library WHERE id = ?1", params![id])
        .map_err(|e| AppError::msg(format!("去重删除失败：{}", e)))?;
    }
  }

  let final_data = merged_data.unwrap_or(data);
  let record = ResumeRecord {
    id: make_id("resume"),
    created_at: now_epoch().to_string(),
    source_file,
    data: final_data,
  };
  let data_json = serde_json::to_string(&record.data)
    .map_err(|e| AppError::msg(format!("序列化简历数据失败：{}", e)))?;
  tx
    .execute(
      "INSERT INTO resume_library (id, created_at, source_file, data_json) VALUES (?1,?2,?3,?4)",
      params![record.id, record.created_at, record.source_file, data_json],
    )
    .map_err(|e| AppError::msg(format!("保存简历失败：{}", e)))?;

  tx
    .commit()
    .map_err(|e| AppError::msg(format!("提交事务失败：{}", e)))?;
  Ok(record)
}

pub fn delete_resume(id: String) -> Result<(), AppError> {
  if delete_resume_if_exists(id.trim())? {
    return Ok(());
  }
  Err(AppError::msg("未找到要删除的简历记录"))
}

pub fn delete_resumes(ids: Vec<String>) -> Result<usize, AppError> {
  let mut deleted = 0usize;
  let mut seen = HashSet::new();
  for raw_id in ids {
    let id = raw_id.trim();
    if id.is_empty() {
      continue;
    }
    if !seen.insert(id.to_string()) {
      continue;
    }
    if delete_resume_if_exists(id)? {
      deleted += 1;
    }
  }
  Ok(deleted)
}

fn delete_resume_if_exists(id: &str) -> Result<bool, AppError> {
  let conn = open_resumes_db()?;
  let target = {
    let mut stmt = conn
      .prepare("SELECT id, source_file, data_json FROM resume_library WHERE id = ?1")
      .map_err(|e| AppError::msg(format!("查询简历失败：{}", e)))?;
    match stmt.query_row(params![id.trim()], |row| {
      Ok(ResumeRecord {
        id: row.get(0)?,
        created_at: String::new(),
        source_file: row.get(1)?,
        data: serde_json::from_str(&row.get::<_, String>(2)?).unwrap_or_default(),
      })
    }) {
      Ok(r) => Some(r),
      Err(rusqlite::Error::QueryReturnedNoRows) => None,
      Err(e) => return Err(AppError::msg(format!("查询简历失败：{}", e))),
    }
  };
  let Some(target) = target else {
    return Ok(false);
  };

  let tx = conn
    .unchecked_transaction()
    .map_err(|e| AppError::msg(format!("开启事务失败：{}", e)))?;

  tx
    .execute("DELETE FROM resume_library WHERE id = ?1", params![id.trim()])
    .map_err(|e| AppError::msg(format!("删除简历失败：{}", e)))?;

  let candidate_name = target.data.basic_info.name.trim();
  let to_delete: Vec<(String, String, String)> = {
    let mut find_stmt = tx
      .prepare(
        "SELECT parsed_id, json_path, jd_screening_json_path FROM parsed_resumes
         WHERE resume_id = ?1
            OR (source_file = ?2 AND (?3 = '' OR candidate_name = ?3))",
      )
      .map_err(|e| AppError::msg(format!("查询关联解析记录失败：{}", e)))?;
    let result = find_stmt
      .query_map(
        params![id.trim(), target.source_file.trim(), candidate_name],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
      )
      .map_err(|e| AppError::msg(format!("读取关联解析记录失败：{}", e)))?
      .filter_map(|r| r.ok())
      .collect();
    result
  };

  for (parsed_id, json_path, jd_path) in &to_delete {
    if !json_path.trim().is_empty() {
      let _ = fs::remove_file(json_path);
      let jd_sib = jd_screening_sibling_path(Path::new(json_path));
      if jd_sib.exists() {
        let _ = fs::remove_file(&jd_sib);
      }
    }
    if !jd_path.trim().is_empty() {
      let _ = fs::remove_file(jd_path);
    }
    tx
      .execute("DELETE FROM parsed_resumes WHERE parsed_id = ?1", params![parsed_id])
      .map_err(|e| AppError::msg(format!("删除关联解析记录失败：{}", e)))?;
  }

  tx
    .commit()
    .map_err(|e| AppError::msg(format!("提交删除事务失败：{}", e)))?;
  Ok(true)
}

pub fn list_jds() -> Result<Vec<JdRecord>, AppError> {
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare("SELECT id, created_at, title, text FROM jd_records ORDER BY created_at DESC")
    .map_err(|e| AppError::msg(format!("查询 JD 库失败：{}", e)))?;
  let items = stmt
    .query_map([], |row| {
      Ok(JdRecord {
        id: row.get(0)?,
        created_at: row.get(1)?,
        title: row.get(2)?,
        text: row.get(3)?,
      })
    })
    .map_err(|e| AppError::msg(format!("读取 JD 库失败：{}", e)))?
    .filter_map(|r| r.ok())
    .collect();
  Ok(items)
}

pub fn save_jd(title: String, text: String) -> Result<JdRecord, AppError> {
  let conn = open_resumes_db()?;
  let record = JdRecord {
    id: make_id("jd"),
    created_at: now_epoch().to_string(),
    title,
    text,
  };
  conn
    .execute(
      "INSERT INTO jd_records (id, created_at, title, text) VALUES (?1,?2,?3,?4)",
      params![record.id, record.created_at, record.title, record.text],
    )
    .map_err(|e| AppError::msg(format!("保存 JD 失败：{}", e)))?;
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

/// 将当前 AI 配置写入 `app-config.json`（路径规则与 `load_settings` 一致）。
pub fn save_settings(settings: &AppSettings) -> Result<(), AppError> {
  let mut s = settings.clone();
  s.llm_provider = s.llm_provider.trim().to_ascii_lowercase();
  if s.llm_provider.is_empty() {
    s.llm_provider = "ollama".to_string();
  }
  s.model_path = s.model_path.trim().to_string();
  s.llama_cli_path = s.llama_cli_path.trim().to_string();
  s.llm_api_key = s.llm_api_key.trim().to_string();
  s.threads = s.threads.clamp(1, 64);
  s.temperature = s.temperature.clamp(0.0, 2.0);
  if let Some(n) = s.cloud_max_output_tokens {
    if n < 2048 {
      s.cloud_max_output_tokens = None;
    } else {
      s.cloud_max_output_tokens = Some(n.min(65536));
    }
  }
  let path = settings_path()?;
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  write_json(&path, &s)?;
  Ok(())
}

/// 当前将读写的 `app-config.json` 绝对路径（用于界面提示）。
pub fn app_settings_file_path() -> Result<String, AppError> {
  Ok(settings_path()?.to_string_lossy().into_owned())
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

fn upsert_parsed_resume_sqlite(record: &ParsedResultRecord, data: &ResumeData, jd_index: &JdScreeningIndex) -> Result<(), AppError> {
  let conn = open_resumes_db()?;
  let skills_text = record.skills.join(" ").to_ascii_lowercase();
  let skills_json = serde_json::to_string(&record.skills).unwrap_or_else(|_| "[]".to_string());
  let work_text = build_work_text(data).to_ascii_lowercase();
  let project_text = build_project_text(data).to_ascii_lowercase();
  let work_years_num = crate::jd::candidate_work_years_num(&record.work_years).unwrap_or(0.0);
  let degree_rank = crate::jd::degree_rank(&record.degree);
  let updated_at = now_epoch().to_string();
  let data_json = serde_json::to_string(data).unwrap_or_default();
  let jd_index_json = serde_json::to_string(jd_index).unwrap_or_default();
  conn
    .execute(
      "INSERT INTO parsed_resumes (
        parsed_id, resume_id, source_file, candidate_name, age, contact, position, degree, work_years, work_years_num,
        degree_rank,
        skills_text, skills_json, work_text, project_text, jd_screening_json_path, json_path, imported_date, updated_at,
        created_at, data_json, jd_screening_index_json
      ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
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
        degree_rank=excluded.degree_rank,
        skills_text=excluded.skills_text,
        skills_json=excluded.skills_json,
        work_text=excluded.work_text,
        project_text=excluded.project_text,
        jd_screening_json_path=excluded.jd_screening_json_path,
        json_path=excluded.json_path,
        imported_date=excluded.imported_date,
        updated_at=excluded.updated_at,
        created_at=excluded.created_at,
        data_json=excluded.data_json,
        jd_screening_index_json=excluded.jd_screening_index_json",
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
        degree_rank,
        skills_text,
        skills_json,
        work_text,
        project_text,
        record.jd_screening_json_path,
        record.json_path,
        record.imported_date,
        updated_at,
        record.created_at,
        data_json,
        jd_index_json,
      ],
    )
    .map_err(|e| AppError::msg(format!("写入 SQLite 失败：{}", e)))?;
  Ok(())
}

pub fn save_parsed_result_json(
  source_file: String,
  resume_id: String,
  data: ResumeData,
  jd_index: JdScreeningIndex,
) -> Result<ParsedResultRecord, AppError> {
  let candidate_name = data.basic_info.name.trim().to_string();
  let age = data.basic_info.age.trim().to_string();
  let contact = data.basic_info.contact.trim().to_string();
  let position = extract_primary_position(&data);
  let degree = extract_top_degree(&data);
  let work_years = calc_work_years(&data);
  let skills = extract_skills(&data);

  let record = ParsedResultRecord {
    id: make_id("parsed"),
    created_at: now_epoch().to_string(),
    imported_date: today_ymd(),
    resume_id: if resume_id.trim().is_empty() { None } else { Some(resume_id.clone()) },
    source_file: source_file.clone(),
    candidate_name: candidate_name.clone(),
    age: age.clone(),
    contact: contact.clone(),
    position,
    degree,
    work_years,
    skills,
    json_path: String::new(),
    jd_screening_json_path: String::new(),
  };

  let conn = open_resumes_db()?;
  let tx = conn
    .unchecked_transaction()
    .map_err(|e| AppError::msg(format!("开启事务失败：{}", e)))?;

  // Dedup: remove old matching records
  let new_identity = identity_key(&candidate_name, &age, &contact);
  let fallback_identity = fallback_identity_key(&candidate_name, &source_file);
  let has_resume_id = !resume_id.trim().is_empty();

  let existing: Vec<ParsedResultRecord> = {
    let mut find_stmt = tx
      .prepare(
        "SELECT parsed_id, resume_id, source_file, candidate_name, age, contact, json_path, jd_screening_json_path
         FROM parsed_resumes",
      )
      .map_err(|e| AppError::msg(format!("查询解析记录失败：{}", e)))?;
    let result = find_stmt
      .query_map([], |row| {
        Ok(ParsedResultRecord {
          id: row.get(0)?,
          created_at: String::new(),
          imported_date: String::new(),
          resume_id: row.get(1)?,
          source_file: row.get(2)?,
          candidate_name: row.get(3)?,
          age: row.get(4)?,
          contact: row.get(5)?,
          position: String::new(),
          degree: String::new(),
          work_years: String::new(),
          skills: Vec::new(),
          json_path: row.get(6)?,
          jd_screening_json_path: row.get(7)?,
        })
      })
      .map_err(|e| AppError::msg(format!("读取解析记录失败：{}", e)))?
      .filter_map(|r| r.ok())
      .collect();
    result
  };

  for old in &existing {
    let matched_resume_id = has_resume_id && old.resume_id.as_deref() == Some(resume_id.as_str());
    let matched_identity = new_identity
      .as_ref()
      .and_then(|key| parsed_record_identity_key(old).map(|k| &k == key))
      .unwrap_or(false);
    let matched_fallback = fallback_identity
      .as_ref()
      .and_then(|key| fallback_identity_key(&old.candidate_name, &old.source_file).map(|k| &k == key))
      .unwrap_or(false);
    if matched_resume_id || matched_identity || matched_fallback {
      // Clean up old JSON files if they exist (pre-migration data)
      if !old.json_path.trim().is_empty() {
        let _ = fs::remove_file(&old.json_path);
        let jd_sib = jd_screening_sibling_path(Path::new(&old.json_path));
        if jd_sib.exists() {
          let _ = fs::remove_file(&jd_sib);
        }
      }
      if !old.jd_screening_json_path.trim().is_empty() {
        let _ = fs::remove_file(&old.jd_screening_json_path);
      }
      tx
        .execute("DELETE FROM parsed_resumes WHERE parsed_id = ?1", params![old.id])
        .map_err(|e| AppError::msg(format!("删除旧解析记录失败：{}", e)))?;
    }
  }

  tx.commit()
    .map_err(|e| AppError::msg(format!("提交去重事务失败：{}", e)))?;

  upsert_parsed_resume_sqlite(&record, &data, &jd_index)?;
  Ok(record)
}

pub fn jd_score_from_local_parsed(jd_text: String) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare(
      "SELECT parsed_id, resume_id, source_file, candidate_name, age, contact,
              position, degree, work_years, skills_json, json_path, jd_screening_json_path,
              data_json
       FROM parsed_resumes
       WHERE data_json != ''",
    )
    .map_err(|e| AppError::msg(format!("查询解析记录失败：{}", e)))?;

  let rows = stmt
    .query_map([], |row| {
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
    })
    .map_err(|e| AppError::msg(format!("读取解析记录失败：{}", e)))?;

  let mut out: Vec<ParsedJdScoreRecord> = Vec::new();
  for row in rows {
    let (parsed_id, resume_id, source_file, candidate_name, age, contact,
         position, degree, work_years, skills_json, json_path, jd_screening_json_path,
         data_json) = row.map_err(|e| AppError::msg(format!("读取记录失败：{}", e)))?;

    let resume: ResumeData = match serde_json::from_str(&data_json) {
      Ok(v) => v,
      Err(_) => continue,
    };
    let skills: Vec<String> = serde_json::from_str(&skills_json).unwrap_or_default();
    let score = crate::jd::score_v1(&resume, &jd_text);
    let name = if candidate_name.trim().is_empty() {
      resume.basic_info.name.trim().to_string()
    } else {
      candidate_name
    };

    out.push(ParsedJdScoreRecord {
      parsed_id,
      resume_id,
      candidate_name: name,
      source_file,
      age,
      contact,
      position,
      degree,
      work_years,
      skills,
      json_path,
      jd_screening_json_path,
      score: score.score,
      score_breakdown: crate::schema::JdScoreBreakdown::default(),
      matched_keywords: score.matched_keywords,
      total_keywords: score.total_keywords,
    });
  }

  out.sort_by(|a, b| b.score.cmp(&a.score));
  Ok(out)
}

fn clamp_limit(limit: i32) -> usize {
  let n = if limit <= 0 { 10 } else { limit };
  (n as usize).min(200)
}

fn ai_extract_jd_requirements(position: &str, jd_text: &str, settings: &AppSettings) -> Result<JdStructuredRequirement, AppError> {
  let mut next_id = 0u32;
  let (pos_masked, m_pos) = mask_sensitive_segments(position.trim(), &mut next_id);
  let (jd_masked, m_jd) = mask_sensitive_segments(jd_text, &mut next_id);
  let mut priv_map = m_pos;
  priv_map.extend_map(m_jd);
  let guard = if priv_map.is_empty() { "" } else { LLM_PLACEHOLDER_GUARD };
  let prompt = format!(
    "你是 JD 结构化提取助手。请根据岗位名和JD，提取筛选所需结构化要求。\n"
  ) +
  "只返回 JSON，不要解释。JSON 格式：\n"
  + "{\"minDegreeRank\":0-4,\"minWorkYears\":number,\"requiredSkills\":[\"...\"],\"preferredSkills\":[\"...\"],\"workKeywords\":[\"...\"],\"projectKeywords\":[\"...\"]}\n"
  + "其中 minDegreeRank 映射：0不限,1大专,2本科,3硕士,4博士。\n"
  + "岗位："
  + &pos_masked
  + "\nJD：\n"
  + &jd_masked
  + guard;

  let llm = LlmSettings::from(settings);
  let json_text = complete_json_prompt(
    "jd_structured",
    &prompt,
    &llm,
    JsonPromptParams {
      temperature: 0.0,
      ollama_num_ctx: 4096,
      ollama_num_predict: None,
    },
  )
  .map_err(|e| AppError::msg(format!("提取 JD 结构化要求失败：{}", e)))?;
  let json_text = unmask_sensitive_segments(&json_text, &priv_map);
  let mut req: JdStructuredRequirement = serde_json::from_str(&json_text)
    .map_err(|e| AppError::msg(format!("JD 结构化 JSON 反序列化失败：{}", e)))?;

  req.min_degree_rank = req.min_degree_rank.clamp(0, 4);
  req.min_work_years = req.min_work_years.max(0.0);
  Ok(req)
}

#[derive(Clone, serde::Serialize)]
pub struct JdFilterProgressEvent {
  pub current: u32,
  pub total: u32,
  pub message: String,
  #[serde(default)]
  pub done: bool,
}

pub fn jd_filter_by_keywords_from_index(
  position: String,
  jd_text: String,
  limit: i32,
  rerank_pool: i32,
  progress: Option<Arc<dyn Fn(JdFilterProgressEvent) + Send + Sync>>,
) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let emit = |ev: JdFilterProgressEvent| {
    if let Some(p) = &progress {
      p(ev);
    }
  };
  let top_n = clamp_limit(limit);
  let rerank_pool = if (1..=200).contains(&rerank_pool) {
    rerank_pool as usize
  } else {
    25
  };
  let settings = load_settings()?;
  emit(JdFilterProgressEvent {
    current: 0,
    total: 0,
    message: "正在提取 JD 结构化要求…".into(),
    done: false,
  });
  let req = ai_extract_jd_requirements(&position, &jd_text, &settings)?;
  let valid_resume_ids: HashSet<String> = list_resumes()?
    .into_iter()
    .map(|x| x.id.trim().to_string())
    .filter(|x| !x.is_empty())
    .collect();
  let conn = open_resumes_db()?;

  let mut pre_candidates: Vec<ParsedJdScoreRecord> = Vec::new();
  let mut extra_data: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();
  let mut seen: HashSet<String> = HashSet::new();
  log_jd_prefilter_query_plan(&conn, req.min_work_years, req.min_degree_rank);
  let mut stmt = conn
    .prepare(
      "SELECT
        parsed_id, resume_id, source_file, candidate_name, age, contact, position, degree,
        work_years, skills_json, work_text, project_text, jd_screening_json_path, json_path,
        data_json, jd_screening_index_json
      FROM parsed_resumes
      WHERE (?1 <= 0 OR work_years_num >= ?1)
        AND (?2 <= 0 OR degree_rank >= ?2)",
    )
    .map_err(|e| AppError::msg(format!("准备 SQLite 查询失败：{}", e)))?;

  let rows = stmt
    .query_map(
      params![req.min_work_years, req.min_degree_rank],
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
          row.get::<_, String>(13)?,
          row.get::<_, String>(14)?,
          row.get::<_, String>(15)?,
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
      jd_screening_json_path,
      json_path,
      data_json,
      jd_screening_index_json,
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

    let pid = parsed_id.clone();
    extra_data.insert(pid.clone(), (data_json, jd_screening_index_json));
    pre_candidates.push(ParsedJdScoreRecord {
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
      jd_screening_json_path,
      score: score.total_score,
      score_breakdown: score.breakdown,
      matched_keywords: score.matched_keywords,
      total_keywords: score.total_keywords,
    });
  }

  pre_candidates.sort_by(|a, b| b.score.cmp(&a.score));
  // 关键词初筛后进入 HR 精排的候选上限：取排序后的前 rerank_pool 条，且全部做精排（默认 200，最高 200）。
  if pre_candidates.len() > rerank_pool {
    pre_candidates.truncate(rerank_pool);
  }

  let n = pre_candidates.len();
  let total = (1 + n as u32).max(1);
  emit(JdFilterProgressEvent {
    current: 1,
    total,
    message: format!("初筛完成，进入 HR 精排（{} 人）", n),
    done: false,
  });

  const HR_RERANK_CONCURRENCY: usize = 8;
  let candidates_mutex = Mutex::new(
    pre_candidates.into_iter().map(Some).collect::<Vec<Option<ParsedJdScoreRecord>>>(),
  );
  let shared_idx = AtomicUsize::new(0);
  let concurrency = HR_RERANK_CONCURRENCY.min(n).max(1);

  std::thread::scope(|s| {
    for _ in 0..concurrency {
      s.spawn(|| {
        loop {
          let i = shared_idx.fetch_add(1, Ordering::SeqCst);
          if i >= n {
            break;
          }
          let mut item = {
            let mut guard = candidates_mutex.lock().unwrap();
            guard[i].take().expect("item already taken")
          };

          let step = 2 + i as u32;
          emit(JdFilterProgressEvent {
            current: step,
            total,
            message: format!("HR 精排 {}/{}：{}", i + 1, n, item.candidate_name),
            done: false,
          });
          let fallback_score = item.score;
          let fallback_breakdown = item.score_breakdown.clone();
          let fallback_matched = item.matched_keywords.clone();
          let fallback_total_keywords = item.total_keywords;

          let data_tuple = extra_data.get(&item.parsed_id).cloned().unwrap_or_default();
          let data_json_str = &data_tuple.0;
          let jd_index_json_str = &data_tuple.1;

          let index_opt: Option<JdScreeningIndex> = if !jd_index_json_str.trim().is_empty() {
            match serde_json::from_str::<JdScreeningIndex>(jd_index_json_str) {
              Ok(idx)
                if !idx.summary_for_jd.trim().is_empty()
                  || !idx.skill_tags.is_empty()
                  || !idx.work_bullets.trim().is_empty()
                  || !idx.project_bullets.trim().is_empty() =>
              {
                Some(idx)
              }
              _ => None,
            }
          } else {
            None
          };

          let hr_ok = index_opt
            .as_ref()
            .and_then(|idx| ai_score_resume_hr_from_jd_index(idx, &position, &jd_text, &req, &settings).ok());

          if let Some((ai_total, ai_breakdown, ai_matched, ai_total_keywords)) = hr_ok {
            item.score = ai_total;
            item.score_breakdown = ai_breakdown;
            item.matched_keywords = ai_matched;
            item.total_keywords = ai_total_keywords;
          } else if !data_json_str.trim().is_empty() {
            if let Ok(resume) = serde_json::from_str::<ResumeData>(data_json_str) {
              if let Ok((ai_total, ai_breakdown, ai_matched, ai_total_keywords)) =
                ai_score_resume_hr(&resume, &position, &jd_text, &req, &settings)
              {
                item.score = ai_total;
                item.score_breakdown = ai_breakdown;
                item.matched_keywords = ai_matched;
                item.total_keywords = ai_total_keywords;
              } else {
                item.score = fallback_score;
                item.score_breakdown = fallback_breakdown;
                item.matched_keywords = fallback_matched;
                item.total_keywords = fallback_total_keywords;
              }
            } else {
              item.score = fallback_score;
              item.score_breakdown = fallback_breakdown;
              item.matched_keywords = fallback_matched;
              item.total_keywords = fallback_total_keywords;
            }
          } else {
            item.score = fallback_score;
            item.score_breakdown = fallback_breakdown;
            item.matched_keywords = fallback_matched;
            item.total_keywords = fallback_total_keywords;
          }

          let mut guard = candidates_mutex.lock().unwrap();
          guard[i] = Some(item);
        }
      });
    }
  });

  let mut out: Vec<ParsedJdScoreRecord> = candidates_mutex
    .into_inner()
    .unwrap()
    .into_iter()
    .filter_map(|x| x)
    .collect();

  out.sort_by(|a, b| b.score.cmp(&a.score));
  if out.len() > top_n {
    out.truncate(top_n);
  }
  emit(JdFilterProgressEvent {
    current: total,
    total,
    message: "筛选完成".into(),
    done: true,
  });
  Ok(out)
}

fn ai_score_resume_match_from_jd_index(
  idx: &JdScreeningIndex,
  jd_text: &str,
  settings: &AppSettings,
) -> Result<(i32, Vec<String>, usize), AppError> {
  let compact = serde_json::to_string(idx)
    .map_err(|e| AppError::msg(format!("序列化 JD 筛选索引失败：{}", e)))?;
  let mut next_id = 0u32;
  let (jd_masked, m1) = mask_sensitive_segments(jd_text, &mut next_id);
  let (compact_masked, m2) = mask_sensitive_segments(&compact, &mut next_id);
  let mut priv_map = m1;
  priv_map.extend_map(m2);
  let guard = if priv_map.is_empty() { "" } else { LLM_PLACEHOLDER_GUARD };
  let prompt = format!(
    "你是简历匹配评估助手。候选人信息以 **JD 筛选索引**（解析阶段预抽取的摘要，已省略部分细节）给出，请仅依据索引与 JD 做匹配评估，勿编造索引中未出现的内容。\n"
  ) +
  "输出要求：只返回 JSON，格式为 {\"score\":0-100,\"matchedKeywords\":[\"...\"],\"totalKeywords\":N}。\n"
  + "评分标准：技能匹配、项目相关性、工作经历相关性。\n"
  + &format!("JD:\n{}\n", jd_masked)
  + &format!("候选人 JD 筛选索引(JSON)：\n{}\n", compact_masked)
  + guard;

  let llm = LlmSettings::from(settings);
  let json_text = complete_json_prompt(
    "jd_model_score_idx",
    &prompt,
    &llm,
    JsonPromptParams {
      temperature: 0.0,
      ollama_num_ctx: 4096,
      ollama_num_predict: None,
    },
  )
  .map_err(|e| AppError::msg(format!("调用模型评分失败：{}", e)))?;
  let json_text = unmask_sensitive_segments(&json_text, &priv_map);
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

fn ai_score_resume_match(resume: &ResumeData, jd_text: &str, settings: &AppSettings) -> Result<(i32, Vec<String>, usize), AppError> {
  let resume_json = serde_json::to_string(resume)
    .map_err(|e| AppError::msg(format!("序列化简历失败：{}", e)))?;
  let mut next_id = 0u32;
  let (jd_masked, m1) = mask_sensitive_segments(jd_text, &mut next_id);
  let (resume_masked, m2) = mask_sensitive_segments(&resume_json, &mut next_id);
  let mut priv_map = m1;
  priv_map.extend_map(m2);
  let guard = if priv_map.is_empty() { "" } else { LLM_PLACEHOLDER_GUARD };

  let prompt = format!(
    "你是简历匹配评估助手。请根据岗位JD与候选人简历打分。\n"
  ) +
  "输出要求：只返回 JSON，格式为 {\"score\":0-100,\"matchedKeywords\":[\"...\"],\"totalKeywords\":N}。\n"
  + "评分标准：技能匹配、项目相关性、工作经历相关性。\n"
  + &format!("JD:\n{}\n", jd_masked)
  + &format!("简历(JSON):\n{}\n", resume_masked)
  + guard;

  let llm = LlmSettings::from(settings);
  let json_text = complete_json_prompt(
    "jd_model_score",
    &prompt,
    &llm,
    JsonPromptParams {
      temperature: 0.0,
      ollama_num_ctx: 4096,
      ollama_num_predict: None,
    },
  )
  .map_err(|e| AppError::msg(format!("调用模型评分失败：{}", e)))?;
  let json_text = unmask_sensitive_segments(&json_text, &priv_map);
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

fn format_jd_requirements(req: &JdStructuredRequirement) -> String {
  let mut parts = Vec::new();
  if !req.required_skills.is_empty() {
    parts.push(format!("  必备技能：{}", req.required_skills.join("、")));
  }
  if !req.preferred_skills.is_empty() {
    parts.push(format!("  加分技能：{}", req.preferred_skills.join("、")));
  }
  if !req.work_keywords.is_empty() {
    parts.push(format!("  工作相关关键词：{}", req.work_keywords.join("、")));
  }
  if !req.project_keywords.is_empty() {
    parts.push(format!("  项目相关关键词：{}", req.project_keywords.join("、")));
  }
  let degree_name = match req.min_degree_rank {
    4 => "博士",
    3 => "硕士",
    2 => "本科",
    1 => "大专",
    _ => "不限",
  };
  parts.push(format!("  最低学历：{}", degree_name));
  if req.min_work_years > 0.0 {
    parts.push(format!("  最低工作年限：{} 年", req.min_work_years));
  }
  if parts.is_empty() {
    return String::new();
  }
  format!("JD 结构化要求（作为评分依据）：\n{}", parts.join("\n"))
}

fn ai_score_resume_hr_from_jd_index(
  idx: &JdScreeningIndex,
  position: &str,
  jd_text: &str,
  req: &JdStructuredRequirement,
  settings: &AppSettings,
) -> Result<(i32, crate::schema::JdScoreBreakdown, Vec<String>, usize), AppError> {
  let compact = serde_json::to_string(idx)
    .map_err(|e| AppError::msg(format!("序列化 JD 筛选索引失败：{}", e)))?;
  let mut next_id = 0u32;
  let (pos_masked, m1) = mask_sensitive_segments(position.trim(), &mut next_id);
  let (jd_masked, m2) = mask_sensitive_segments(jd_text, &mut next_id);
  let (compact_masked, m3) = mask_sensitive_segments(&compact, &mut next_id);
  let mut priv_map = m1;
  priv_map.extend_map(m2);
  priv_map.extend_map(m3);
  let guard = if priv_map.is_empty() { "" } else { LLM_PLACEHOLDER_GUARD };
  let req_text = format_jd_requirements(req);
  let prompt = format!(
    "你是企业招聘 HR，正在进行批量候选人筛选，请保持评分尺度一致，逐项对照、分步评估。\n\
\n\
评估步骤：\n\
  1. 对照硬性条件，逐项检查候选人匹配情况，记录命中和短板\n\
  2. 按扣分规则计算各子项分数，再按权重合成总分\n\
\n\
━━━ 硬性筛选条件（评分优先依据）━━━\n\
{}\n\
\n\
岗位：{}\n\
JD：\n{}\n\
\n\
候选人 JD 筛选索引（仅预抽取摘要，以下字段未出现即视为该信息缺失，不得编造）：\n{}\n\
\n\
━━━ 扣分规则（各子项起始 100 分）━━━\n\
  skillScore：缺一项必备技能 -5 分，匹配一项加分技能 +3（上限 +15）；综合覆盖比例评定\n\
  yearsScore：达标 = 100，每少 1 年 -15，最低 0\n\
  degreeScore：达标 = 100，差一档 -25（例如要求本科、候选人大专 → 75）\n\
  workScore：逐项对照 JD 工作关键词，有明确匹配 +20/项，完全无匹配 = 0\n\
  projectScore：逐项对照 JD 项目关键词，有明确匹配 +20/项，完全无匹配 = 0\n\
\n\
总分 = skillScore×0.3 + yearsScore×0.2 + degreeScore×0.1 + workScore×0.2 + projectScore×0.2（四舍五入取整）\n\
\n\
最终评分锚定：\n\
  80-100：核心技能全覆盖，项目/经验高度对口，学历年限均达标 → 强烈推荐面试\n\
  60-79：大部分核心技能匹配，部分项目相关，个别项不足 → 可面试\n\
  40-59：仅少量技能或经验交集，多项目不匹配 → 一般不推荐\n\
  0-39：技能/经验基本不匹配或存在硬伤\n\
\n\
仅返回 JSON，格式严格如下：\n\
{{\n\
  \"totalScore\": 0-100,\n\
  \"breakdown\": {{\n\
    \"skillScore\": 0-100,\n\
    \"yearsScore\": 0-100,\n\
    \"degreeScore\": 0-100,\n\
    \"workScore\": 0-100,\n\
    \"projectScore\": 0-100\n\
  }},\n\
  \"rationale\": \"80 字以内评估依据，包含最关键的匹配点与短板\",\n\
  \"matchedKeywords\": [\"最多12条证据短语\"],\n\
  \"totalKeywords\": 5\n\
}}\n\
要求：分数为整数；rationale 必须填写并基于对照结果；不要输出 JSON 之外的任何文字。",
    req_text,
    pos_masked,
    jd_masked,
    compact_masked
  ) + guard;

  let llm = LlmSettings::from(settings);
  let json_text = complete_json_prompt(
    "jd_hr_score_idx",
    &prompt,
    &llm,
    JsonPromptParams {
      temperature: 0.0,
      ollama_num_ctx: 8192,
      ollama_num_predict: None,
    },
  )
  .map_err(|e| AppError::msg(format!("调用 HR 评估模型失败：{}", e)))?;
  let json_text = unmask_sensitive_segments(&json_text, &priv_map);
  let v: Value = serde_json::from_str(&json_text)
    .map_err(|e| AppError::msg(format!("HR 评估 JSON 解析失败：{}", e)))?;

  let read_score = |obj: &Value, key: &str| -> i32 {
    obj
      .get(key)
      .and_then(|x| x.as_i64())
      .unwrap_or(0)
      .clamp(0, 100) as i32
  };
  let total_score = read_score(&v, "totalScore");
  let b = v.get("breakdown").cloned().unwrap_or(Value::Null);
  let rationale = v
    .get("rationale")
    .and_then(|x| x.as_str())
    .unwrap_or("")
    .trim()
    .to_string();
  let breakdown = crate::schema::JdScoreBreakdown {
    skill_score: read_score(&b, "skillScore"),
    years_score: read_score(&b, "yearsScore"),
    degree_score: read_score(&b, "degreeScore"),
    work_score: read_score(&b, "workScore"),
    project_score: read_score(&b, "projectScore"),
    rationale,
  };
  let matched_keywords = v
    .get("matchedKeywords")
    .and_then(|x| x.as_array())
    .map(|arr| {
      arr
        .iter()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();
  let total_keywords = v.get("totalKeywords").and_then(|x| x.as_u64()).unwrap_or(5) as usize;

  Ok((total_score, breakdown, matched_keywords, total_keywords))
}

fn ai_score_resume_hr(
  resume: &ResumeData,
  position: &str,
  jd_text: &str,
  req: &JdStructuredRequirement,
  settings: &AppSettings,
) -> Result<(i32, crate::schema::JdScoreBreakdown, Vec<String>, usize), AppError> {
  let resume_json = serde_json::to_string(resume)
    .map_err(|e| AppError::msg(format!("序列化简历失败：{}", e)))?;
  let mut next_id = 0u32;
  let (pos_masked, m1) = mask_sensitive_segments(position.trim(), &mut next_id);
  let (jd_masked, m2) = mask_sensitive_segments(jd_text, &mut next_id);
  let (resume_masked, m3) = mask_sensitive_segments(&resume_json, &mut next_id);
  let mut priv_map = m1;
  priv_map.extend_map(m2);
  priv_map.extend_map(m3);
  let guard = if priv_map.is_empty() { "" } else { LLM_PLACEHOLDER_GUARD };
  let req_text = format_jd_requirements(req);
  let prompt = format!(
    "你是企业招聘 HR，正在进行批量候选人筛选，请保持评分尺度一致，逐项对照、分步评估。\n\
\n\
评估步骤：\n\
  1. 对照硬性条件，逐项检查候选人匹配情况，记录命中和短板\n\
  2. 按扣分规则计算各子项分数，再按权重合成总分\n\
\n\
━━━ 硬性筛选条件（评分优先依据）━━━\n\
{}\n\
\n\
岗位：{}\n\
JD：\n{}\n\
\n\
候选人简历(JSON)：\n{}\n\
\n\
━━━ 扣分规则（各子项起始 100 分）━━━\n\
  skillScore：缺一项必备技能 -5 分，匹配一项加分技能 +3（上限 +15）；综合覆盖比例评定\n\
  yearsScore：达标 = 100，每少 1 年 -15，最低 0\n\
  degreeScore：达标 = 100，差一档 -25（例如要求本科、候选人大专 → 75）\n\
  workScore：逐项对照 JD 工作关键词，有明确匹配 +20/项，完全无匹配 = 0\n\
  projectScore：逐项对照 JD 项目关键词，有明确匹配 +20/项，完全无匹配 = 0\n\
\n\
总分 = skillScore×0.3 + yearsScore×0.2 + degreeScore×0.1 + workScore×0.2 + projectScore×0.2（四舍五入取整）\n\
\n\
最终评分锚定：\n\
  80-100：核心技能全覆盖，项目/经验高度对口，学历年限均达标 → 强烈推荐面试\n\
  60-79：大部分核心技能匹配，部分项目相关，个别项不足 → 可面试\n\
  40-59：仅少量技能或经验交集，多项目不匹配 → 一般不推荐\n\
  0-39：技能/经验基本不匹配或存在硬伤\n\
\n\
仅返回 JSON，格式严格如下：\n\
{{\n\
  \"totalScore\": 0-100,\n\
  \"breakdown\": {{\n\
    \"skillScore\": 0-100,\n\
    \"yearsScore\": 0-100,\n\
    \"degreeScore\": 0-100,\n\
    \"workScore\": 0-100,\n\
    \"projectScore\": 0-100\n\
  }},\n\
  \"rationale\": \"80 字以内评估依据，包含最关键的匹配点与短板\",\n\
  \"matchedKeywords\": [\"最多12条证据短语\"],\n\
  \"totalKeywords\": 5\n\
}}\n\
要求：分数为整数；rationale 必须填写并基于对照结果；不要输出 JSON 之外的任何文字。",
    req_text,
    pos_masked,
    jd_masked,
    resume_masked
  ) + guard;

  let llm = LlmSettings::from(settings);
  let json_text = complete_json_prompt(
    "jd_hr_score",
    &prompt,
    &llm,
    JsonPromptParams {
      temperature: 0.0,
      ollama_num_ctx: 8192,
      ollama_num_predict: None,
    },
  )
  .map_err(|e| AppError::msg(format!("调用 HR 评估模型失败：{}", e)))?;
  let json_text = unmask_sensitive_segments(&json_text, &priv_map);
  let v: Value = serde_json::from_str(&json_text)
    .map_err(|e| AppError::msg(format!("HR 评估 JSON 解析失败：{}", e)))?;

  let read_score = |obj: &Value, key: &str| -> i32 {
    obj
      .get(key)
      .and_then(|x| x.as_i64())
      .unwrap_or(0)
      .clamp(0, 100) as i32
  };
  let total_score = read_score(&v, "totalScore");
  let b = v.get("breakdown").cloned().unwrap_or(Value::Null);
  let rationale = v
    .get("rationale")
    .and_then(|x| x.as_str())
    .unwrap_or("")
    .trim()
    .to_string();
  let breakdown = crate::schema::JdScoreBreakdown {
    skill_score: read_score(&b, "skillScore"),
    years_score: read_score(&b, "yearsScore"),
    degree_score: read_score(&b, "degreeScore"),
    work_score: read_score(&b, "workScore"),
    project_score: read_score(&b, "projectScore"),
    rationale,
  };
  let matched_keywords = v
    .get("matchedKeywords")
    .and_then(|x| x.as_array())
    .map(|arr| {
      arr
        .iter()
        .filter_map(|x| x.as_str().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
    })
    .unwrap_or_default();
  let total_keywords = v.get("totalKeywords").and_then(|x| x.as_u64()).unwrap_or(5) as usize;

  Ok((total_score, breakdown, matched_keywords, total_keywords))
}

pub fn jd_filter_by_model_from_parsed(_position: String, jd_text: String, limit: i32) -> Result<Vec<ParsedJdScoreRecord>, AppError> {
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare(
      "SELECT parsed_id, resume_id, source_file, candidate_name, age, contact,
              position, degree, work_years, skills_json, json_path, jd_screening_json_path,
              data_json, jd_screening_index_json
       FROM parsed_resumes
       WHERE data_json != ''",
    )
    .map_err(|e| AppError::msg(format!("查询解析记录失败：{}", e)))?;

  let rows = stmt
    .query_map([], |row| {
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
        row.get::<_, String>(13)?,
      ))
    })
    .map_err(|e| AppError::msg(format!("读取解析记录失败：{}", e)))?;

  let settings = load_settings()?;
  let top_n = clamp_limit(limit);

  let mut out: Vec<ParsedJdScoreRecord> = Vec::new();
  let mut seen: HashSet<String> = HashSet::new();
  for row in rows {
    let (parsed_id, resume_id, source_file, candidate_name, age, contact,
         position, degree, work_years, skills_json, json_path, jd_screening_json_path,
         data_json, jd_screening_index_json) =
      row.map_err(|e| AppError::msg(format!("读取记录失败：{}", e)))?;

    let skills: Vec<String> = serde_json::from_str(&skills_json).unwrap_or_default();

    let index_opt: Option<JdScreeningIndex> = if !jd_screening_index_json.trim().is_empty() {
      match serde_json::from_str::<JdScreeningIndex>(&jd_screening_index_json) {
        Ok(idx)
          if !idx.summary_for_jd.trim().is_empty()
            || !idx.skill_tags.is_empty()
            || !idx.work_bullets.trim().is_empty()
            || !idx.project_bullets.trim().is_empty() =>
        {
          Some(idx)
        }
        _ => None,
      }
    } else {
      None
    };

    let (score, matched_keywords, total_keywords) = if let Some(ref idx) = index_opt {
      match ai_score_resume_match_from_jd_index(idx, &jd_text, &settings) {
        Ok(v) => v,
        Err(_) => {
          let resume: ResumeData = match serde_json::from_str(&data_json) {
            Ok(v) => v,
            Err(_) => continue,
          };
          match ai_score_resume_match(&resume, &jd_text, &settings) {
            Ok(v) => v,
            Err(_) => continue,
          }
        }
      }
    } else {
      let resume: ResumeData = match serde_json::from_str(&data_json) {
        Ok(v) => v,
        Err(_) => continue,
      };
      match ai_score_resume_match(&resume, &jd_text, &settings) {
        Ok(v) => v,
        Err(_) => continue,
      }
    };

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
      position,
      degree,
      work_years,
      skills,
      json_path,
      jd_screening_json_path,
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

// ── Token 消耗追踪 ──

pub fn flush_token_usage() -> Result<(), AppError> {
  let records = drain_token_usage_log();
  if records.is_empty() {
    return Ok(());
  }
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare(
      "INSERT INTO token_usage (created_at, provider, model, label, prompt_tokens, completion_tokens, total_tokens)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
    )
    .map_err(|e| AppError::msg(format!("准备 token_usage 插入失败：{}", e)))?;
  for r in &records {
    stmt
      .execute(rusqlite::params![
        r.created_at,
        r.provider,
        r.model,
        r.label,
        r.prompt_tokens,
        r.completion_tokens,
        r.total_tokens,
      ])
      .map_err(|e| AppError::msg(format!("写入 token_usage 失败：{}", e)))?;
  }
  Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenStats {
  pub total_prompt_tokens: i64,
  pub total_completion_tokens: i64,
  pub total_tokens: i64,
  pub call_count: i64,
  pub by_provider: Vec<ProviderTokenStat>,
  pub recent: Vec<TokenUsageRecord>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTokenStat {
  pub provider: String,
  pub total_tokens: i64,
  pub call_count: i64,
}

pub fn get_token_stats(limit: usize) -> Result<TokenStats, AppError> {
  flush_token_usage()?;
  let conn = open_resumes_db()?;

  let (total_prompt_tokens, total_completion_tokens, total_tokens, call_count): (i64, i64, i64, i64) = conn
    .query_row(
      "SELECT COALESCE(SUM(prompt_tokens),0), COALESCE(SUM(completion_tokens),0), COALESCE(SUM(total_tokens),0), COUNT(*) FROM token_usage",
      [],
      |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )
    .unwrap_or((0, 0, 0, 0));

  let mut by_stmt = conn
    .prepare("SELECT provider, COALESCE(SUM(total_tokens),0), COUNT(*) FROM token_usage GROUP BY provider ORDER BY SUM(total_tokens) DESC")
    .map_err(|e| AppError::msg(format!("查询 token 按 provider 统计失败：{}", e)))?;
  let by_provider: Vec<ProviderTokenStat> = by_stmt
    .query_map([], |r| {
      Ok(ProviderTokenStat {
        provider: r.get(0)?,
        total_tokens: r.get(1)?,
        call_count: r.get(2)?,
      })
    })
    .map_err(|e| AppError::msg(format!("遍历 token provider 统计失败：{}", e)))?
    .filter_map(|x| x.ok())
    .collect();

  let limit = limit.clamp(1, 500);
  let mut rec_stmt = conn
    .prepare("SELECT created_at, provider, model, label, prompt_tokens, completion_tokens, total_tokens FROM token_usage ORDER BY id DESC LIMIT ?1")
    .map_err(|e| AppError::msg(format!("查询 token 日志失败：{}", e)))?;
  let recent: Vec<TokenUsageRecord> = rec_stmt
    .query_map([limit], |r| {
      Ok(TokenUsageRecord {
        created_at: r.get(0)?,
        provider: r.get(1)?,
        model: r.get(2)?,
        label: r.get(3)?,
        prompt_tokens: r.get(4)?,
        completion_tokens: r.get(5)?,
        total_tokens: r.get(6)?,
      })
    })
    .map_err(|e| AppError::msg(format!("遍历 token 日志失败：{}", e)))?
    .filter_map(|x| x.ok())
    .collect();

  Ok(TokenStats {
    total_prompt_tokens,
    total_completion_tokens,
    total_tokens,
    call_count,
    by_provider,
    recent,
  })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyTokenStat {
  pub date: String,
  pub total_tokens: i64,
  pub prompt_tokens: i64,
  pub completion_tokens: i64,
  pub call_count: i64,
}

pub fn get_token_daily_stats(days: usize) -> Result<Vec<DailyTokenStat>, AppError> {
  flush_token_usage()?;
  let conn = open_resumes_db()?;
  let days = days.clamp(1, 365) as i64;
  let mut stmt = conn
    .prepare(
      "SELECT date(CAST(created_at AS INTEGER), 'unixepoch') as day,
              COALESCE(SUM(total_tokens),0),
              COALESCE(SUM(prompt_tokens),0),
              COALESCE(SUM(completion_tokens),0),
              COUNT(*)
       FROM token_usage
       WHERE CAST(created_at AS INTEGER) > (CAST(strftime('%s','now') AS INTEGER) - ?1 * 86400)
       GROUP BY day
       ORDER BY day DESC",
    )
    .map_err(|e| AppError::msg(format!("查询每日 token 统计失败：{}", e)))?;
  let rows: Vec<DailyTokenStat> = stmt
    .query_map([days], |r| {
      Ok(DailyTokenStat {
        date: r.get(0)?,
        total_tokens: r.get(1)?,
        prompt_tokens: r.get(2)?,
        completion_tokens: r.get(3)?,
        call_count: r.get(4)?,
      })
    })
    .map_err(|e| AppError::msg(format!("遍历每日 token 统计失败：{}", e)))?
    .filter_map(|x| x.ok())
    .collect();
  Ok(rows)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelTokenStat {
  pub model: String,
  pub provider: String,
  pub total_tokens: i64,
  pub prompt_tokens: i64,
  pub completion_tokens: i64,
  pub call_count: i64,
}

pub fn get_token_model_stats() -> Result<Vec<ModelTokenStat>, AppError> {
  flush_token_usage()?;
  let conn = open_resumes_db()?;
  let mut stmt = conn
    .prepare(
      "SELECT model,
              COALESCE(provider, ''),
              COALESCE(SUM(total_tokens),0),
              COALESCE(SUM(prompt_tokens),0),
              COALESCE(SUM(completion_tokens),0),
              COUNT(*)
       FROM token_usage
       GROUP BY model
       ORDER BY SUM(total_tokens) DESC",
    )
    .map_err(|e| AppError::msg(format!("查询按模型 token 统计失败：{}", e)))?;
  let rows: Vec<ModelTokenStat> = stmt
    .query_map([], |r| {
      Ok(ModelTokenStat {
        model: r.get(0)?,
        provider: r.get(1)?,
        total_tokens: r.get(2)?,
        prompt_tokens: r.get(3)?,
        completion_tokens: r.get(4)?,
        call_count: r.get(5)?,
      })
    })
    .map_err(|e| AppError::msg(format!("遍历按模型 token 统计失败：{}", e)))?
    .filter_map(|x| x.ok())
    .collect();
  Ok(rows)
}

