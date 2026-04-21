use crate::errors::AppError;
use crate::schema::{AppSettings, ProjectItem, ResumeData, WorkItem};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

/// 与 `run_ollama_json` 中 Ollama `num_ctx` 保持一致，便于日志对照
const RESUME_PARSE_NUM_CTX: u32 = 65536;
/// 单次 Ollama 请求最大等待时长（秒），避免任务无限挂起。
const OLLAMA_REQUEST_TIMEOUT_SECS: u64 = 180;
/// 单次生成最大 token 数，避免使用默认较小上限导致 JSON 被截断。
const OLLAMA_NUM_PREDICT: i32 = 8192;
/// 单阶段（stage1/stage2）最大尝试次数：首次 + 重试。
const OLLAMA_MAX_ATTEMPTS: usize = 2;
/// 重试前等待毫秒。
const OLLAMA_RETRY_DELAY_MS: u64 = 1200;

/// 构建时嵌入仓库根目录的 `解析结果模板.json`；运行时同目录若存在同名文件则优先读取。
const EMBEDDED_RESUME_TEMPLATE_JSON: &str =
  include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../解析结果模板.json"));

fn default_llm_provider() -> String {
  "ollama".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmSettings {
  pub llama_cli_path: String,
  pub model_path: String,
  pub threads: i32,
  pub temperature: f32,
  #[serde(default = "default_llm_provider", alias = "llmProvider")]
  pub llm_provider: String,
}

impl From<&AppSettings> for LlmSettings {
  fn from(s: &AppSettings) -> Self {
    Self {
      llama_cli_path: s.llama_cli_path.clone(),
      model_path: s.model_path.clone(),
      threads: s.threads,
      temperature: s.temperature,
      llm_provider: s.llm_provider.clone(),
    }
  }
}

/// 调用本地模型生成一段 JSON 文本（与简历解析阶段相同的重试与校验逻辑）。
#[derive(Debug, Clone)]
pub struct JsonPromptParams {
  pub temperature: f32,
  pub ollama_num_ctx: u32,
  pub ollama_num_predict: Option<i32>,
}

fn provider_is_lmstudio(raw: &str) -> bool {
  matches!(
    raw.trim().to_ascii_lowercase().as_str(),
    "lmstudio" | "lm-studio" | "lm_studio"
  )
}

pub fn parse_resume_with_llm(text: &str, settings: &LlmSettings) -> Result<ResumeData, AppError> {
  if settings.model_path.trim().is_empty() {
    resume_parse_log!(error, "resume_parse: 中止，未配置模型名");
    return Err(AppError::msg(
      "请在 app-config.json 的 modelPath 中填写模型名（Ollama 如 qwen2.5:3b；LM Studio 与左侧已选模型 ID 一致）",
    ));
  }

  if looks_like_gguf_path(&settings.model_path) {
    resume_parse_log!(error, "resume_parse: 中止，modelPath 不能为 .gguf 路径");
    return Err(AppError::msg(
      "modelPath 需要填写模型名（如 qwen2.5:3b），不再支持 .gguf 文件路径",
    ));
  }

  let base_log = if provider_is_lmstudio(&settings.llm_provider) {
    normalize_lmstudio_base_url(&settings.llama_cli_path)
  } else {
    normalize_ollama_base_url(&settings.llama_cli_path)
  };
  let text_chars = text.chars().count();
  resume_parse_log!(
    info,
    "resume_parse: 开始 provider={} model={} base={} text_chars={} num_ctx={} threads={} temp={}",
    settings.llm_provider.trim(),
    settings.model_path.trim(),
    base_log,
    text_chars,
    RESUME_PARSE_NUM_CTX,
    settings.threads,
    settings.temperature
  );

  let tpl_content = load_resume_template_content()?;
  let tpl_for_prompt = template_for_prompt(&tpl_content);

  // 第一阶段：先抽取稳定结构（公司/职位/时间段/项目名称等骨架信息）
  let stage1_prompt = build_stage1_prompt(&tpl_for_prompt, text);
  resume_parse_log!(
    debug,
    "resume_parse: stage1_prompt_chars={}",
    stage1_prompt.chars().count()
  );
  let stage1_json = run_ollama_json("stage1", &stage1_prompt, settings)?;
  resume_parse_log!(
    debug,
    "resume_parse: stage1 模型原始 JSON 长度={}",
    stage1_json.chars().count()
  );
  let stage1_data = parse_resume_data_flexible(&stage1_json).map_err(|e| {
    resume_parse_log!(
      error,
      "resume_parse: stage1 反序列化失败 err={} json_prefix={}",
      e,
      clip(&stage1_json, 800)
    );
    AppError::msg(format!(
      "第一阶段反序列化失败：{}\nJSON原文：{}",
      e,
      clip(&stage1_json, 1200)
    ))
  })?;
  resume_parse_log!(
    info,
    "resume_parse: stage1 成功 work_entries={} project_entries={}",
    stage1_data.work_experience.len(),
    stage1_data.project_experience.len()
  );

  // 第二阶段：基于第一阶段骨架补全描述细节，提升内容质量
  let stage1_seed = serde_json::to_string_pretty(&stage1_data)
    .unwrap_or_else(|_| stage1_json.clone());
  let stage2_prompt = build_stage2_prompt(&tpl_for_prompt, text, &stage1_seed);
  resume_parse_log!(
    debug,
    "resume_parse: stage2_prompt_chars={} stage1_seed_chars={}",
    stage2_prompt.chars().count(),
    stage1_seed.chars().count()
  );

  let final_data = match run_ollama_json("stage2", &stage2_prompt, settings).and_then(|json| {
    let n = json.chars().count();
    parse_resume_data_flexible(&json).map_err(|e| {
      resume_parse_log!(
        warn,
        "resume_parse: stage2 JSON 反序列化失败 len={} err={} json_prefix={}",
        n,
        e,
        clip(&json, 600)
      );
      AppError::msg(e)
    })
  }) {
    Ok(v) => {
      resume_parse_log!(
        info,
        "resume_parse: stage2 成功 work_entries={} project_entries={}",
        v.work_experience.len(),
        v.project_experience.len()
      );
      v
    }
    Err(e) => {
      resume_parse_log!(
        warn,
        "resume_parse: stage2 未采用，回退 stage1 结果。原因: {}",
        e
      );
      stage1_data.clone()
    }
  };

  // 防止第二阶段“补全”时把第一阶段已抽到的项目/工作经历条目变少。
  let final_data = preserve_project_items(&stage1_data, final_data);
  let final_data = preserve_work_items(&stage1_data, final_data);
  let final_data = preserve_basic_and_details(&stage1_data, final_data);

  let grounded = filter_resume_by_source_text(final_data, text);
  resume_parse_log!(
    info,
    "resume_parse: 完成（过滤后）name={:?} work_entries={} project_entries={}",
    grounded.basic_info.name,
    grounded.work_experience.len(),
    grounded.project_experience.len()
  );
  Ok(grounded)
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
4. 项目经历必须尽可能完整抽取（来自“项目经历/项目经验/项目描述”等章节），不要遗漏条目。每条项目中：projectDescription 侧重背景、职责、技术方案与实现要点；**projectAchievements 必须承载简历里该项目的「项目业绩」「项目成果」「业绩」「效果/指标」等成果类表述**（含量化数据、获奖、业务效果）；若原文把业绩写在项目段落内或单独小标题下，须完整迁入 projectAchievements，勿仅堆在 projectDescription 而留空成果。
5. basicInfo.skills：除简历中明确列出的技能（如「专业技能」「技术栈」「掌握」等小节）外，必须结合 **工作经历** 的 description 与 **项目经历** 的 projectDescription、projectAchievements 中实际出现或可合理归纳的技术要素进行补充，包括：编程语言、框架、库、数据库与中间件、云平台、工具链、工程实践相关术语等；表述用简短名词短语，去重，与原文表述一致或可核对，不得凭空捏造。
6. 严禁混入其他候选人的信息；若原文无证据，不得编造。
7. 仅返回 JSON 对象，不要返回 markdown 或解释文字。
8. 必须输出完整、可解析的 JSON，不得中途截断或提前结束。

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
2. 可补充 description / projectDescription / projectAchievements 的细节，但不要编造不存在的公司/项目。补全 projectAchievements 时，**优先从原文「项目业绩」「项目成果」「业绩」「效果」等小节或项目块内的成果描述完整迁入**，与 projectDescription 分工：描述讲“做了什么”，成果讲“效果与业绩”。
3. projectExperience 的条目数量不得少于第一阶段，不能因为补全而删减已有项目。
4. 同步检查并完善 basicInfo.skills：在第一阶段已有技能基础上，根据本阶段补全后的 **工作经历、项目经历** 全文，补充其中出现或可归纳的技术技能（须能在原简历中核对，表述简短、去重）；若经历中无新增技术信息则保持原 skills。
5. 严禁混入其他候选人的信息；只有简历原文可找到依据的经历才能保留。
6. 仅返回 JSON 对象，不要返回 markdown 或解释文字。
7. 必须输出完整、可解析的 JSON，不得中途截断或提前结束。

简历内容：
"""{text}""""#,
    tpl = tpl,
    seed = stage1_seed,
    text = text
  )
}

