use crate::schema::ResumeData;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PdfExportOptions {
  pub include_name: bool,
  pub include_gender: bool,
  pub include_age: bool,
  pub include_contact: bool,
  pub include_skills: bool,
  /// 自定义主题目录名（相对于项目根，如 "jsonresume-theme-flat" 或 "node_modules/jsonresume-theme-elegant"）。空字符串表示使用内置默认主题。
  #[serde(default)]
  pub theme_dir: String,
}

impl Default for PdfExportOptions {
  fn default() -> Self {
    Self {
      include_name: true,
      include_gender: true,
      include_age: true,
      include_contact: true,
      include_skills: true,
      theme_dir: String::new(),
    }
  }
}

fn clean_inline(v: &str) -> String {
  v.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

fn safe(v: &str) -> String {
  let s = clean_inline(v);
  if s.is_empty() { "-".to_string() } else { s }
}

pub fn resume_data_to_json_resume(resume: &ResumeData, options: &PdfExportOptions) -> Value {
  let b = &resume.basic_info;

  let work = {
    let mut keys = resume.work_experience.keys().cloned().collect::<Vec<_>>();
    keys.sort_by(|a, b| a.parse::<i32>().unwrap_or(0).cmp(&b.parse::<i32>().unwrap_or(0)));
    keys
      .iter()
      .filter_map(|k| resume.work_experience.get(k))
      .filter(|w| !w.company.trim().is_empty() || !w.position.trim().is_empty() || !w.period.trim().is_empty() || !w.description.trim().is_empty())
      .map(|w| {
        json!({
          "name": safe(&w.company),
          "position": safe(&w.position),
          "startDate": "",
          "endDate": safe(&w.period),
          "summary": clean_inline(&w.description),
          "highlights": []
        })
      })
      .collect::<Vec<_>>()
  };

  let education = b
    .education
    .iter()
    .filter(|e| !e.school.trim().is_empty() || !e.degree.trim().is_empty() || !e.major.trim().is_empty() || !e.period.trim().is_empty())
    .map(|e| {
      json!({
        "institution": safe(&e.school),
        "area": safe(&e.major),
        "studyType": safe(&e.degree),
        "startDate": "",
        "endDate": safe(&e.period),
        "score": "",
        "courses": []
      })
    })
    .collect::<Vec<_>>();

  let projects = {
    let mut keys = resume.project_experience.keys().cloned().collect::<Vec<_>>();
    keys.sort_by(|a, b| a.parse::<i32>().unwrap_or(0).cmp(&b.parse::<i32>().unwrap_or(0)));
    keys
      .iter()
      .filter_map(|k| resume.project_experience.get(k))
      .filter(|p| !p.project_name.trim().is_empty() || !p.project_description.trim().is_empty() || !p.project_achievements.trim().is_empty())
      .map(|p| {
        let mut highlights = Vec::<String>::new();
        let ach = clean_inline(&p.project_achievements);
        if !ach.is_empty() {
          highlights.push(ach);
        }
        json!({
          "name": safe(&p.project_name),
          "description": clean_inline(&p.project_description),
          "highlights": highlights,
          "keywords": [],
          "startDate": "",
          "endDate": "",
          "url": ""
        })
      })
      .collect::<Vec<_>>()
  };

  let skills = if options.include_skills {
    let list = b
      .skills
      .iter()
      .map(|x| clean_inline(x))
      .filter(|x| !x.is_empty())
      .map(|x| json!({"name": x, "level": "", "keywords": []}))
      .collect::<Vec<_>>();
    list
  } else {
    Vec::new()
  };

  json!({
    "basics": {
      "name": if options.include_name { safe(&b.name) } else { "".to_string() },
      "label": "",
      "xGender": if options.include_gender { safe(&b.gender) } else { "".to_string() },
      "xAge": if options.include_age { safe(&b.age) } else { "".to_string() },
      "image": "",
      "email": "",
      "phone": if options.include_contact { safe(&b.contact) } else { "".to_string() },
      "url": "",
      "summary": "",
      "location": {
        "address": "",
        "postalCode": "",
        "city": "",
        "countryCode": "CN",
        "region": ""
      },
      "profiles": []
    },
    "work": work,
    "education": education,
    "skills": skills,
    "projects": projects,
    "certificates": b
      .certificates
      .iter()
      .map(|c| json!({"name": clean_inline(c), "date": "", "issuer": "", "url": ""}))
      .collect::<Vec<_>>(),
    "languages": [],
    "interests": [],
    "references": [],
    "meta": {
      "canonical": "",
      "version": "v1.0.0",
      "lastModified": ""
    }
  })
}
