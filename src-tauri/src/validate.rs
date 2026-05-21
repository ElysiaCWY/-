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

/// 安全网：如果 unmask 因不可见字符差异等原因未命中，清除残留的 __RM_PRIV_NNNN__ 占位符。
fn strip_remaining_priv_placeholders(raw: &str) -> String {
  let s = raw.trim();
  if s.is_empty() {
    return String::new();
  }
  // 匹配 __RM_PRIV_ 开头、四位数字、__ 结尾的占位符，及其附近可能的额外空白/标点。
  if let Ok(re) = Regex::new(r"__RM_PRIV_\d{4}__") {
    let cleaned = re.replace_all(s, "").to_string();
    let trimmed = cleaned.trim();
    if !trimmed.is_empty() {
      return trimmed.to_string();
    }
  }
  // 如果整个名字就是占位符本身（或清理后为空），返回空字符串
  if s.starts_with("__RM_PRIV_") {
    return String::new();
  }
  s.to_string()
}

/// 去掉姓名中误粘的电话号码、状态词、表头词（模型常把相邻字段合并到姓名）。
fn normalize_display_name(raw: &str) -> String {
  let mut s = raw.trim().to_string();
  if s.is_empty() {
    return s;
  }

  // 如果包含 @，大概率是邮箱地址被误填到姓名字段，直接清空
  if s.contains('@') {
    return String::new();
  }

  // 去掉常见的状态标记（含括号、分隔符等形式）
  // "朱在职" → "朱", "张三(在职)" → "张三", "李四 - 离职" → "李四"
  for pat in &["在职", "离职", "待业", "应届", "已离职", "找工作"] {
    // 带括号包裹
    for (l, r) in &[("（", "）"), ("(", ")"), ("【", "】"), ("[", "]")] {
      let wrapped = format!("{}{}{}", l, pat, r);
      s = s.replace(&wrapped, "");
    }
    // 带前导空格或分隔符
    for sep in &[" - ", " · ", " | ", "，", ",", " "] {
      if let Some(pos) = s.find(&format!("{}{}", sep, pat)) {
        s = s[..pos].to_string();
      }
    }
    // 紧贴后缀（无空格）
    if s.ends_with(pat) && s.as_str() != *pat {
      s = s[..s.len() - pat.len()].trim_end().to_string();
    }
  }

  // 如果像电话号码/数字串（数字占比 > 60%），清空
  let digit_hyphen = s.chars().filter(|c| c.is_ascii_digit() || *c == '-' || *c == ' ' || *c == '(' || *c == ')').count();
  if digit_hyphen as f64 / s.chars().count().max(1) as f64 > 0.6 {
    return String::new();
  }

  // 去掉仅剩单字指示词（如 "男"、"女"）
  if matches!(s.trim(), "男" | "女" | "Male" | "Female") {
    return String::new();
  }

  // 如果剩余内容主要是 ASCII 字符且不像是正常英文名（长短、大小写），则视为乱码清空
  let trimmed = s.trim();
  let ascii_ratio = trimmed.chars().filter(|c| c.is_ascii()).count() as f64 / trimmed.chars().count().max(1) as f64;
  let has_cjk = trimmed.chars().any(|c| ('\u{4E00}'..='\u{9FFF}').contains(&c) || ('\u{3400}'..='\u{4DBF}').contains(&c));
  // 纯 ASCII 但没有大写字母开头（如 "abcd"、"123abc"）→ 不是正常姓名
  if ascii_ratio > 0.9 && !has_cjk {
    let has_upper = trimmed.chars().any(|c| c.is_ascii_uppercase());
    let letter_ratio = trimmed.chars().filter(|c| c.is_ascii_alphabetic()).count() as f64 / trimmed.chars().count().max(1) as f64;
    // 字母占比低（< 50%）或完全没有大写 → 不像正常英文名
    if letter_ratio < 0.5 || !has_upper {
      return String::new();
    }
  }

  const SUFFIXES: &[&str] = &[
    " 性别", "性别", " Sex", " Gender",
    " 男", " 女",
    " Male", " Female",
    " 年龄", " 年纪",
    " 姓名", " 名字", " Name",
    " 电话", " 手机", " 联系方式", " 邮箱",
    " 先生", " 女士", " 小姐",
    " 评价", " 备注", " 说明",
    " 简历", " 个人简历",
    " 在线", " 离线",
    " 的简历",
  ];
  let mut changed = true;
  while changed {
    changed = false;
    let t = s.trim_end();
    for suf in SUFFIXES {
      if t.ends_with(suf) {
        s = t[..t.len() - suf.len()].trim_end().to_string();
        changed = true;
        break;
      }
    }
  }
  if let Ok(re) = Regex::new(r"\s+") {
    s = re.replace_all(s.trim(), " ").to_string();
  }
  s
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
  b.name = normalize_display_name(&b.name);
  b.name = strip_remaining_priv_placeholders(&b.name);
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