fn llm_log_prefix(label: &str) -> &'static str {
  if label == "stage1" || label == "stage2" {
    "resume_parse"
  } else {
    "app_llm"
  }
}

/// 统一入口：Ollama `/api/generate` 或 LM Studio OpenAI 兼容 `POST .../chat/completions`。
pub fn complete_json_prompt(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  if provider_is_lmstudio(&settings.llm_provider) {
    run_lmstudio_json(label, prompt, settings, params)
  } else {
    run_ollama_json_params(label, prompt, settings, params)
  }
}

fn run_ollama_json(label: &str, prompt: &str, settings: &LlmSettings) -> Result<String, AppError> {
  complete_json_prompt(
    label,
    prompt,
    settings,
    JsonPromptParams {
      temperature: settings.temperature,
      ollama_num_ctx: RESUME_PARSE_NUM_CTX,
      ollama_num_predict: Some(OLLAMA_NUM_PREDICT),
    },
  )
}

fn run_ollama_json_params(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  let base_url = normalize_ollama_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/api/generate", base_url);

  let mut opts = serde_json::Map::new();
  opts.insert("temperature".into(), json!(params.temperature));
  opts.insert("num_ctx".into(), json!(params.ollama_num_ctx));
  opts.insert("num_thread".into(), json!(settings.threads));
  if let Some(np) = params.ollama_num_predict {
    opts.insert("num_predict".into(), json!(np));
  }

  let body = json!({
    "model": settings.model_path.trim(),
    "prompt": prompt,
    "format": "json",
    "stream": false,
    "options": Value::Object(opts),
  });

  let np_log = params.ollama_num_predict.unwrap_or(-1);
  resume_parse_log!(
    debug,
    "{}: [{}] POST {} prompt_chars={} timeout_s={} max_attempts={} num_predict={}",
    llm_log_prefix(label),
    label,
    endpoint,
    prompt.chars().count(),
    OLLAMA_REQUEST_TIMEOUT_SECS,
    OLLAMA_MAX_ATTEMPTS,
    np_log
  );

  let mut last_err: Option<String> = None;
  for attempt in 1..=OLLAMA_MAX_ATTEMPTS {
    let result = ureq::post(&endpoint)
      .set("Content-Type", "application/json")
      .timeout(Duration::from_secs(OLLAMA_REQUEST_TIMEOUT_SECS))
      .send_json(body.clone());

    let resp = match result {
      Ok(resp) => resp,
      Err(e) => {
        let err_text = e.to_string();
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}；{}ms 后重试",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            base_url,
            err_text,
            OLLAMA_RETRY_DELAY_MS
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}",
          llm_log_prefix(label),
          label,
          attempt,
          OLLAMA_MAX_ATTEMPTS,
          base_url,
          err_text
        );
        return Err(AppError::msg(format!(
          "调用 Ollama 失败（阶段：{}，已重试 {} 次）：{}。请确认 Ollama 可访问：{}；若偶发卡住，建议重试本批次。",
          label,
          OLLAMA_MAX_ATTEMPTS,
          err_text,
          base_url
        )));
      }
    };

    let payload: Value = resp.into_json().map_err(|e| {
      resume_parse_log!(
        error,
        "{}: [{}] 解析 Ollama JSON 响应失败: {}",
        llm_log_prefix(label),
        label,
        e
      );
      AppError::msg(format!("解析 Ollama 响应失败：{}", e))
    })?;

    let raw_text = payload
      .get("response")
      .and_then(|v| v.as_str())
      .ok_or_else(|| {
        resume_parse_log!(
          error,
          "{}: [{}] 响应缺少 response 字段 payload={}",
          llm_log_prefix(label),
          label,
          payload
        );
        AppError::msg(format!("Ollama 响应缺少 response 字段：{}", payload))
      })?;

    let extracted = match extract_json_object(raw_text) {
      Some(v) => v,
      None => {
        let err_text = format!("模型输出中未找到 JSON 对象。原始输出前800字符：{}", clip(raw_text, 800));
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] 无法提取 JSON（第{}/{}次）；{}ms 后重试 raw_prefix={}",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            OLLAMA_RETRY_DELAY_MS,
            clip(raw_text, 400)
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] 无法从模型输出中提取 JSON，raw_prefix={}",
          llm_log_prefix(label),
          label,
          clip(raw_text, 800)
        );
        return Err(AppError::msg(err_text));
      }
    };

    match serde_json::from_str::<Value>(&extracted) {
      Ok(_) => return Ok(extracted),
      Err(e) => {
        let err_text = format!("模型输出 JSON 语法错误：{}", e);
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] JSON 语法错误（第{}/{}次）err={}；{}ms 后重试 json_prefix={}",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            e,
            OLLAMA_RETRY_DELAY_MS,
            clip(&extracted, 400)
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] JSON 语法错误（第{}/{}次）err={} json_prefix={}",
          llm_log_prefix(label),
          label,
          attempt,
          OLLAMA_MAX_ATTEMPTS,
          e,
          clip(&extracted, 800)
        );
        return Err(AppError::msg(format!("{}。原始输出前800字符：{}", err_text, clip(&extracted, 800))));
      }
    }
  }

  Err(AppError::msg(format!(
    "调用 Ollama 失败（阶段：{}）：{}",
    label,
    last_err.unwrap_or_else(|| "未知错误".to_string())
  )))
}

