use crate::schema::{BasicInfo, EducationItem, ProjectItem, ResumeData, WorkItem};
use regex::Regex;
use std::collections::BTreeMap;
use time::OffsetDateTime;

pub fn normalize_resume(mut r: ResumeData) -> ResumeData {
  r.work_experience = normalize_work_experience(r.work_experience);
  r.project_experience = normalize_indexed_map(r.project_experience, ProjectItem::default());
  r.basic_info = normalize_basic(r.basic_info);
  r.basic_info.skills = merge_skill_sources(
    r.basic_info.skills,
    &r.work_experience,
    &r.project_experience,
  );
  r
}

fn normalize_work_experience(m: BTreeMap<String, WorkItem>) -> BTreeMap<String, WorkItem> {
  let mut ordered: Vec<(usize, WorkItem)> = Vec::new();
  for (k, v) in m {
    let idx = k.parse::<usize>().unwrap_or(9999);
    ordered.push((idx, v));
  }
  ordered.sort_by_key(|(idx, _)| *idx);

  let mut merged: Vec<WorkItem> = Vec::new();
  for (_, item) in ordered {
    let item = normalize_work_item(item);

    if let Some(existing) = merged.iter_mut().find(|w| w.company == item.company) {
      existing.position = merge_unique_text(&existing.position, &item.position, " / ");
      existing.period = merge_unique_text(&existing.period, &item.period, "；");
      existing.description = merge_description(&existing.description, &item.description);
    } else {
      merged.push(item);
    }
  }

  let mut out: BTreeMap<String, WorkItem> = BTreeMap::new();
  if merged.is_empty() {
    out.insert("1".to_string(), WorkItem::default());
    return out;
  }

  for (i, item) in merged.into_iter().enumerate() {
    out.insert((i + 1).to_string(), item);
  }
  out
}

fn normalize_work_item(item: WorkItem) -> WorkItem {
  WorkItem {
    company: item.company.trim().to_string(),
    position: item.position.trim().to_string(),
    period: normalize_period(&item.period),
    description: item.description.trim().to_string(),
  }
}

fn normalize_period(raw: &str) -> String {
  let mut s = raw.trim().replace('\u{00A0}', " ");

  // 去掉括号中的“持续时长”说明，例如：(8个月) / （8 个月）
  if let Some(idx) = s.find('（') {
    s = s[..idx].trim().to_string();
  }
  if let Some(idx) = s.find('(') {
    s = s[..idx].trim().to_string();
  }

  // 归一空白，避免出现多余空格
  if let Ok(space_re) = Regex::new(r"\s+") {
    s = space_re.replace_all(&s, " ").to_string();
  }

  // 去掉尾部裸露的时长文本，例如："2015.01-2015.09 8 个月"
  if let Ok(duration_re) = Regex::new(r"\s*(\d+\s*年(\s*\d+\s*个?月)?|\d+\s*个?月(\s*\d+\s*天)?)\s*$") {
    s = duration_re.replace(&s, "").trim().to_string();
  }

  s
}

fn merge_unique_text(existing: &str, incoming: &str, separator: &str) -> String {
  let existing = existing.trim();
  let incoming = incoming.trim();

  if existing.is_empty() {
    return incoming.to_string();
  }
  if incoming.is_empty() {
    return existing.to_string();
  }
  if existing == incoming {
    return existing.to_string();
  }

  let mut parts: Vec<String> = Vec::new();
  for part in existing.split(separator).chain(incoming.split(separator)) {
    let text = part.trim();
    if text.is_empty() {
      continue;
    }
    if !parts.iter().any(|x| x == text) {
      parts.push(text.to_string());
    }
  }

  if parts.is_empty() {
    String::new()
  } else {
    parts.join(separator)
  }
}

fn merge_description(a: &str, b: &str) -> String {
  if a.is_empty() {
    return b.to_string();
  }
  if b.is_empty() {
    return a.to_string();
  }
  if a.contains(b) {
    return a.to_string();
  }
  if b.contains(a) {
    return b.to_string();
  }
  format!("{}\n{}", a, b)
}

fn normalize_basic(mut b: BasicInfo) -> BasicInfo {
  if b.education.is_empty() {
    b.education.push(EducationItem::default());
  } else {
    for e in &mut b.education {
      *e = EducationItem {
        school: e.school.trim().to_string(),
        major: e.major.trim().to_string(),
        degree: e.degree.trim().to_string(),
        period: e.period.trim().to_string(),
      };
    }
  }
  b.name = b.name.trim().to_string();
  b.age = normalize_age(&b.age);
  b.contact = b.contact.trim().to_string();
  b.gender = b.gender.trim().to_string();
  b.skills = b
    .skills
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>();
  b.skills = dedup_skill_list(b.skills);
  b.certificates = b
    .certificates
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();
  b
}

