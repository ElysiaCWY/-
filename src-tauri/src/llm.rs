use crate::errors::AppError;
use crate::schema::{AppSettings, JdScreeningIndex, ProjectItem, ResumeData, ResumeParseOutput, WorkItem};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// 单次 LLM 调用的 token 消耗记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsageRecord {
  pub created_at: String,
  pub provider: String,
  pub model: String,
  pub label: String,
  pub prompt_tokens: i64,
  pub completion_tokens: i64,
  pub total_tokens: i64,
}

static TOKEN_USAGE_LOG: Mutex<Vec<TokenUsageRecord>> = Mutex::new(Vec::new());

fn push_token_usage(record: TokenUsageRecord) {
  if let Ok(mut g) = TOKEN_USAGE_LOG.lock() {
    g.push(record);
  }
}

pub fn drain_token_usage_log() -> Vec<TokenUsageRecord> {
  let mut g = TOKEN_USAGE_LOG.lock().unwrap_or_else(|e| e.into_inner());
  std::mem::take(&mut *g)
}

fn now_epoch_str() -> String {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs().to_string())
    .unwrap_or_else(|_| "0".to_string())
}

/// 与 `run_ollama_json` 中 Ollama `num_ctx` 保持一致，便于日志对照
const RESUME_PARSE_NUM_CTX: u32 = 65536;
/// 单次 Ollama 请求最大等待时长（秒），避免任务无限挂起。
const OLLAMA_REQUEST_TIMEOUT_SECS: u64 = 180;
/// 单次生成最大 token 数，避免使用默认较小上限导致 JSON 被截断。
const OLLAMA_NUM_PREDICT: i32 = 8192;
/// OpenAI 兼容 **云端**（DeepSeek / DashScope / 火山方舟）简历解析：单轮含 `resume` + `jdScreeningIndex`，输出较长；可用 `cloudMaxOutputTokens` 再收紧以换时间。
const CLOUD_RESUME_PARSE_MAX_TOKENS: u32 = 24576;
/// 云端 JD 相关单次补全（结构化提取、打分等）略高于本地默认。
const CLOUD_JD_MAX_TOKENS: u32 = 8192;
/// 单阶段简历解析请求最大尝试次数：首次 + 重试。
const OLLAMA_MAX_ATTEMPTS: usize = 2;
/// 重试前等待毫秒。
const OLLAMA_RETRY_DELAY_MS: u64 = 1200;

fn request_timeout_secs_for_label(label: &str) -> u64 {
  // JD 筛选会在一次请求中触发多次模型调用，缩短单次超时可显著降低“卡住”体感。
  if label.starts_with("jd_") {
    45
  } else {
    OLLAMA_REQUEST_TIMEOUT_SECS
  }
}

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
  #[serde(default, alias = "llmApiKey")]
  pub llm_api_key: String,
  /// 云端 `max_tokens` 上限；见 `AppSettings::cloud_max_output_tokens`。
  #[serde(default, alias = "cloudMaxOutputTokens")]
  pub cloud_max_output_tokens: Option<u32>,
  /// 禁用云端模型思考/推理模式。
  #[serde(default)]
  pub disable_thinking: bool,
}