fn lmstudio_max_tokens(params: &JsonPromptParams) -> u32 {
  let n = params.ollama_num_predict.unwrap_or(4096);
  n.clamp(256, 32768) as u32
}

fn run_lmstudio_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  let base_url = normalize_lmstudio_base_url(&settings.llama_cli_path);
  let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
  let max_tokens = lmstudio_max_tokens(&params);
  let schema_name = if label == "stage1" || label == "stage2" {
    "resume_parse_schema"
  } else {
    "generic_json_schema"
  };
  let response_format = if label == "stage1" || label == "stage2" {
    json!({
      "type": "json_schema",
      "json_schema": {
        "name": schema_name,
        "strict": true,
        "schema": resume_json_schema()
      }
    })
  } else {
    json!({ "type": "json_object" })
  };

  let body = json!({
    "model": settings.model_path.trim(),
    "messages": [
      { "role": "user", "content": prompt }
    ],
    "temperature": params.temperature,
    "max_tokens": max_tokens,
    "stream": false,
    // LM Studio(OpenAI 兼容)结构化输出：阶段1/2使用 JSON Schema 强约束。
    "response_format": response_format
  });

  resume_parse_log!(
    debug,
    "{}: [{}] POST {} prompt_chars={} timeout_s={} max_attempts={} max_tokens={}",
    llm_log_prefix(label),
    label,
    endpoint,
    prompt.chars().count(),
    OLLAMA_REQUEST_TIMEOUT_SECS,
    OLLAMA_MAX_ATTEMPTS,
    max_tokens
  );

  let mut last_err: Option<String> = None;
  for attempt in 1..=OLLAMA_MAX_ATTEMPTS {
    let result = ureq::post(&endpoint)
      .set("Content-Type", "application/json")
      .timeout(Duration::from_secs(OLLAMA_REQUEST_TIMEOUT_SECS))
      .send_json(body.clone());

    let resp = match result {
      Ok(resp) => resp,
      Err(e) => {
        let err_text = e.to_string();
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}；{}ms 后重试",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            base_url,
            err_text,
            OLLAMA_RETRY_DELAY_MS
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}",
          llm_log_prefix(label),
          label,
          attempt,
          OLLAMA_MAX_ATTEMPTS,
          base_url,
          err_text
        );
        return Err(AppError::msg(format!(
          "调用 LM Studio 失败（阶段：{}，已重试 {} 次）：{}。请确认已启动本地服务器且地址为：{}（OpenAI 兼容 /v1）；若偶发卡住，建议重试本批次。",
          label,
          OLLAMA_MAX_ATTEMPTS,
          err_text,
          base_url
        )));
      }
    };

    let payload: Value = resp.into_json().map_err(|e| {
      resume_parse_log!(
        error,
        "{}: [{}] 解析 LM Studio JSON 响应失败: {}",
        llm_log_prefix(label),
        label,
        e
      );
      AppError::msg(format!("解析 LM Studio 响应失败：{}", e))
    })?;

    if let Some(err) = payload.get("error") {
      let msg = err.to_string();
      resume_parse_log!(
        error,
        "{}: [{}] LM Studio 返回 error: {}",
        llm_log_prefix(label),
        label,
        msg
      );
      return Err(AppError::msg(format!("LM Studio 返回错误：{}", msg)));
    }

    let first_choice = payload
      .get("choices")
      .and_then(|c| c.as_array())
      .and_then(|a| a.first())
      .ok_or_else(|| {
        resume_parse_log!(
          error,
          "{}: [{}] 响应缺少 choices[0] payload={}",
          llm_log_prefix(label),
          label,
          payload
        );
        AppError::msg(format!("LM Studio 响应缺少 choices[0]：{}", payload))
      })?;

    let finish_reason = first_choice
      .get("finish_reason")
      .and_then(|v| v.as_str())
      .unwrap_or("");
    let message = first_choice.get("message").and_then(|m| m.as_object()).ok_or_else(|| {
      resume_parse_log!(
        error,
        "{}: [{}] 响应缺少 choices[0].message payload={}",
        llm_log_prefix(label),
        label,
        payload
      );
      AppError::msg(format!("LM Studio 响应缺少 choices[0].message：{}", payload))
    })?;
    let content_text = message
      .get("content")
      .and_then(|v| v.as_str())
      .unwrap_or("");
    let reasoning_text = message
      .get("reasoning_content")
      .and_then(|v| v.as_str())
      .unwrap_or("");
    let (raw_text, source_field) = if !content_text.trim().is_empty() {
      (content_text, "content")
    } else if !reasoning_text.trim().is_empty() {
      resume_parse_log!(
        warn,
        "{}: [{}] content 为空，回退使用 reasoning_content finish_reason={}",
        llm_log_prefix(label),
        label,
        finish_reason
      );
      (reasoning_text, "reasoning_content")
    } else {
      resume_parse_log!(
        error,
        "{}: [{}] message.content 与 reasoning_content 均为空 finish_reason={} payload={}",
        llm_log_prefix(label),
        label,
        finish_reason,
        payload
      );
      return Err(AppError::msg(format!(
        "LM Studio 响应内容为空（content/reasoning_content 均为空，finish_reason={}）",
        finish_reason
      )));
    };

    let extracted = match extract_json_object(raw_text) {
      Some(v) => v,
      None => {
        let err_text = format!("模型输出中未找到 JSON 对象。原始输出前800字符：{}", clip(raw_text, 800));
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] 无法提取 JSON（第{}/{}次）；{}ms 后重试 source={} finish_reason={} raw_prefix={}",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            OLLAMA_RETRY_DELAY_MS,
            source_field,
            finish_reason,
            clip(raw_text, 400)
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] 无法从模型输出中提取 JSON，source={} finish_reason={} raw_prefix={}",
          llm_log_prefix(label),
          label,
          source_field,
          finish_reason,
          clip(raw_text, 800)
        );
        return Err(AppError::msg(err_text));
      }
    };

    match serde_json::from_str::<Value>(&extracted) {
      Ok(_) => return Ok(extracted),
      Err(e) => {
        let err_text = format!("模型输出 JSON 语法错误：{}", e);
        last_err = Some(err_text.clone());
        if attempt < OLLAMA_MAX_ATTEMPTS {
          resume_parse_log!(
            warn,
            "{}: [{}] JSON 语法错误（第{}/{}次）err={}；{}ms 后重试 json_prefix={}",
            llm_log_prefix(label),
            label,
            attempt,
            OLLAMA_MAX_ATTEMPTS,
            e,
            OLLAMA_RETRY_DELAY_MS,
            clip(&extracted, 400)
          );
          thread::sleep(Duration::from_millis(OLLAMA_RETRY_DELAY_MS));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] JSON 语法错误（第{}/{}次）err={} json_prefix={}",
          llm_log_prefix(label),
          label,
          attempt,
          OLLAMA_MAX_ATTEMPTS,
          e,
          clip(&extracted, 800)
        );
        return Err(AppError::msg(format!("{}。原始输出前800字符：{}", err_text, clip(&extracted, 800))));
      }
    }
  }

  Err(AppError::msg(format!(
    "调用 LM Studio 失败（阶段：{}）：{}",
    label,
    last_err.unwrap_or_else(|| "未知错误".to_string())
  )))
}

