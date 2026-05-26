use crate::schema::{JdScoreBreakdown, JdScoreResult, ResumeData};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

pub fn score_v1(resume: &ResumeData, jd_text: &str) -> JdScoreResult {
  let resume_text = flatten_resume(resume);
  let jd_keywords = extract_keywords_v1(jd_text);

  let mut matched = Vec::new();
  let mut score = 0i32;

  for kw in &jd_keywords {
    if contains_keyword(&resume_text, kw) {
      matched.push(kw.clone());
      score += weight_of(kw);
    }
  }

  JdScoreResult {
    score,
    matched_keywords: matched,
    total_keywords: jd_keywords.len(),
  }
}

fn flatten_resume(r: &ResumeData) -> String {
  let mut parts = Vec::new();
  parts.push(r.basic_info.name.clone());
  parts.push(r.basic_info.age.clone());
  parts.push(r.basic_info.gender.clone());
  for e in &r.basic_info.education {
    parts.push(e.school.clone());
    parts.push(e.major.clone());
    parts.push(e.degree.clone());
    parts.push(e.period.clone());
  }
  parts.extend(r.basic_info.skills.clone());
  parts.extend(r.basic_info.certificates.clone());
  for (_, w) in &r.work_experience {
    parts.push(w.company.clone());
    parts.push(w.position.clone());
    parts.push(w.period.clone());
    parts.push(w.description.clone());
  }
  for (_, p) in &r.project_experience {
    parts.push(p.project_name.clone());
    parts.push(p.project_description.clone());
    parts.push(p.project_achievements.clone());
  }
  parts.join("\n").to_lowercase()
}

pub fn extract_keywords_v1(jd: &str) -> Vec<String> {
  // v1：非常轻量的关键词抽取：
  // - 英文/数字 token
  // - 常见技术词（含 C++ / C# 之类符号）
  let jd = jd.replace("\r\n", "\n");
  let mut set = BTreeSet::new();

  let re = Regex::new(r"(?i)\b[a-z][a-z0-9\+\#\.\-]{1,30}\b").unwrap();
  for cap in re.captures_iter(&jd) {
    let t = cap.get(0).unwrap().as_str().to_lowercase();
    if t.len() >= 2 {
      set.insert(t);
    }
  }

  // 中文：提取一些常见分隔后的片段（简化版）
  for line in jd.lines() {
    for chunk in line.split(|c: char| "，。；、/|:：,;()（）[]【】 \t".contains(c)) {
      let c = chunk.trim();
      if c.chars().count() >= 2 && c.chars().count() <= 10 {
        // 过滤掉纯数字
        if c.chars().all(|x| x.is_ascii_digit()) {
          continue;
        }
        set.insert(c.to_lowercase());
      }
    }
  }

  // 控制规模，避免低配下过多匹配
  set.into_iter().take(80).collect()
}

fn contains_keyword(text_lower: &str, kw_lower: &str) -> bool {
  word_match(text_lower, &kw_lower.to_lowercase())
}

/// 判断是否为 CJK 字符（中文、日文汉字等）。
fn is_cjk(c: char) -> bool {
  matches!(c,
    '\u{4E00}'..='\u{9FFF}'
    | '\u{3400}'..='\u{4DBF}'
    | '\u{F900}'..='\u{FAFF}'
    | '\u{2E80}'..='\u{2FDF}'
  )
}

/// 对字符串中连续的 CJK 字符序列提取 bigram。
fn cjk_bigrams(s: &str) -> Vec<String> {
  let cjk_only: Vec<char> = s.chars().filter(|&c| is_cjk(c)).collect();
  if cjk_only.len() < 2 {
    return cjk_only.iter().map(|c| c.to_string()).collect();
  }
  cjk_only.windows(2).map(|w| w.iter().collect()).collect()
}

/// 英文关键词用词边界匹配（避免 Java 命中 JavaScript）。
fn match_english_keyword(text_lower: &str, kw_lower: &str) -> bool {
  let mut start = 0usize;
  while let Some(pos) = text_lower[start..].find(kw_lower) {
    let abs = start + pos;
    let before_ok = abs == 0 || {
      let b = text_lower.as_bytes()[abs - 1];
      !b.is_ascii_alphanumeric()
    };
    let after_pos = abs + kw_lower.len();
    let after_ok = after_pos >= text_lower.len() || {
      let b = text_lower.as_bytes()[after_pos];
      !b.is_ascii_alphanumeric()
    };
    if before_ok && after_ok {
      return true;
    }
    start = abs + 1;
  }
  false
}