impl From<&AppSettings> for LlmSettings {
  fn from(s: &AppSettings) -> Self {
    Self {
      llama_cli_path: s.llama_cli_path.clone(),
      model_path: s.model_path.clone(),
      threads: s.threads,
      temperature: s.temperature,
      llm_provider: s.llm_provider.clone(),
      llm_api_key: s.llm_api_key.clone(),
      cloud_max_output_tokens: s.cloud_max_output_tokens,
      disable_thinking: s.disable_thinking,
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

fn provider_is_deepseek(raw: &str) -> bool {
  raw.trim().to_ascii_lowercase() == "deepseek"
}

fn provider_is_dashscope(raw: &str) -> bool {
  matches!(
    raw.trim().to_ascii_lowercase().as_str(),
    "dashscope" | "qwen"
  )
}

/// 火山引擎方舟（豆包等）OpenAI 兼容接口，`POST {base}/chat/completions`。
fn provider_is_volc_ark(raw: &str) -> bool {
  matches!(
    raw.trim().to_ascii_lowercase().as_str(),
    "doubao" | "ark" | "volcengine" | "volc"
  )
}

fn normalize_base_url(input: &str, default_url: &str, default_scheme: &str, suffix: &str) -> String {
  let raw = input.trim();
  let base = if raw.is_empty() || raw.to_ascii_lowercase().ends_with(".exe") {
    default_url.to_string()
  } else if raw.starts_with("http://") || raw.starts_with("https://") {
    raw.trim_end_matches('/').to_string()
  } else {
    format!("{}://{}", default_scheme, raw.trim_end_matches('/'))
  };
  let base = base.trim_end_matches('/');
  if suffix.is_empty() {
    return base.to_string();
  }
  if base.to_ascii_lowercase().ends_with(suffix) {
    base.to_string()
  } else {
    format!("{}{}", base, suffix)
  }
}

fn normalize_deepseek_base_url(input: &str) -> String {
  normalize_base_url(input, "https://api.deepseek.com", "https", "/v1")
}

fn resolve_api_key(
  settings: &LlmSettings,
  provider_name: &str,
  env_var_names: &[&str],
) -> Result<String, AppError> {
  let from_config = settings.llm_api_key.trim();
  if !from_config.is_empty() {
    return Ok(from_config.to_string());
  }
  for key in env_var_names {
    if let Ok(v) = std::env::var(key) {
      let v = v.trim();
      if !v.is_empty() {
        return Ok(v.to_string());
      }
    }
  }
  Err(AppError::msg(format!(
    "{}：请在 app-config.json 中配置 llmApiKey，或设置环境变量 {}",
    provider_name, env_var_names[0]
  )))
}

fn deepseek_api_key(settings: &LlmSettings) -> Result<String, AppError> {
  resolve_api_key(settings, "DeepSeek", &["DEEPSEEK_API_KEY"])
}

fn dashscope_api_key(settings: &LlmSettings) -> Result<String, AppError> {
  resolve_api_key(settings, "DashScope", &["DASHSCOPE_API_KEY", "QWEN_API_KEY"])
}

fn volc_ark_api_key(settings: &LlmSettings) -> Result<String, AppError> {
  resolve_api_key(settings, "火山方舟", &["ARK_API_KEY", "VOLCENGINE_API_KEY", "DOUBAO_API_KEY"])
}

fn normalize_dashscope_base_url(input: &str) -> String {
  let base = normalize_base_url(input, "https://dashscope.aliyuncs.com/compatible-mode/v1", "https", "");
  let lower = base.to_ascii_lowercase();
  if lower.ends_with("/v1") {
    return base;
  }
  if lower.ends_with("/compatible-mode") {
    return format!("{}/v1", base);
  }
  format!("{}/compatible-mode/v1", base.trim_end_matches('/'))
}

fn normalize_volc_ark_base_url(input: &str) -> String {
  normalize_base_url(input, "https://ark.cn-beijing.volces.com/api/v3", "https", "/api/v3")
}

pub fn parse_resume_with_llm(text: &str, settings: &LlmSettings) -> Result<ResumeParseOutput, AppError> {
  if settings.model_path.trim().is_empty() {
    resume_parse_log!(error, "resume_parse: 中止，未配置模型名");
    return Err(AppError::msg(
      "请在 app-config.json 的 modelPath 中填写模型名（Ollama 如 qwen2.5:3b；LM Studio 与所选模型 ID 一致；DeepSeek / DashScope 填控制台中的模型 ID；火山方舟如 doubao-seed-2-0-mini-260428）",
    ));
  }

  if looks_like_gguf_path(&settings.model_path) {
    resume_parse_log!(error, "resume_parse: 中止，modelPath 不能为 .gguf 路径");
    return Err(AppError::msg(
      "modelPath 需要填写模型名（如 qwen2.5:3b），不再支持 .gguf 文件路径",
    ));
  }

  if provider_is_deepseek(&settings.llm_provider) {
    let _ = deepseek_api_key(settings)?;
  }
  if provider_is_dashscope(&settings.llm_provider) {
    let _ = dashscope_api_key(settings)?;
  }
  if provider_is_volc_ark(&settings.llm_provider) {
    let _ = volc_ark_api_key(settings)?;
  }

  let base_log = if provider_is_deepseek(&settings.llm_provider) {
    normalize_deepseek_base_url(&settings.llama_cli_path)
  } else if provider_is_dashscope(&settings.llm_provider) {
    normalize_dashscope_base_url(&settings.llama_cli_path)
  } else if provider_is_volc_ark(&settings.llm_provider) {
    normalize_volc_ark_base_url(&settings.llama_cli_path)
  } else if provider_is_lmstudio(&settings.llm_provider) {
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

  let (masked_text, priv_map) = crate::privacy_mask::mask_sensitive_segments_single(text);
  let prompt = build_single_stage_resume_prompt(&tpl_for_prompt, &masked_text, !priv_map.is_empty());
  resume_parse_log!(
    debug,
    "resume_parse: single_stage_prompt_chars={}",
    prompt.chars().count()
  );

  let raw_json = run_ollama_json("resume_parse", &prompt, settings)?;
  resume_parse_log!(
    debug,
    "resume_parse: single_stage 模型原始 JSON 长度={}",
    raw_json.chars().count()
  );

  let raw_json = crate::privacy_mask::unmask_sensitive_segments(&raw_json, &priv_map);
  let out = parse_resume_and_index_from_model_json(&raw_json, text).map_err(|e| {
    resume_parse_log!(
      error,
      "resume_parse: 解析模型 JSON 失败 err={} json_prefix={}",
      e,
      clip(&raw_json, 800)
    );
    AppError::msg(format!(
      "简历解析失败：{}\nJSON原文：{}",
      e,
      clip(&raw_json, 1200)
    ))
  })?;

  resume_parse_log!(
    info,
    "resume_parse: single_stage 成功 work_entries={} project_entries={}",
    out.resume.work_experience.len(),
    out.resume.project_experience.len()
  );

  resume_parse_log!(
    info,
    "resume_parse: 完成（过滤后）name={:?} work_entries={} project_entries={}",
    out.resume.basic_info.name,
    out.resume.work_experience.len(),
    out.resume.project_experience.len()
  );
  Ok(out)
}

/// 模型应返回 `{{ "resume": {{...}}, "jdScreeningIndex": {{...}} }}`；兼容仅返回简历根对象（无索引时程序补全）。
fn parse_resume_and_index_from_model_json(raw: &str, source_text: &str) -> Result<ResumeParseOutput, String> {
  let raw_clean = sanitize_json_control_chars_inside_strings(raw.trim());
  let v: Value = serde_json::from_str(&raw_clean).map_err(|e| e.to_string())?;
  let obj = v.as_object().ok_or_else(|| "模型输出不是 JSON 对象".to_string())?;

  let (resume_json_str, mut jd_index) = if obj.contains_key("resume") && obj.contains_key("jdScreeningIndex") {
    let resume_val = obj.get("resume").cloned().ok_or_else(|| "缺少 resume".to_string())?;
    let resume_str = serde_json::to_string(&resume_val).map_err(|e| e.to_string())?;
    let idx_val = obj.get("jdScreeningIndex").cloned().unwrap_or(Value::Object(Map::new()));
    let jd: JdScreeningIndex = serde_json::from_value(idx_val).unwrap_or_default();
    (resume_str, jd)
  } else if obj.contains_key("basicInfo") {
    (raw_clean, JdScreeningIndex::default())
  } else {
    return Err("模型输出须包含 resume 与 jdScreeningIndex，或为旧版单对象简历（含 basicInfo）".to_string());
  };

  let resume = parse_resume_data_flexible(&resume_json_str)?;
  let resume = filter_resume_by_source_text(resume, source_text);
  jd_index = merge_sparse_jd_index(&resume, jd_index);
  Ok(ResumeParseOutput {
    resume,
    jd_screening_index: jd_index,
  })
}

fn merge_sparse_jd_index(data: &ResumeData, mut jd: JdScreeningIndex) -> JdScreeningIndex {
  let need_fallback = jd.summary_for_jd.trim().is_empty()
    && jd.skill_tags.is_empty()
    && jd.work_bullets.trim().is_empty()
    && jd.project_bullets.trim().is_empty();
  if need_fallback {
    return jd_screening_index_from_resume(data);
  }
  if jd.skill_tags.is_empty() && !data.basic_info.skills.is_empty() {
    jd.skill_tags = data
      .basic_info
      .skills
      .iter()
      .filter(|s| !s.trim().is_empty())
      .take(20)
      .cloned()
      .collect();
  }
  jd
}

fn jd_screening_index_from_resume(data: &ResumeData) -> JdScreeningIndex {
  let skills: Vec<String> = data
    .basic_info
    .skills
    .iter()
    .filter(|s| !s.trim().is_empty())
    .take(20)
    .cloned()
    .collect();
  let work_bullets: String = data
    .work_experience
    .values()
    .take(8)
    .map(|w| {
      let d = clip(w.description.trim(), 160);
      format!("{}｜{}｜{}", w.company.trim(), w.position.trim(), d)
    })
    .filter(|s| !s.trim().is_empty() && s != "｜｜")
    .collect::<Vec<_>>()
    .join("\n");
  let project_bullets: String = data
    .project_experience
    .values()
    .take(8)
    .map(|p| {
      let d = clip(p.project_description.trim(), 120);
      let a = clip(p.project_achievements.trim(), 120);
      format!("{}｜{}｜{}", p.project_name.trim(), d, a)
    })
    .filter(|s| !s.trim().is_empty() && s != "｜｜")
    .collect::<Vec<_>>()
    .join("\n");
  let summary_for_jd = format!(
    "（程序摘要）技能{}项；工作条目{}；项目条目{}。",
    skills.len(),
    data.work_experience.len(),
    data.project_experience.len()
  );
  JdScreeningIndex {
    summary_for_jd,
    skill_tags: skills,
    role_tags: Vec::new(),
    domain_tags: Vec::new(),
    work_bullets,
    project_bullets,
  }
}

fn build_single_stage_resume_prompt(tpl: &str, text: &str, privacy_masked: bool) -> String {
  let privacy_block = if privacy_masked {
    r##"

11. 【隐私占位符】简历原文中部分姓名与联系方式已替换为形如 __RM_PRIV_0000__ 的占位符（与明文对应关系仅保存在本机进程内，不写入模型侧持久化）。你在 JSON 的所有字符串中必须 **原样** 使用这些占位符（含下划线与四位数字），不得改写为真实中文/数字/邮箱；basicInfo.name、basicInfo.contact 及 jdScreeningIndex 中凡对应原文敏感处也必须使用同一占位符，不得编造新的真实隐私。
"##
  } else {
    ""
  };
  format!(
    r#"解析简历（单阶段：完整简历 + JD 筛选索引一次输出），只输出 **一个** JSON 对象。

顶层必须恰好两个字段（不得增删顶层键名）：
- "resume"：与下方「简历模板」同结构的完整对象（camelCase 字段名）。
- "jdScreeningIndex"：供后续岗位匹配使用的精简索引，字段固定为：
```json
{{
  "summaryForJd": "200～500 字中文，概括职业主线、年限感、核心技术栈与 2～4 条可核对成果，勿编造",
  "skillTags": ["最多20个技能关键词，与 resume 中技能/经历可核对"],
  "roleTags": ["如 后端开发，最多6个"],
  "domainTags": ["行业或业务域，最多6个"],
  "workBullets": "换行分隔，每行一条工作要点摘要（公司｜岗位｜要点），每条不超过约80字",
  "projectBullets": "换行分隔，每行一条项目要点（项目名｜职责｜成果摘要）"
}}
```

简历模板（resume 对象须与此一致）：
```json
{tpl}
```

对 resume 对象的要求：
1. 严格按模板字段输出，不要新增或删除 resume 内字段。
2. 工作经历与项目经历尽量完整枚举，按时间倒序；不要把多段经历合并成一段。
3. 每条工作经历 description 写全职责与要点（有原文依据时尽量充实）。
4. 项目经历完整抽取；projectAchievements 承载业绩/指标/成果类表述。
5. basicInfo.skills 须结合经历归纳技术名词，去重，不得凭空捏造。
6. basicInfo.name 仅填姓名或称呼本身（2～4 个汉字或完整英文名），严禁填入：电话号码、手机号、邮箱、在职状态（在职/离职/待业/应届）、性别（男/女）、年龄、学历、岗位名称。这些内容必须写入 contact、gender、age 等对应字段，而非姓名。
7. 严禁混入其他候选人信息。

对 jdScreeningIndex 的要求：
8. 所有内容须能在上方简历原文或 resume 对象中找到依据；不得虚构公司/项目/证书。
9. 仅返回 JSON 对象，不要 markdown 围栏或解释文字。
10. 必须输出完整、可解析的 JSON，不得中途截断。
{privacy_block}
简历原文：
"""{text}""""#,
    tpl = tpl,
    text = text,
    privacy_block = privacy_block
  )
}

fn llm_log_prefix(label: &str) -> &'static str {
  if label == "resume_parse" {
    "resume_parse"
  } else {
    "app_llm"
  }
}

/// 统一入口：Ollama、LM Studio、DeepSeek、DashScope、火山方舟（OpenAI 兼容 `.../chat/completions`）。
pub fn complete_json_prompt(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  if provider_is_deepseek(&settings.llm_provider) {
    run_deepseek_json(label, prompt, settings, params)
  } else if provider_is_dashscope(&settings.llm_provider) {
    run_dashscope_json(label, prompt, settings, params)
  } else if provider_is_volc_ark(&settings.llm_provider) {
    run_volc_ark_json(label, prompt, settings, params)
  } else if provider_is_lmstudio(&settings.llm_provider) {
    run_lmstudio_json(label, prompt, settings, params)
  } else {
    run_ollama_json_params(label, prompt, settings, params)
  }
}

/// 共享重试 + JSON 提取 + 验证循环。`do_request` 每个 attempt 调用一次，负责发送 HTTP 请求并返回原始响应文本。
fn run_llm_with_retry_and_validate(
  label: &str,
  base_url: &str,
  provider_name: &str,
  max_attempts: usize,
  retry_delay_ms: u64,
  log_prefix: &str,
  mut do_request: impl FnMut() -> Result<String, AppError>,
) -> Result<String, AppError> {
  let mut last_err: Option<String> = None;
  for attempt in 1..=max_attempts {
    let raw_text = match do_request() {
      Ok(t) => t,
      Err(e) => {
        let err_text = e.to_string();
        last_err = Some(err_text.clone());
        if attempt < max_attempts {
          resume_parse_log!(
            warn,
            "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}；{}ms 后重试",
            log_prefix, label, attempt, max_attempts,
            base_url, err_text, retry_delay_ms
          );
          thread::sleep(Duration::from_millis(retry_delay_ms));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] HTTP 请求失败（第{}/{}次）{} err={}",
          log_prefix, label, attempt, max_attempts,
          base_url, err_text
        );
        return Err(e);
      }
    };

    let extracted = match extract_json_object(&raw_text) {
      Some(v) => sanitize_json_control_chars_inside_strings(&v),
      None => {
        let err_text = format!(
          "模型输出中未找到 JSON 对象。原始输出前800字符：{}",
          clip(&raw_text, 800)
        );
        last_err = Some(err_text.clone());
        if attempt < max_attempts {
          resume_parse_log!(
            warn,
            "{}: [{}] 无法提取 JSON（第{}/{}次）；{}ms 后重试 raw_prefix={}",
            log_prefix, label, attempt, max_attempts,
            retry_delay_ms, clip(&raw_text, 400)
          );
          thread::sleep(Duration::from_millis(retry_delay_ms));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] 无法从模型输出中提取 JSON，raw_prefix={}",
          log_prefix, label, clip(&raw_text, 800)
        );
        return Err(AppError::msg(err_text));
      }
    };

    match serde_json::from_str::<Value>(&extracted) {
      Ok(_) => return Ok(extracted),
      Err(e) => {
        let err_text = format!("模型输出 JSON 语法错误：{}", e);
        last_err = Some(err_text.clone());
        if attempt < max_attempts {
          resume_parse_log!(
            warn,
            "{}: [{}] JSON 语法错误（第{}/{}次）err={}；{}ms 后重试 json_prefix={}",
            log_prefix, label, attempt, max_attempts,
            e, retry_delay_ms, clip(&extracted, 400)
          );
          thread::sleep(Duration::from_millis(retry_delay_ms));
          continue;
        }
        resume_parse_log!(
          error,
          "{}: [{}] JSON 语法错误（第{}/{}次）err={} json_prefix={}",
          log_prefix, label, attempt, max_attempts,
          e, clip(&extracted, 800)
        );
        return Err(AppError::msg(format!(
          "{}。原始输出前800字符：{}",
          err_text, clip(&extracted, 800)
        )));
      }
    }
  }

  Err(AppError::msg(format!(
    "调用 {} 失败（阶段：{}）：{}",
    provider_name,
    label,
    last_err.unwrap_or_else(|| "未知错误".to_string())
  )))
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
  let timeout_secs = request_timeout_secs_for_label(label);
  resume_parse_log!(
    debug,
    "{}: [{}] POST {} prompt_chars={} timeout_s={} max_attempts={} num_predict={}",
    llm_log_prefix(label),
    label,
    endpoint,
    prompt.chars().count(),
    timeout_secs,
    OLLAMA_MAX_ATTEMPTS,
    np_log
  );

  let log_prefix = llm_log_prefix(label);
  run_llm_with_retry_and_validate(
    label,
    &base_url,
    "Ollama",
    OLLAMA_MAX_ATTEMPTS,
    OLLAMA_RETRY_DELAY_MS,
    log_prefix,
    || {
      let resp = ureq::post(&endpoint)
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(timeout_secs))
        .send_json(body.clone())
        .map_err(|e| {
          AppError::msg(format!(
            "调用 Ollama 失败（阶段：{}，已重试 {} 次）：{}。请确认 Ollama 可访问：{}；若偶发卡住，建议重试本批次。",
            label, OLLAMA_MAX_ATTEMPTS, e, base_url
          ))
        })?;
      let payload: Value = resp.into_json().map_err(|e| {
        resume_parse_log!(
          error,
          "{}: [{}] 解析 Ollama JSON 响应失败: {}",
          log_prefix, label, e
        );
        AppError::msg(format!("解析 Ollama 响应失败：{}", e))
      })?;
      // 记录 token 消耗（Ollama）
      {
        let prompt_tokens = payload.get("prompt_eval_count").and_then(|v| v.as_i64()).unwrap_or(0);
        let completion_tokens = payload.get("eval_count").and_then(|v| v.as_i64()).unwrap_or(0);
        push_token_usage(TokenUsageRecord {
          created_at: now_epoch_str(),
          provider: "ollama".into(),
          model: settings.model_path.trim().to_string(),
          label: label.to_string(),
          prompt_tokens,
          completion_tokens,
          total_tokens: prompt_tokens + completion_tokens,
        });
      }
      payload
        .get("response")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
          resume_parse_log!(
            error,
            "{}: [{}] 响应缺少 response 字段 payload={}",
            log_prefix, label, payload
          );
          AppError::msg(format!("Ollama 响应缺少 response 字段：{}", payload))
        })
    },
  )
}