fn resume_json_schema() -> Value {
  json!({
    "type": "object",
    "additionalProperties": false,
    "required": ["basicInfo", "workExperience", "projectExperience"],
    "properties": {
      "basicInfo": {
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "age", "contact", "gender", "education", "skills", "certificates"],
        "properties": {
          "name": { "type": "string" },
          "age": { "type": "string" },
          "contact": { "type": "string" },
          "gender": { "type": "string" },
          "education": {
            "type": "array",
            "items": {
              "type": "object",
              "additionalProperties": false,
              "required": ["school", "major", "degree", "period"],
              "properties": {
                "school": { "type": "string" },
                "major": { "type": "string" },
                "degree": { "type": "string" },
                "period": { "type": "string" }
              }
            }
          },
          "skills": {
            "type": "array",
            "items": { "type": "string" }
          },
          "certificates": {
            "type": "array",
            "items": { "type": "string" }
          }
        }
      },
      "workExperience": {
        "type": "object",
        "additionalProperties": {
          "type": "object",
          "additionalProperties": false,
          "required": ["company", "position", "period", "description"],
          "properties": {
            "company": { "type": "string" },
            "position": { "type": "string" },
            "period": { "type": "string" },
            "description": { "type": "string" }
          }
        }
      },
      "projectExperience": {
        "type": "object",
        "additionalProperties": {
          "type": "object",
          "additionalProperties": false,
          "required": ["projectName", "projectDescription", "projectAchievements"],
          "properties": {
            "projectName": { "type": "string" },
            "projectDescription": { "type": "string" },
            "projectAchievements": { "type": "string" }
          }
        }
      }
    }
  })
}

