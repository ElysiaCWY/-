use crate::errors::AppError;
use crate::schema::ResumeData;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSettings {
  pub llama_cli_path: String,
  pub model_path: String,
  pub threads: i32,
  pub temperature: f32,
}

pub fn parse_resume_with_llm(text: &str, settings: &LlmSettings) -> Result<ResumeData, AppError> {
  if settings.llama_cli_path.trim().is_empty() {
    return Err(AppError::msg("请在设置中填写 llama-cli 路径"));
  }
  if settings.model_path.trim().is_empty() {
    return Err(AppError::msg("请在设置中填写模型(GGUF)路径"));
  }

  let llama_cli_resolved = resolve_runtime_path(&settings.llama_cli_path)?;
  let model_resolved = resolve_runtime_path(&settings.model_path)?;

  if !llama_cli_resolved.exists() {
    return Err(AppError::msg(format!(
      "llama-cli 路径不存在：{}",
      llama_cli_resolved.display()
    )));
  }
  if !model_resolved.exists() {
    return Err(AppError::msg(format!(
      "模型路径不存在：{}",
      model_resolved.display()
    )));
  }

  let mut effective_settings = settings.clone();
  effective_settings.llama_cli_path = llama_cli_resolved.to_string_lossy().to_string();
  effective_settings.model_path = model_resolved.to_string_lossy().to_string();

  let tpl_path = std::env::current_dir()
    .unwrap_or_default()
    .join("解析结果模板.js");
  let tpl_content = std::fs::read_to_string(&tpl_path).unwrap_or_else(|_| {
    "{\n  \"basicInfo\": {},\n  \"workExperience\": {},\n  \"projectExperience\": {}\n}".to_string()
  });

  let prompt = format!(
    r#"解析简历，提取以下信息：
1、基础信息：姓名、年龄、性别、教育背景、技能、证书。
2、完整的工作经历（按时间倒序排列，不能有重复）。
3、完整的项目经历（包含项目名称、项目描述、项目成果）。

以 js 文件格式输出，模板是“解析结果模板.js”文件，该文件内容如下：
```javascript
{tpl}
```

【特别注意】：为了程序能够成功读取，你在填写模板内容时，必须输出标准、无包裹的 JSON 字符串。
不要带 var/const 声明，不要带 module.exports，不要输出 markdown 代码块或解释文本，
请直接输出以 '{{' 开始、以 '}}' 结束的 JSON。
所有 key 必须使用双引号。

简历内容：
"""{text}""""#,
    tpl = tpl_content,
    text = clip(text, 14000)
  );

  let json_res = run_llama_json(&prompt, &effective_settings, 4000)?;
  let merged: ResumeData = serde_json::from_str(&json_res).map_err(|e| {
    AppError::msg(format!(
      "反序列化大模型结果失败：{}\nJSON原文：{}",
      e, json_res
    ))
  })?;

  Ok(merged)
}

fn resolve_runtime_path(input: &str) -> Result<PathBuf, AppError> {
  let p = Path::new(input.trim());
  if p.is_absolute() {
    return Ok(p.to_path_buf());
  }

  if let Ok(exe) = std::env::current_exe() {
    if let Some(exe_dir) = exe.parent() {
      let candidate = exe_dir.join(p);
      if candidate.exists() {
        return Ok(candidate);
      }
    }
  }

  if let Ok(cwd) = std::env::current_dir() {
    return Ok(cwd.join(p));
  }

  Err(AppError::msg("无法解析相对路径，请改为绝对路径"))
}

fn run_llama_json(prompt: &str, settings: &LlmSettings, max_tokens: i32) -> Result<String, AppError> {
  let resolved_model_path = Path::new(&settings.model_path);
  let model_dir = resolved_model_path.parent().unwrap_or(Path::new(""));
  let model_file = resolved_model_path.file_name().unwrap_or_default();

  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();

  let prompt_file_name = format!("prompt_{}.txt", now);
  let prompt_file_path = model_dir.join(&prompt_file_name);

  if let Err(e) = std::fs::write(&prompt_file_path, prompt) {
    return Err(AppError::msg(format!("写入临时 prompt 文件失败：{}", e)));
  }

  let mut cmd = Command::new(&settings.llama_cli_path);
  cmd.creation_flags(0x08000000)
    .current_dir(model_dir)
    .arg("-m")
    .arg(model_file)
    .arg("-t")
    .arg(settings.threads.to_string())
    .arg("--temp")
    .arg(format!("{:.2}", settings.temperature))
    .arg("-n")
    .arg(max_tokens.to_string())
    .arg("-c")
    .arg("8000")
    .arg("-f")
    .arg(&prompt_file_name)
    .arg("--no-display-prompt")
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

  let out = cmd.output();
  let _ = std::fs::remove_file(&prompt_file_path);

  let out = out.map_err(|e| AppError::msg(format!("启动 llama-cli 失败：{}", e)))?;
  if !out.status.success() {
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    return Err(AppError::msg(format!("llama-cli 运行失败：{}", err)));
  }

  let stdout = String::from_utf8_lossy(&out.stdout).to_string();
  extract_json_object(&stdout)
    .ok_or_else(|| AppError::msg(format!("模型输出中未找到 JSON 对象\n{}", stdout)))
}

fn extract_json_object(s: &str) -> Option<String> {
  if let Some(last_md) = s.rfind("```json") {
    let sub = &s[last_md + 7..];
    if let Some(end_md) = sub.find("```") {
      let candidate = sub[..end_md].trim().to_string();
      let re_trailing = Regex::new(r",\s*([}\]])").ok()?;
      return Some(re_trailing.replace_all(&candidate, "$1").to_string());
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