fn lmstudio_max_tokens(params: &JsonPromptParams) -> u32 {
  let n = params.ollama_num_predict.unwrap_or(4096);
  n.clamp(256, 32768) as u32
}

/// 云端 API 的 `max_tokens`：在本地推导值基础上适度抬高（LM Studio 仍走 `lmstudio_max_tokens`）。
fn openai_compat_max_tokens(
  vendor: OpenAiCompatVendor,
  label: &str,
  params: &JsonPromptParams,
  cloud_max_output_tokens: Option<u32>,
) -> u32 {
  let base = lmstudio_max_tokens(params);
  let computed = match vendor {
    OpenAiCompatVendor::LmStudio => base,
    OpenAiCompatVendor::DeepSeek
    | OpenAiCompatVendor::DashScope
    | OpenAiCompatVendor::VolcArk => {
      if label == "resume_parse" {
        base.max(CLOUD_RESUME_PARSE_MAX_TOKENS).min(65536)
      } else if label.starts_with("jd_") {
        base.max(CLOUD_JD_MAX_TOKENS).min(32768)
      } else {
        base.max(8192).min(32768)
      }
    }
  };
  match (
    vendor,
    cloud_max_output_tokens.filter(|&c| c >= 2048).map(|c| c.min(65536)),
  ) {
    (
      OpenAiCompatVendor::DeepSeek | OpenAiCompatVendor::DashScope | OpenAiCompatVendor::VolcArk,
      Some(cap),
    ) => computed.min(cap),
    _ => computed,
  }
}

