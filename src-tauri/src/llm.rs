use crate::errors::AppError;
use crate::schema::ResumeData;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSettings {
  pub llama_cli_path: String,
  pub model_path: String,
  pub threads: i32,
  pub temperature: f32,
}

pub fn parse_resume_with_llm(text: &str, settings: &LlmSettings) -> Result<ResumeData, AppError> {
  if settings.model_path.trim().is_empty() {
    return Err(AppError::msg("请在设置中填写 Ollama 模型名，例如 qwen2.5:3b"));
  }

  if looks_like_gguf_path(&settings.model_path) {
    return Err(AppError::msg(
      "当前已切换为 Ollama 调用，modelPath 需要填写模型名（如 qwen2.5:3b），不再支持 .gguf 文件路径",
    ));
  }

  let tpl_path = resolve_template_path("解析结果模板.json")?;
  let tpl_content = std::fs::read_to_string(&tpl_path).map_err(|e| {
    AppError::msg(format!(
      "读取模板文件失败：{}。请确认文件存在且可读：{}",
      e,
      tpl_path.display()
    ))
  })?;
  let tpl_for_prompt = template_for_prompt(&tpl_content);

  // 第一阶段：先抽取稳定结构（公司/职位/时间段/项目名称等骨架信息）
  let stage1_prompt = build_stage1_prompt(&tpl_for_prompt, text);
  let stage1_json = run_ollama_json(&stage1_prompt, settings)?;
  let stage1_data = parse_resume_data_flexible(&stage1_json).map_err(|e| {
    AppError::msg(format!(
      "第一阶段反序列化失败：{}\nJSON原文：{}",
      e,
      clip(&stage1_json, 1200)
    ))
  })?;

  // 第二阶段：基于第一阶段骨架补全描述细节，提升内容质量
  let stage1_seed = serde_json::to_string_pretty(&stage1_data)
    .unwrap_or_else(|_| stage1_json.clone());
  let stage2_prompt = build_stage2_prompt(&tpl_for_prompt, text, &stage1_seed);

  let final_data = match run_ollama_json(&stage2_prompt, settings)
    .and_then(|json| {
      parse_resume_data_flexible(&json).map_err(AppError::msg)
    }) {
    Ok(v) => v,
    // 第二阶段失败时回退第一阶段结果，避免解析流程整体失败。
    Err(_) => stage1_data,
  };

  Ok(final_data)
}

fn build_stage1_prompt(tpl: &str, text: &str) -> String {
  format!(
    r#"解析简历（第一阶段：结构抽取），只输出 JSON。

下面是字段结构模板（JSON）：
```json
{tpl}
```

要求：
1. 严格按模板字段输出，不要新增或删除字段。
2. 工作经历与项目经历要尽量完整枚举，按时间倒序。
3. 本阶段优先保证结构完整和分段正确，description/projectAchievements 可简写。
4. 仅返回 JSON 对象，不要返回 markdown 或解释文字。

简历内容：
"""{text}""""#,
    tpl = tpl,
    text = text
  )
}

fn build_stage2_prompt(tpl: &str, text: &str, stage1_seed: &str) -> String {
  format!(
    r#"解析简历（第二阶段：细节补全），只输出 JSON。

下面是字段结构模板（JSON）：
```json
{tpl}
```

下面是第一阶段结果（结构骨架），请在不破坏结构的前提下补全内容：
```json
{seed}
```

要求：
1. 保持 workExperience / projectExperience 的条目结构，不要把多段经历合并成一段。
2. 可补充 description / projectDescription / projectAchievements 的细节，但不要编造不存在的公司/项目。
3. 仅返回 JSON 对象，不要返回 markdown 或解释文字。

简历内容：
"""{text}""""#,
    tpl = tpl,
    seed = stage1_seed,
    text = text
  )
}