/// 统一关键词匹配：英文走词边界，中文走 bigram 重叠度。
fn word_match(text: &str, kw: &str) -> bool {
  let kw_trim = kw.trim();
  if kw_trim.is_empty() {
    return false;
  }
  let text_lower = text.to_ascii_lowercase();

  // 包含英文字母 → 词边界匹配
  if kw_trim.chars().any(|c| c.is_ascii_alphabetic()) {
    return match_english_keyword(&text_lower, &kw_trim.to_ascii_lowercase());
  }

  // 纯 CJK 关键词 → bigram 重叠度匹配
  if kw_trim.chars().all(|c| is_cjk(c) || c.is_ascii_whitespace()) {
    let kw_clean: String = kw_trim.chars().filter(|&c| !c.is_ascii_whitespace()).collect();
    if kw_clean.chars().count() <= 1 {
      return text.contains(&kw_clean);
    }
    let kw_bg = cjk_bigrams(&kw_clean);
    let tx_bg = cjk_bigrams(text);
    if kw_bg.is_empty() {
      return text.contains(&kw_clean);
    }
    let matched = kw_bg.iter().filter(|b| tx_bg.contains(b)).count();
    let ratio = matched as f32 / kw_bg.len() as f32;
    return ratio >= 0.6;
  }

  // 混合（如 "C++开发"）回退到子串匹配
  text_lower.contains(&kw_trim.to_ascii_lowercase())
}

