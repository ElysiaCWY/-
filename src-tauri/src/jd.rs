use crate::schema::{JdScoreResult, ResumeData};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};

pub fn score_v1(resume: &ResumeData, jd_text: &str) -> JdScoreResult {
  let resume_text = flatten_resume(resume);
  let jd_keywords = extract_keywords(jd_text);

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

fn extract_keywords(jd: &str) -> Vec<String> {
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
  text_lower.contains(&kw_lower.to_lowercase())
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

