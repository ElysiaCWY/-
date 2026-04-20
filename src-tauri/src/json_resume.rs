use crate::schema::ResumeData;
use serde_json::{json, Value};

fn clean_inline(v: &str) -> String {
  v.split_whitespace().collect::<Vec<_>>().join(" ").trim().to_string()
}

fn safe(v: &str) -> String {
  let s = clean_inline(v);
  if s.is_empty() { "-".to_string() } else { s }
}

pub fn resume_data_to_json_resume(resume: &ResumeData, include_skills: bool) -> Value {
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

  let skills = if include_skills {
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
      "name": safe(&b.name),
      "label": "",
      "xGender": safe(&b.gender),
      "xAge": safe(&b.age),
      "image": "",
      "email": "",
      "phone": safe(&b.contact),
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