fn weight_of(kw: &str) -> i32 {
  // v1 权重：更偏向技术关键词
  let mut weights: BTreeMap<&'static str, i32> = BTreeMap::new();
  for k in ["java", "python", "golang", "go", "rust", "c++", "c#", "javascript", "typescript"] {
    weights.insert(k, 6);
  }
  for k in ["spring", "django", "flask", "react", "vue", "node", "nodejs", "kubernetes", "docker"] {
    weights.insert(k, 5);
  }
  for k in ["mysql", "postgres", "redis", "mongodb", "elasticsearch", "kafka", "spark"] {
    weights.insert(k, 4);
  }

  let k = kw.to_lowercase();
  *weights.get(k.as_str()).unwrap_or(&2)
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JdStructuredRequirement {
  pub min_degree_rank: i32,
  pub min_work_years: f32,
  pub required_skills: Vec<String>,
  pub preferred_skills: Vec<String>,
  pub work_keywords: Vec<String>,
  pub project_keywords: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ResumeStructuredForScore {
  pub degree: String,
  pub work_years: String,
  pub skills: Vec<String>,
  pub work_text: String,
  pub project_text: String,
}

#[derive(Debug, Clone, Default)]
pub struct StructuredScoreResult {
  pub total_score: i32,
  pub breakdown: JdScoreBreakdown,
  pub matched_keywords: Vec<String>,
  pub total_keywords: usize,
}

pub fn degree_rank(s: &str) -> i32 {
  let v = s.trim().to_ascii_lowercase();
  if v.contains("博士") || v.contains("phd") {
    return 4;
  }
  if v.contains("硕士") || v.contains("研究生") || v.contains("master") {
    return 3;
  }
  if v.contains("本科") || v.contains("学士") || v.contains("bachelor") {
    return 2;
  }
  if v.contains("大专") || v.contains("专科") || v.contains("college") {
    return 1;
  }
  0
}

pub fn candidate_work_years_num(s: &str) -> Option<f32> {
  let v = s.trim();
  if v.is_empty() {
    return None;
  }
  if v.contains("1年以下") {
    return Some(0.5);
  }
  if let Ok(re) = Regex::new(r"(\d+(?:\.\d+)?)") {
    if let Some(cap) = re.captures(v) {
      return cap.get(1).and_then(|m| m.as_str().parse::<f32>().ok());
    }
  }
  None
}

pub fn score_structured_resume(req: &JdStructuredRequirement, resume: &ResumeStructuredForScore) -> StructuredScoreResult {
  let mut matched = BTreeSet::new();
  let mut total_keywords = 0usize;

  let skill_pool = normalize_items(&resume.skills).join(" ");
  let work_text = resume.work_text.trim().to_ascii_lowercase();
  let project_text = resume.project_text.trim().to_ascii_lowercase();

  let required_skills = normalize_items(&req.required_skills);
  let preferred_skills = normalize_items(&req.preferred_skills);
  let work_keywords = normalize_items(&req.work_keywords);
  let project_keywords = normalize_items(&req.project_keywords);

  total_keywords += required_skills.len() + preferred_skills.len() + work_keywords.len() + project_keywords.len();
  if total_keywords == 0 {
    total_keywords = 1;
  }

  let req_hit = hit_count(&skill_pool, &required_skills, &mut matched);
  let pref_hit = hit_count(&skill_pool, &preferred_skills, &mut matched);
  let work_hit = hit_count(&work_text, &work_keywords, &mut matched);
  let project_hit = hit_count(&project_text, &project_keywords, &mut matched);

  let skill_total = required_skills.len() + preferred_skills.len();
  let skill_ratio = if skill_total == 0 {
    0.5
  } else {
    ((req_hit as f32) * 1.2 + pref_hit as f32) / ((required_skills.len() as f32) * 1.2 + preferred_skills.len() as f32 + 1e-6)
  }
  .clamp(0.0, 1.0);

  let years_score_raw = score_years(req.min_work_years, candidate_work_years_num(&resume.work_years).unwrap_or(0.0));
  let degree_score_raw = score_degree(req.min_degree_rank, degree_rank(&resume.degree));
  let work_score_raw = ratio_or_default(work_hit, work_keywords.len(), 0.5);
  let project_score_raw = ratio_or_default(project_hit, project_keywords.len(), 0.5);

  let breakdown = JdScoreBreakdown {
    skill_score: (skill_ratio * 100.0).round() as i32,
    years_score: years_score_raw,
    degree_score: degree_score_raw,
    work_score: (work_score_raw * 100.0).round() as i32,
    project_score: (project_score_raw * 100.0).round() as i32,
    rationale: String::new(),
  };

  let total = (breakdown.skill_score as f32) * 0.3
    + (breakdown.years_score as f32) * 0.1
    + (breakdown.degree_score as f32) * 0.1
    + (breakdown.work_score as f32) * 0.25
    + (breakdown.project_score as f32) * 0.25;

  StructuredScoreResult {
    total_score: total.round().clamp(0.0, 100.0) as i32,
    breakdown,
    matched_keywords: matched.into_iter().collect(),
    total_keywords,
  }
}

fn normalize_items(items: &[String]) -> Vec<String> {
  let mut out = Vec::new();
  for item in items {
    let t = item.trim().to_ascii_lowercase();
    if t.is_empty() {
      continue;
    }
    if !out.iter().any(|x| x == &t) {
      out.push(t);
    }
  }
  out
}

fn hit_count(text: &str, kws: &[String], matched: &mut BTreeSet<String>) -> usize {
  let mut count = 0usize;
  for kw in kws {
    if word_match(text, kw) {
      count += 1;
      matched.insert(kw.clone());
    }
  }
  count
}

fn ratio_or_default(hit: usize, total: usize, default_v: f32) -> f32 {
  if total == 0 {
    return default_v;
  }
  (hit as f32 / total as f32).clamp(0.0, 1.0)
}

fn score_years(req_years: f32, candidate_years: f32) -> i32 {
  if req_years <= 0.0 {
    return 100;
  }
  // 平滑函数：在 0.8 倍附近开始明显加分，达到/超过要求时逐渐趋近满分。
  let ratio = (candidate_years / req_years).max(0.0).min(1.8);
  let z = 8.0 * (ratio - 0.8);
  let score = 100.0 / (1.0 + (-z).exp());
  score.round().clamp(0.0, 100.0) as i32
}

fn score_degree(req_rank: i32, candidate_rank: i32) -> i32 {
  if req_rank <= 0 {
    return 100;
  }
  if candidate_rank >= req_rank {
    return 100;
  }
  if candidate_rank + 1 == req_rank {
    return 70;
  }
  if candidate_rank + 2 == req_rank {
    return 40;
  }
  20
}