fn run_ollama_json(prompt: &str, settings: &LlmSettings) -> Result<String, AppError> {
  let base_url = normalize_ollama_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/api/generate", base_url);

  let body = json!({
    "model": settings.model_path.trim(),
    "prompt": prompt,
    "format": "json",
    "stream": false,
    "options": {
      "temperature": settings.temperature,
      "num_ctx": 8000,
      "num_thread": settings.threads,
    }
  });

  let resp = ureq::post(&endpoint)
    .set("Content-Type", "application/json")
    .send_json(body)
    .map_err(|e| {
      AppError::msg(format!(
        "调用 Ollama 失败：{}。请确认 Ollama 已启动（ollama serve）且地址可访问：{}",
        e, base_url
      ))
    })?;

  let payload: Value = resp
    .into_json()
    .map_err(|e| AppError::msg(format!("解析 Ollama 响应失败：{}", e)))?;

  let raw_text = payload
    .get("response")
    .and_then(|v| v.as_str())
    .ok_or_else(|| AppError::msg(format!("Ollama 响应缺少 response 字段：{}", payload)))?;

  extract_json_object(raw_text).ok_or_else(|| {
    AppError::msg(format!(
      "模型输出中未找到 JSON 对象。原始输出前800字符：{}",
      clip(raw_text, 800)
    ))
  })
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

fn looks_like_gguf_path(value: &str) -> bool {
  let v = value.trim().to_ascii_lowercase();
  v.ends_with(".gguf")
}

fn extract_json_object(s: &str) -> Option<String> {
  let s = s.trim().trim_matches('\u{feff}');

  if let Some(last_md) = s.rfind("```json") {
    let sub = &s[last_md + 7..];
    if let Some(end_md) = sub.find("```") {
      let candidate = sub[..end_md].trim().to_string();
      let re_trailing = Regex::new(r",\s*([}\]])").ok()?;
      return Some(re_trailing.replace_all(&candidate, "$1").to_string());
    }
  }

  if let Some(last_md) = s.rfind("```javascript") {
    let sub = &s[last_md + 13..];
    if let Some(end_md) = sub.find("```") {
      let js = sub[..end_md].trim();
      if let (Some(start), Some(end)) = (js.find('{'), js.rfind('}')) {
        if end > start {
          let candidate = js[start..=end].trim().to_string();
          let re_trailing = Regex::new(r",\s*([}\]])").ok()?;
          return Some(re_trailing.replace_all(&candidate, "$1").to_string());
        }
      }
    }
  }

  let mut search_start = 0;
  if let Some(idx) = s.rfind("[End thinking]") {
    search_start = idx + "[End thinking]".len();
  } else if let Some(idx) = s.rfind("</think>") {
    search_start = idx + "</think>".len();
  } else if let Some(idx) = s.rfind("【特别注意】") {
    search_start = idx + "【特别注意】".len();
  }

  let sub = &s[search_start..];
  let start = sub.find('{')?;
  let end = sub.rfind('}')?;
  if end <= start {
    return None;
  }

  let candidate = sub[start..=end].trim().trim_matches('`').trim().to_string();
  let re_trailing = Regex::new(r",\s*([}\]])").ok()?;
  Some(re_trailing.replace_all(&candidate, "$1").to_string())
}

fn clip(s: &str, max_chars: usize) -> String {
  s.chars().take(max_chars).collect()
}

fn template_for_prompt(raw: &str) -> String {
  let mut v: Value = match serde_json::from_str(raw) {
    Ok(v) => v,
    Err(_) => return raw.to_string(),
  };
  strip_comment_fields(&mut v);
  serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw.to_string())
}

fn strip_comment_fields(v: &mut Value) {
  match v {
    Value::Object(map) => {
      map.retain(|k, _| !k.starts_with("_comment"));
      for value in map.values_mut() {
        strip_comment_fields(value);
      }
    }
    Value::Array(arr) => {
      for item in arr {
        strip_comment_fields(item);
      }
    }
    _ => {}
  }
}

fn parse_resume_data_flexible(s: &str) -> Result<ResumeData, String> {
  if let Ok(parsed) = serde_json::from_str::<ResumeData>(s) {
    return Ok(parsed);
  }

  let mut v: Value = serde_json::from_str(s).map_err(|e| format!("JSON语法错误：{}", e))?;
  repair_resume_value(&mut v);

  serde_json::from_value::<ResumeData>(v.clone())
    .map_err(|e| format!("结构修复后仍不匹配：{}；修复后JSON前1200字符：{}", e, clip(&v.to_string(), 1200)))
}

fn repair_resume_value(v: &mut Value) {
  let mut root = match v.as_object() {
    Some(obj) => obj.clone(),
    None => return,
  };

  if let Some(wrapped) = root.get("resumeData").and_then(|x| x.as_object()).cloned() {
    root = wrapped;
  }

  alias_field(&mut root, "basic_info", "basicInfo");
  alias_field(&mut root, "work_experience", "workExperience");
  alias_field(&mut root, "project_experience", "projectExperience");

  if let Some(mut basic_obj) = root.get("basicInfo").and_then(|x| x.as_object()).cloned() {
    if !root.contains_key("workExperience") {
      if let Some(misplaced) = basic_obj.remove("workExperience").or_else(|| basic_obj.remove("work_experience")) {
        root.insert("workExperience".to_string(), misplaced);
      }
    }
    if !root.contains_key("projectExperience") {
      if let Some(misplaced) = basic_obj.remove("projectExperience").or_else(|| basic_obj.remove("project_experience")) {
        root.insert("projectExperience".to_string(), misplaced);
      }
    }
    root.insert("basicInfo".to_string(), Value::Object(basic_obj));
  }

  ensure_object_field(&mut root, "basicInfo");
  ensure_object_field(&mut root, "workExperience");
  ensure_object_field(&mut root, "projectExperience");

  if let Some(w) = root.get_mut("workExperience") {
    *w = to_indexed_object(w);
    sanitize_indexed_items(w, &["company", "position", "period", "description"]);
  }
  if let Some(p) = root.get_mut("projectExperience") {
    *p = to_indexed_object(p);
    sanitize_indexed_items(
      p,
      &["projectName", "projectDescription", "projectAchievements"],
    );
  }

  sanitize_basic_info(&mut root);

  *v = Value::Object(root);
}