#[derive(Clone, Copy)]
enum OpenAiCompatVendor {
  LmStudio,
  DeepSeek,
  DashScope,
  VolcArk,
}

fn run_lmstudio_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  run_openai_compatible_json(label, prompt, settings, params, OpenAiCompatVendor::LmStudio)
}

fn run_deepseek_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  run_openai_compatible_json(label, prompt, settings, params, OpenAiCompatVendor::DeepSeek)
}

fn run_dashscope_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  run_openai_compatible_json(label, prompt, settings, params, OpenAiCompatVendor::DashScope)
}

fn run_volc_ark_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
) -> Result<String, AppError> {
  run_openai_compatible_json(label, prompt, settings, params, OpenAiCompatVendor::VolcArk)
}

fn run_openai_compatible_json(
  label: &str,
  prompt: &str,
  settings: &LlmSettings,
  params: JsonPromptParams,
  vendor: OpenAiCompatVendor,
) -> Result<String, AppError> {
  let base_url = match vendor {
    OpenAiCompatVendor::LmStudio => normalize_lmstudio_base_url(&settings.llama_cli_path),
    OpenAiCompatVendor::DeepSeek => normalize_deepseek_base_url(&settings.llama_cli_path),
    OpenAiCompatVendor::DashScope => normalize_dashscope_base_url(&settings.llama_cli_path),
    OpenAiCompatVendor::VolcArk => normalize_volc_ark_base_url(&settings.llama_cli_path),
  };
  let vendor_zh = match vendor {
    OpenAiCompatVendor::LmStudio => "LM Studio",
    OpenAiCompatVendor::DeepSeek => "DeepSeek",
    OpenAiCompatVendor::DashScope => "DashScope",
    OpenAiCompatVendor::VolcArk => "火山方舟",
  };
  let bearer_header: Option<String> = match vendor {
    OpenAiCompatVendor::LmStudio => None,
    OpenAiCompatVendor::DeepSeek => Some(format!("Bearer {}", deepseek_api_key(settings)?)),
    OpenAiCompatVendor::DashScope => Some(format!("Bearer {}", dashscope_api_key(settings)?)),
    OpenAiCompatVendor::VolcArk => Some(format!("Bearer {}", volc_ark_api_key(settings)?)),
  };
  let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
  let max_tokens = openai_compat_max_tokens(vendor, label, &params, settings.cloud_max_output_tokens);
  let schema_name = "generic_json_schema";
  // 简历解析为 resume + jdScreeningIndex 嵌套结构，OpenAI json_schema 约束不适用，统一走 text。
  let structured_stage = false;
  // DeepSeek / DashScope 等当前不支持或不稳定支持 response_format=json_schema，直接走 text。
  let schema_fallback_enabled = matches!(
    vendor,
    OpenAiCompatVendor::DeepSeek | OpenAiCompatVendor::DashScope | OpenAiCompatVendor::VolcArk
  );
  let mut timeout_secs = request_timeout_secs_for_label(label);
  if matches!(
    vendor,
    OpenAiCompatVendor::DeepSeek | OpenAiCompatVendor::DashScope | OpenAiCompatVendor::VolcArk
  ) {
    if label == "resume_parse" {
      timeout_secs = timeout_secs.max(300);
    } else if label.starts_with("jd_") {
      timeout_secs = timeout_secs.max(120);
    } else {
      timeout_secs = timeout_secs.max(180);
    }
  }

  resume_parse_log!(
    debug,
    "{}: [{}] POST {} prompt_chars={} timeout_s={} max_attempts={} max_tokens={}",
    llm_log_prefix(label),
    label,
    endpoint,
    prompt.chars().count(),
    timeout_secs,
    OLLAMA_MAX_ATTEMPTS,
    max_tokens
  );

  let log_prefix = llm_log_prefix(label);
  let schema_fallback = std::cell::Cell::new(schema_fallback_enabled);
  run_llm_with_retry_and_validate(
    label,
    &base_url,
    vendor_zh,
    OLLAMA_MAX_ATTEMPTS,
    OLLAMA_RETRY_DELAY_MS,
    log_prefix,
    || {
      let sf = schema_fallback.get();
      let response_format = if structured_stage && !sf {
        json!({
          "type": "json_schema",
          "json_schema": {
            "name": schema_name,
            "strict": true,
            "schema": resume_json_schema()
          }
        })
      } else {
        json!({ "type": "text" })
      };
      let mut body_map = serde_json::Map::new();
      body_map.insert("model".into(), json!(settings.model_path.trim()));
      body_map.insert("messages".into(), json!([{ "role": "user", "content": prompt }]));
      body_map.insert("temperature".into(), json!(params.temperature));
      body_map.insert("max_tokens".into(), json!(max_tokens));
      body_map.insert("stream".into(), json!(false));
      body_map.insert("response_format".into(), response_format);
      if settings.disable_thinking {
        match vendor {
          OpenAiCompatVendor::DashScope => {
            body_map.insert("enable_thinking".into(), json!(false));
          }
          OpenAiCompatVendor::DeepSeek | OpenAiCompatVendor::VolcArk => {
            body_map.insert("thinking".into(), json!({"type": "disabled"}));
          }
          _ => {}
        }
      }
      let body = Value::Object(body_map);
      let mut req_builder = ureq::post(&endpoint)
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(timeout_secs));
      if let Some(ref auth) = bearer_header {
        req_builder = req_builder.set("Authorization", auth.as_str());
      }
      let result = req_builder.send_json(body.clone());

      let resp = match result {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
          let err_body = resp
            .into_string()
            .unwrap_or_else(|_| "<empty error body>".to_string());
          if structured_stage && !sf && code == 400 {
            schema_fallback.set(true);
            resume_parse_log!(
              warn,
              "{}: [{}] {} 不支持 json_schema，自动降级为 text 后重试；err={}",
              log_prefix, label, vendor_zh,
              clip(&format!("status code {}: {}", code, err_body), 500)
            );
            return Err(AppError::msg("schema_fallback_retry"));
          }
          let hint = match vendor {
            OpenAiCompatVendor::LmStudio => format!(
              "请确认已启动本地服务器且地址为：{}（OpenAI 兼容 /v1）；若偶发卡住，建议重试本批次。", base_url
            ),
            OpenAiCompatVendor::DeepSeek => format!(
              "请确认 llmApiKey / 环境变量 DEEPSEEK_API_KEY 正确，且可访问：{}。", base_url
            ),
            OpenAiCompatVendor::DashScope => format!(
              "请确认 llmApiKey / 环境变量 DASHSCOPE_API_KEY 正确，且可访问：{}（国际区可改 llamaCliPath 为国际 compatible-mode 地址）。", base_url
            ),
            OpenAiCompatVendor::VolcArk => format!(
              "请确认 llmApiKey / 环境变量 ARK_API_KEY 正确，且可访问：{}（其他地域 endpoint 可改 llamaCliPath）。", base_url
            ),
          };
          return Err(AppError::msg(format!(
            "调用 {} 失败（阶段：{}，已重试 {} 次）：status code {}: {}。{}",
            vendor_zh, label, OLLAMA_MAX_ATTEMPTS, code, err_body, hint
          )));
        }
        Err(e) => {
          let hint = match vendor {
            OpenAiCompatVendor::LmStudio => format!(
              "请确认已启动本地服务器且地址为：{}（OpenAI 兼容 /v1）；若偶发卡住，建议重试本批次。", base_url
            ),
            OpenAiCompatVendor::DeepSeek => format!(
              "请确认 llmApiKey / 环境变量 DEEPSEEK_API_KEY 正确，且可访问：{}。", base_url
            ),
            OpenAiCompatVendor::DashScope => format!(
              "请确认 llmApiKey / 环境变量 DASHSCOPE_API_KEY 正确，且可访问：{}（国际区可改 llamaCliPath 为国际 compatible-mode 地址）。", base_url
            ),
            OpenAiCompatVendor::VolcArk => format!(
              "请确认 llmApiKey / 环境变量 ARK_API_KEY 正确，且可访问：{}（其他地域 endpoint 可改 llamaCliPath）。", base_url
            ),
          };
          return Err(AppError::msg(format!(
            "调用 {} 失败（阶段：{}，已重试 {} 次）：{}。{}",
            vendor_zh, label, OLLAMA_MAX_ATTEMPTS, e, hint
          )));
        }
      };

      let payload: Value = resp.into_json().map_err(|e| {
        resume_parse_log!(
          error,
          "{}: [{}] 解析 {} JSON 响应失败: {}",
          log_prefix, label, vendor_zh, e
        );
        AppError::msg(format!("解析 {} 响应失败：{}", vendor_zh, e))
      })?;

      if let Some(err) = payload.get("error") {
        let msg = err.to_string();
        resume_parse_log!(
          error,
          "{}: [{}] {} 返回 error: {}",
          log_prefix, label, vendor_zh, msg
        );
        return Err(AppError::msg(format!("{} 返回错误：{}", vendor_zh, msg)));
      }

      // 记录 token 消耗（OpenAI 兼容云端 API）
      {
        let usage = payload.get("usage");
        let prompt_tokens = usage.and_then(|u| u.get("prompt_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
        let completion_tokens = usage.and_then(|u| u.get("completion_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
        let total_tokens = usage.and_then(|u| u.get("total_tokens")).and_then(|v| v.as_i64()).unwrap_or(0);
        push_token_usage(TokenUsageRecord {
          created_at: now_epoch_str(),
          provider: vendor_zh.to_lowercase().replace(" ", "_"),
          model: settings.model_path.trim().to_string(),
          label: label.to_string(),
          prompt_tokens,
          completion_tokens,
          total_tokens,
        });
      }

      let first_choice = payload
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .ok_or_else(|| {
          resume_parse_log!(
            error,
            "{}: [{}] 响应缺少 choices[0] payload={}",
            log_prefix, label, payload
          );
          AppError::msg(format!("{} 响应缺少 choices[0]：{}", vendor_zh, payload))
        })?;

      let finish_reason = first_choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
      let message = first_choice.get("message").and_then(|m| m.as_object()).ok_or_else(|| {
        resume_parse_log!(
          error,
          "{}: [{}] 响应缺少 choices[0].message payload={}",
          log_prefix, label, payload
        );
        AppError::msg(format!("{} 响应缺少 choices[0].message：{}", vendor_zh, payload))
      })?;
      let content_text = message.get("content").and_then(|v| v.as_str()).unwrap_or("");
      let reasoning_text = message.get("reasoning_content").and_then(|v| v.as_str()).unwrap_or("");

      if !content_text.trim().is_empty() {
        Ok(content_text.to_string())
      } else if !reasoning_text.trim().is_empty() {
        resume_parse_log!(
          warn,
          "{}: [{}] content 为空，回退使用 reasoning_content finish_reason={}",
          log_prefix, label, finish_reason
        );
        Ok(reasoning_text.to_string())
      } else {
        resume_parse_log!(
          error,
          "{}: [{}] message.content 与 reasoning_content 均为空 finish_reason={} payload={}",
          log_prefix, label, finish_reason, payload
        );
        Err(AppError::msg(format!(
          "{} 响应内容为空（content/reasoning_content 均为空，finish_reason={}）",
          vendor_zh, finish_reason
        )))
      }
    },
  )
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
  normalize_base_url(input, "http://127.0.0.1:1234", "http", "/v1")
}

fn normalize_ollama_base_url(input: &str) -> String {
  normalize_base_url(input, "http://127.0.0.1:11434", "http", "")
}

fn looks_like_gguf_path(value: &str) -> bool {
  let v = value.trim().to_ascii_lowercase();
  v.ends_with(".gguf")
}

/// 部分云端模型会在 JSON **字符串值**内直接插入换行等控制字符（非法 JSON）。在解析前将其替换为空格，避免 `serde_json` 报错。
fn sanitize_json_control_chars_inside_strings(raw: &str) -> String {
  let mut out = String::with_capacity(raw.len());
  let mut in_string = false;
  let mut escape = false;
  for ch in raw.chars() {
    if in_string {
      if escape {
        out.push(ch);
        escape = false;
        continue;
      }
      match ch {
        '\\' => {
          out.push(ch);
          escape = true;
        }
        '"' => {
          in_string = false;
          out.push(ch);
        }
        ch if ch <= '\u{1F}' => out.push(' '),
        _ => out.push(ch),
      }
    } else if ch == '"' {
      in_string = true;
      out.push(ch);
    } else {
      out.push(ch);
    }
  }
  out
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
  let candidate = sanitize_json_control_chars_inside_strings(&candidate);
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
  to_indexed_map(kept)
}

fn filter_project_experience(input: &BTreeMap<String, ProjectItem>, text_norm: &str) -> BTreeMap<String, ProjectItem> {
  let mut kept: Vec<ProjectItem> = Vec::new();
  for item in input.values() {
    if keep_project_item(item, text_norm) {
      kept.push(item.clone());
    }
  }
  to_indexed_map(kept)
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

fn to_indexed_map<T>(items: Vec<T>) -> BTreeMap<String, T> {
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