pub fn normalize_lmstudio_base_url(input: &str) -> String {
  let raw = input.trim();
  let base = if raw.is_empty() || raw.to_ascii_lowercase().ends_with(".exe") {
    "http://127.0.0.1:1234".to_string()
  } else if raw.starts_with("http://") || raw.starts_with("https://") {
    raw.trim_end_matches('/').to_string()
  } else {
    format!("http://{}", raw.trim_end_matches('/'))
  };

  let base = base.trim_end_matches('/');
  if base.to_ascii_lowercase().ends_with("/v1") {
    base.to_string()
  } else {
    format!("{}/v1", base)
  }
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
  unwrap_wrapped_resume_payload(&mut v);
  repair_resume_value(&mut v);

  serde_json::from_value::<ResumeData>(v.clone())
    .map_err(|e| format!("结构修复后仍不匹配：{}；修复后JSON前1200字符：{}", e, clip(&v.to_string(), 1200)))
}

/// 兼容某些推理模型返回包装结构：
/// 1) {"response":"{...真正JSON...}"}（内层是 JSON 字符串）
/// 2) {"name":"解析简历（第一阶段...）","response":"..."}（顶层字段仅为包装元信息）
fn unwrap_wrapped_resume_payload(v: &mut Value) {
  let Some(obj) = v.as_object() else {
    return;
  };
  let response_text = obj.get("response").and_then(|x| x.as_str()).map(|x| x.trim().to_string());
  let Some(resp) = response_text else {
    return;
  };
  if resp.is_empty() {
    return;
  }

  let candidate = extract_json_object(&resp).unwrap_or(resp);
  if let Ok(inner) = serde_json::from_str::<Value>(&candidate) {
    if inner.is_object() {
      *v = inner;
    }
  }
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
    alias_field(&mut basic_obj, "phone", "contact");
    alias_field(&mut basic_obj, "mobile", "contact");
    alias_field(&mut basic_obj, "tel", "contact");
    alias_field(&mut basic_obj, "联系方式", "contact");
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

  // 兼容部分模型把基础信息直接放在顶层（name/age/contact/...），而非 basicInfo。
  merge_top_level_basic_fields(&mut root);

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

fn merge_top_level_basic_fields(root: &mut Map<String, Value>) {
  let mut basic = root
    .get("basicInfo")
    .and_then(|v| v.as_object())
    .cloned()
    .unwrap_or_default();

  let scalar_keys = ["name", "age", "contact", "gender"];
  for key in scalar_keys {
    let should_fill = basic
      .get(key)
      .and_then(|v| v.as_str())
      .map(|s| s.trim().is_empty())
      .unwrap_or(true);
    if !should_fill {
      continue;
    }
    if let Some(v) = root.get(key).cloned() {
      basic.insert(key.to_string(), v);
    }
  }

  let collection_keys = ["education", "skills", "certificates"];
  for key in collection_keys {
    let should_fill = match basic.get(key) {
      Some(Value::Array(arr)) => arr.is_empty(),
      Some(Value::Null) | None => true,
      _ => false,
    };
    if !should_fill {
      continue;
    }
    if let Some(v) = root.get(key).cloned() {
      basic.insert(key.to_string(), v);
    }
  }

  root.insert("basicInfo".to_string(), Value::Object(basic));
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
  ensure_string_key(&mut basic, "contact");
  ensure_string_key(&mut basic, "gender");

  match basic.get("skills") {
    Some(Value::Array(_)) => {}
    _ => {
      basic.insert("skills".to_string(), Value::Array(vec![]));
    }
  }

  // 证书：模板要求 Vec<String>，模型常输出 [{ "name": "...", "period": "..." }, ...]
  let cert_raw = basic.remove("certificates");
  let cert_arr = match cert_raw {
    Some(Value::Array(arr)) => arr
      .into_iter()
      .filter_map(|item| match item {
        Value::String(s) => {
          let t = s.trim();
          if t.is_empty() {
            None
          } else {
            Some(Value::String(s))
          }
        }
        Value::Object(o) => {
          let name = o
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
          let period = o
            .get("period")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
          let s = match (name.is_empty(), period.is_empty()) {
            (true, true) => return None,
            (false, true) => name.to_string(),
            (true, false) => period.to_string(),
            (false, false) => format!("{}（{}）", name, period),
          };
          Some(Value::String(s))
        }
        _ => None,
      })
      .collect::<Vec<_>>(),
    _ => vec![],
  };
  basic.insert("certificates".to_string(), Value::Array(cert_arr));

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

fn preserve_project_items(stage1: &ResumeData, mut final_data: ResumeData) -> ResumeData {
  let s1 = count_non_empty_projects(&stage1.project_experience);
  let s2 = count_non_empty_projects(&final_data.project_experience);
  if s1 > 0 && s2 < s1 {
    final_data.project_experience = stage1.project_experience.clone();
  }
  final_data
}

/// 第二阶段模型常把多段工作经历合并成更少条目；若条数变少则回退第一阶段骨架（与 `preserve_project_items` 对称）。
fn preserve_work_items(stage1: &ResumeData, mut final_data: ResumeData) -> ResumeData {
  let s1 = count_non_empty_work(&stage1.work_experience);
  let s2 = count_non_empty_work(&final_data.work_experience);
  if s1 > s2 {
    resume_parse_log!(
      warn,
      "resume_parse: stage2 工作经历条数少于 stage1（{} < {}），回退 stage1 工作经历",
      s2,
      s1
    );
    final_data.work_experience = stage1.work_experience.clone();
  }
  // 条目数一致时，仍可能出现阶段2把时间段截断为“2018 年 09 月 -”这类不完整形式；
  // 对同索引条目做逐项兜底，优先保留更完整的时间段。
  backfill_work_periods_from_stage1(stage1, &mut final_data);
  final_data
}

fn looks_like_prompt_title_name(name: &str) -> bool {
  let n = name.trim();
  n.starts_with("解析简历（") || n.starts_with("解析简历(") || n.contains("第一阶段") || n.contains("第二阶段")
}

fn preserve_basic_and_details(stage1: &ResumeData, mut final_data: ResumeData) -> ResumeData {
  let s1_name = stage1.basic_info.name.trim();
  let s2_name = final_data.basic_info.name.trim();
  if !s1_name.is_empty() && (s2_name.is_empty() || looks_like_prompt_title_name(s2_name)) {
    resume_parse_log!(
      warn,
      "resume_parse: stage2 姓名疑似异常（'{}'），回退 stage1 姓名='{}'",
      s2_name,
      s1_name
    );
    final_data.basic_info.name = stage1.basic_info.name.clone();
  }

  for (idx, item) in final_data.work_experience.iter_mut() {
    if let Some(s1) = stage1.work_experience.get(idx) {
      if item.description.trim().is_empty() && !s1.description.trim().is_empty() {
        item.description = s1.description.clone();
      }
      if item.company.trim().is_empty() && !s1.company.trim().is_empty() {
        item.company = s1.company.clone();
      }
      if item.position.trim().is_empty() && !s1.position.trim().is_empty() {
        item.position = s1.position.clone();
      }
    }
  }

  for (idx, item) in final_data.project_experience.iter_mut() {
    if let Some(s1) = stage1.project_experience.get(idx) {
      if item.project_description.trim().is_empty() && !s1.project_description.trim().is_empty() {
        item.project_description = s1.project_description.clone();
      }
      if item.project_achievements.trim().is_empty() && !s1.project_achievements.trim().is_empty() {
        item.project_achievements = s1.project_achievements.clone();
      }
      if item.project_name.trim().is_empty() && !s1.project_name.trim().is_empty() {
        item.project_name = s1.project_name.clone();
      }
    }
  }

  final_data
}

fn looks_incomplete_period(period: &str) -> bool {
  let p = period.trim();
  if p.is_empty() {
    return true;
  }
  let compact = p.replace(' ', "");
  compact.ends_with('-')
    || compact.ends_with('－')
    || compact.ends_with('–')
    || compact.ends_with('—')
    || compact.ends_with("至")
    || compact.ends_with("到")
}

fn is_more_complete_period(candidate: &str, current: &str) -> bool {
  let c = candidate.trim();
  let cur = current.trim();
  if c.is_empty() {
    return false;
  }
  if looks_incomplete_period(cur) && !looks_incomplete_period(c) {
    return true;
  }
  c.chars().count() > cur.chars().count() + 2
}

fn backfill_work_periods_from_stage1(stage1: &ResumeData, final_data: &mut ResumeData) {
  for (idx, final_item) in final_data.work_experience.iter_mut() {
    if let Some(stage1_item) = stage1.work_experience.get(idx) {
      if is_more_complete_period(&stage1_item.period, &final_item.period) {
        resume_parse_log!(
          debug,
          "resume_parse: work period backfill idx={} from='{}' to='{}'",
          idx,
          final_item.period,
          stage1_item.period
        );
        final_item.period = stage1_item.period.clone();
      }
    }
  }
}

fn count_non_empty_work(items: &BTreeMap<String, WorkItem>) -> usize {
  items
    .values()
    .filter(|w| {
      !w.company.trim().is_empty()
        || !w.position.trim().is_empty()
        || !w.period.trim().is_empty()
        || !w.description.trim().is_empty()
    })
    .count()
}

fn count_non_empty_projects(items: &BTreeMap<String, ProjectItem>) -> usize {
  items
    .values()
    .filter(|p| {
      !p.project_name.trim().is_empty()
        || !p.project_description.trim().is_empty()
        || !p.project_achievements.trim().is_empty()
    })
    .count()
}

fn filter_resume_by_source_text(mut data: ResumeData, source_text: &str) -> ResumeData {
  let text_norm = normalize_match_text(source_text);

  let filtered_work = filter_work_experience(&data.work_experience, &text_norm);
  if !filtered_work.is_empty() {
    data.work_experience = filtered_work;
  }

  let filtered_project = filter_project_experience(&data.project_experience, &text_norm);
  if !filtered_project.is_empty() {
    data.project_experience = filtered_project;
  }

  data
}

fn filter_work_experience(input: &BTreeMap<String, WorkItem>, text_norm: &str) -> BTreeMap<String, WorkItem> {
  let mut kept: Vec<WorkItem> = Vec::new();
  for item in input.values() {
    if keep_work_item(item, text_norm) {
      kept.push(item.clone());
    }
  }
  to_indexed_work_map(kept)
}

fn filter_project_experience(input: &BTreeMap<String, ProjectItem>, text_norm: &str) -> BTreeMap<String, ProjectItem> {
  let mut kept: Vec<ProjectItem> = Vec::new();
  for item in input.values() {
    if keep_project_item(item, text_norm) {
      kept.push(item.clone());
    }
  }
  to_indexed_project_map(kept)
}

fn keep_work_item(item: &WorkItem, text_norm: &str) -> bool {
  let company = item.company.trim();
  let position = item.position.trim();
  let period = item.period.trim();
  let desc = item.description.trim();

  if company.is_empty() && position.is_empty() && period.is_empty() && desc.is_empty() {
    return true;
  }

  // 与项目经历一致：公司或职位非空时默认保留，避免原文标点/空格差异导致子串匹配失败而误删整条经历。
  if !company.is_empty() || !position.is_empty() {
    return true;
  }

  if has_text_evidence(company, text_norm) {
    return true;
  }
  let desc_anchor = short_anchor(desc, 24);

  if has_text_evidence(position, text_norm) && has_text_evidence(&desc_anchor, text_norm) {
    return true;
  }
  if has_text_evidence(&desc_anchor, text_norm) {
    return true;
  }

  false
}

fn keep_project_item(item: &ProjectItem, text_norm: &str) -> bool {
  let name = item.project_name.trim();
  let desc = item.project_description.trim();
  let ach = item.project_achievements.trim();

  if name.is_empty() && desc.is_empty() && ach.is_empty() {
    return true;
  }

  // 项目名非空时默认保留，避免模型改写措辞导致“证据匹配不到”而误删。
  if !name.is_empty() {
    return true;
  }
  if has_text_evidence(&short_anchor(desc, 24), text_norm) {
    return true;
  }
  if has_text_evidence(&short_anchor(ach, 24), text_norm) {
    return true;
  }

  false
}

fn to_indexed_work_map(items: Vec<WorkItem>) -> BTreeMap<String, WorkItem> {
  let mut out = BTreeMap::new();
  for (i, item) in items.into_iter().enumerate() {
    out.insert((i + 1).to_string(), item);
  }
  out
}

fn to_indexed_project_map(items: Vec<ProjectItem>) -> BTreeMap<String, ProjectItem> {
  let mut out = BTreeMap::new();
  for (i, item) in items.into_iter().enumerate() {
    out.insert((i + 1).to_string(), item);
  }
  out
}

fn short_anchor(s: &str, max_chars: usize) -> String {
  let t = s.trim();
  if t.is_empty() {
    return String::new();
  }
  t.chars().take(max_chars).collect()
}

fn has_text_evidence(needle: &str, text_norm: &str) -> bool {
  let n = normalize_match_text(needle);
  if n.chars().count() < 2 {
    return false;
  }
  text_norm.contains(&n)
}

fn normalize_match_text(s: &str) -> String {
  s.chars()
    .filter(|c| {
      !c.is_whitespace()
        && !matches!(
          c,
          ',' | '，' | '.' | '。' | ';' | '；' | ':' | '：' | '/' | '\\' | '-' | '_' | '(' | ')' | '（' | '）' | '[' | ']' | '【' | '】' | '"' | '\''
        )
    })
    .flat_map(|c| c.to_lowercase())
    .collect::<String>()
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

fn load_resume_template_content() -> Result<String, AppError> {
  let path = match resolve_template_path("解析结果模板.json") {
    Ok(p) => p,
    Err(e) => {
      resume_parse_log!(
        debug,
        "resume_parse: 模板路径不可用，使用内置模板 err={}",
        e
      );
      return Ok(EMBEDDED_RESUME_TEMPLATE_JSON.to_string());
    }
  };
  if path.exists() {
    let content = std::fs::read_to_string(&path).map_err(|e| {
      AppError::msg(format!(
        "读取模板文件失败：{}。路径：{}",
        e,
        path.display()
      ))
    })?;
    resume_parse_log!(
      debug,
      "resume_parse: 使用外部解析结果模板 path={}",
      path.display()
    );
    return Ok(content);
  }
  resume_parse_log!(
    debug,
    "resume_parse: 未找到外部解析结果模板，使用内置模板"
  );
  Ok(EMBEDDED_RESUME_TEMPLATE_JSON.to_string())
}