fn alias_field(root: &mut Map<String, Value>, from: &str, to: &str) {
  if root.contains_key(to) {
    return;
  }
  if let Some(v) = root.remove(from) {
    root.insert(to.to_string(), v);
  }
}

fn ensure_object_field(root: &mut Map<String, Value>, key: &str) {
  match root.get(key) {
    Some(Value::Object(_)) => {}
    _ => {
      root.insert(key.to_string(), Value::Object(Map::new()));
    }
  }
}

fn to_indexed_object(input: &Value) -> Value {
  match input {
    Value::Array(arr) => {
      let mut out = Map::new();
      for (i, item) in arr.iter().enumerate() {
        out.insert((i + 1).to_string(), item.clone());
      }
      Value::Object(out)
    }
    Value::Object(map) => {
      let is_indexed = map.keys().all(|k| k.parse::<usize>().is_ok());
      if is_indexed {
        Value::Object(map.clone())
      } else if map.is_empty() {
        Value::Object(Map::new())
      } else {
        let mut out = Map::new();
        out.insert("1".to_string(), Value::Object(map.clone()));
        Value::Object(out)
      }
    }
    _ => Value::Object(Map::new()),
  }
}

fn sanitize_basic_info(root: &mut Map<String, Value>) {
  let mut basic = root
    .get("basicInfo")
    .and_then(|v| v.as_object())
    .cloned()
    .unwrap_or_default();

  ensure_string_key(&mut basic, "name");
  ensure_string_key(&mut basic, "age");
  ensure_string_key(&mut basic, "gender");

  match basic.get("skills") {
    Some(Value::Array(_)) => {}
    _ => {
      basic.insert("skills".to_string(), Value::Array(vec![]));
    }
  }
  match basic.get("certificates") {
    Some(Value::Array(_)) => {}
    _ => {
      basic.insert("certificates".to_string(), Value::Array(vec![]));
    }
  }

  let edu = basic.get("education").cloned().unwrap_or(Value::Array(vec![]));
  let mut edu_arr = match edu {
    Value::Array(a) => a,
    Value::Object(o) => vec![Value::Object(o)],
    _ => vec![],
  };
  if edu_arr.is_empty() {
    edu_arr.push(Value::Object(Map::new()));
  }
  for item in &mut edu_arr {
    let mut obj = item.as_object().cloned().unwrap_or_default();
    ensure_string_key(&mut obj, "school");
    ensure_string_key(&mut obj, "major");
    ensure_string_key(&mut obj, "degree");
    ensure_string_key(&mut obj, "period");
    *item = Value::Object(obj);
  }
  basic.insert("education".to_string(), Value::Array(edu_arr));

  root.insert("basicInfo".to_string(), Value::Object(basic));
}

fn sanitize_indexed_items(value: &mut Value, required_keys: &[&str]) {
  let Some(map) = value.as_object_mut() else {
    return;
  };

  for item in map.values_mut() {
    let mut obj = match item {
      Value::Object(o) => o.clone(),
      Value::String(s) => {
        let mut m = Map::new();
        m.insert("description".to_string(), Value::String(s.clone()));
        m
      }
      _ => Map::new(),
    };

    for key in required_keys {
      if !obj.contains_key(*key) {
        obj.insert((*key).to_string(), Value::String(String::new()));
      }
    }

    *item = Value::Object(obj);
  }
}

fn ensure_string_key(obj: &mut Map<String, Value>, key: &str) {
  match obj.get(key) {
    Some(Value::String(_)) => {}
    Some(v) => {
      obj.insert(key.to_string(), Value::String(v.to_string()));
    }
    None => {
      obj.insert(key.to_string(), Value::String(String::new()));
    }
  }
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

fn resolve_template_path(file_name: &str) -> Result<PathBuf, AppError> {
  if let Ok(exe) = std::env::current_exe() {
    if let Some(exe_dir) = exe.parent() {
      if let Some(found) = find_in_ancestors(exe_dir, file_name, 6) {
        return Ok(found);
      }
    }
  }

  if let Ok(cwd) = std::env::current_dir() {
    if let Some(found) = find_in_ancestors(&cwd, file_name, 6) {
      return Ok(found);
    }
    return Ok(cwd.join(file_name));
  }

  Err(AppError::msg(format!("无法定位模板文件：{}", file_name)))
}
