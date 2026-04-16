use crate::schema::{BasicInfo, EducationItem, ProjectItem, ResumeData, WorkItem};
use std::collections::BTreeMap;

pub fn normalize_resume(mut r: ResumeData) -> ResumeData {
  r.basic_info = normalize_basic(r.basic_info);
  r.work_experience = normalize_work_experience(r.work_experience);
  r.project_experience = normalize_indexed_map(r.project_experience, ProjectItem::default());
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
    let item = WorkItem {
      company: item.company.trim().to_string(),
      position: item.position.trim().to_string(),
      period: item.period.trim().to_string(),
      description: item.description.trim().to_string(),
    };

    if let Some(existing) = merged.iter_mut().find(|w| {
      w.company == item.company && w.position == item.position && w.period == item.period
    }) {
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
  b.age = b.age.trim().to_string();
  b.gender = b.gender.trim().to_string();
  b.skills = b
    .skills
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();
  b.certificates = b
    .certificates
    .into_iter()
    .map(|s| s.trim().to_string())
    .filter(|s| !s.is_empty())
    .collect();
  b
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