fn merge_skill_sources(
  explicit_skills: Vec<String>,
  work: &BTreeMap<String, WorkItem>,
  project: &BTreeMap<String, ProjectItem>,
) -> Vec<String> {
  let mut merged = dedup_skill_list(explicit_skills);

  let mut text_parts: Vec<String> = Vec::new();
  for item in work.values() {
    text_parts.push(item.position.clone());
    text_parts.push(item.description.clone());
  }
  for item in project.values() {
    text_parts.push(item.project_name.clone());
    text_parts.push(item.project_description.clone());
    text_parts.push(item.project_achievements.clone());
  }
  let corpus = text_parts.join("\n").to_ascii_lowercase();

  for skill in infer_skills_from_text(&corpus) {
    if !contains_skill(&merged, &skill) {
      merged.push(skill);
    }
  }

  merged
}

fn dedup_skill_list(skills: Vec<String>) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for skill in skills {
    let s = skill.trim();
    if s.is_empty() {
      continue;
    }
    if !contains_skill(&out, s) {
      out.push(s.to_string());
    }
  }
  out
}

fn contains_skill(skills: &[String], candidate: &str) -> bool {
  let c = candidate.trim();
  if c.is_empty() {
    return false;
  }
  let c_lower = c.to_ascii_lowercase();
  skills.iter().any(|s| s.trim().to_ascii_lowercase() == c_lower)
}

fn infer_skills_from_text(text_lower: &str) -> Vec<String> {
  let dict: [(&str, &[&str]); 20] = [
    ("Java", &["java"]),
    ("Golang", &["golang", "go语言", "go 开发"]),
    ("Python", &["python"]),
    ("JavaScript", &["javascript"]),
    ("TypeScript", &["typescript"]),
    ("Vue", &["vue"]),
    ("React", &["react"]),
    ("Node.js", &["node.js", "nodejs"]),
    ("SpringBoot", &["springboot", "spring boot"]),
    ("SpringCloud", &["springcloud", "spring cloud"]),
    ("MyBatisPlus", &["mybatisplus", "mybatis-plus"]),
    ("MySQL", &["mysql"]),
    ("Redis", &["redis"]),
    ("Kafka", &["kafka"]),
    ("Elasticsearch", &["elasticsearch"]),
    ("Docker", &["docker"]),
    ("K8S", &["k8s", "kubernetes"]),
    ("Netty", &["netty"]),
    ("C#", &["c#", "csharp", ".net"]),
    ("C++", &["c++"]),
  ];

  let mut out: Vec<String> = Vec::new();
  for (skill, aliases) in dict {
    if aliases.iter().any(|a| text_lower.contains(&a.to_ascii_lowercase())) {
      out.push(skill.to_string());
    }
  }
  out
}

fn normalize_age(age_raw: &str) -> String {
  let s = age_raw.trim();
  if s.is_empty() {
    return String::new();
  }

  if s.contains('岁') {
    return s.to_string();
  }

  let now_year = OffsetDateTime::now_utc().year();

  if let Ok(n) = s.parse::<i32>() {
    if (16..=80).contains(&n) {
      return n.to_string();
    }
    if (1900..=now_year).contains(&n) {
      return (now_year - n).max(0).to_string();
    }
  }

  if let Some(year) = extract_birth_year_like(s) {
    if (1900..=now_year).contains(&year) {
      return (now_year - year).max(0).to_string();
    }
  }

  s.to_string()
}

fn extract_birth_year_like(s: &str) -> Option<i32> {
  let trimmed = s.trim();
  if trimmed.len() < 4 {
    return None;
  }

  let chars: Vec<char> = trimmed.chars().collect();
  if chars.len() < 4 || !chars[0..4].iter().all(|c| c.is_ascii_digit()) {
    return None;
  }

  // 仅接受形如 YYYY、YYYY.MM、YYYY-MM、YYYY/MM、YYYY年MM月(DD日)
  let year = chars[0..4].iter().collect::<String>().parse::<i32>().ok()?;
  let rest = chars[4..].iter().collect::<String>();
  if rest.is_empty() {
    return Some(year);
  }

  let allowed = |c: char| c.is_ascii_digit() || matches!(c, '.' | '-' | '/' | '年' | '月' | '日' | ' ');
  if rest.chars().all(allowed) {
    return Some(year);
  }

  None
}

fn normalize_indexed_map<T: Clone>(m: BTreeMap<String, T>, default_item: T) -> BTreeMap<String, T> {
  let mut items: Vec<(usize, T)> = Vec::new();
  for (k, v) in m {
    let idx = k.parse::<usize>().unwrap_or(9999);
    items.push((idx, v));
  }
  items.sort_by_key(|(idx, _)| *idx);

  let mut out: BTreeMap<String, T> = BTreeMap::new();
  let mut next = 1usize;
  for (_, v) in items {
    out.insert(next.to_string(), v);
    next += 1;
  }
  if out.is_empty() {
    out.insert("1".to_string(), default_item);
  }
  out
}

