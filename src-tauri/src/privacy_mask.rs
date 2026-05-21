//! 发往外部 LLM 前对姓名、电话、邮箱等做占位符脱敏，响应返回后再还原，降低隐私经模型侧泄露的风险。
//! 占位符仅存在于本进程内存中的映射表；不落盘、不单独加密传输（模型仍可见占位符串本身）。

/// 附加在发往模型的用户提示末尾：约束输出 JSON 中原样保留 `__RM_PRIV_NNNN__`。
pub const LLM_PLACEHOLDER_GUARD: &str = "\n【隐私】若上文含形如 __RM_PRIV_0000__ 的占位符（四位递增数字），你在返回的 JSON 字符串中必须 **原样** 保留这些占位符，不得改写为真实手机号、邮箱或姓名。\n";

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct PrivacyTokenMap {
  pairs: Vec<(String, String)>,
}

impl PrivacyTokenMap {
  pub fn is_empty(&self) -> bool {
    self.pairs.is_empty()
  }

  pub fn extend_map(&mut self, other: PrivacyTokenMap) {
    self.pairs.extend(other.pairs);
  }
}

#[derive(Clone)]
struct Span {
  start: usize,
  end: usize,
  text: String,
}

fn re_email() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?i)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("email re")
  })
}

fn re_mobile_cn() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:\+?86[\s\-]?)?1[3-9]\d(?:[\s\-]?\d{4}){2}").expect("mobile re")
  })
}

fn re_landline() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"\b0\d{2,3}[\s\-]?\d{7,8}\b").expect("landline re"))
}

fn re_labeled_phone() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:电话|手机|Tel|Mobile|联系方式)\s*[:：]\s*([\d+\s\-–—]{8,22})").expect("labeled phone")
  })
}

fn re_qq() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"(?i)\bQQ\s*[:：]?\s*(\d{5,12})\b").expect("qq re"))
}

fn re_wechat() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"微信\s*[:：]\s*([^\s\n，,。；;、]{1,32})").expect("wechat re"))
}

fn re_name_labeled() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:姓名|名字|本名|应聘人|求职者|申请人|候选人|Name|Candidate)\s*[:：]\s*([一-龥A-Za-z·•．.\s]{2,20})").expect("name labeled")
  })
}

// ── P0: 首行推断 + 姓氏字典 ────────────────────────────────────────

/// 常见中文姓氏（前 120 个，覆盖 ~90% 人口）
fn common_surnames() -> &'static [&'static str] {
  &[
    "王", "李", "张", "刘", "陈", "杨", "黄", "赵", "周", "吴",
    "徐", "孙", "马", "胡", "朱", "郭", "何", "罗", "高", "林",
    "郑", "梁", "谢", "唐", "许", "邓", "冯", "韩", "曹", "曾",
    "彭", "萧", "蔡", "潘", "田", "董", "袁", "于", "余", "叶",
    "蒋", "杜", "苏", "魏", "程", "吕", "丁", "沈", "任", "姚",
    "卢", "傅", "钟", "姜", "崔", "谭", "廖", "范", "汪", "陆",
    "金", "石", "戴", "贾", "韦", "夏", "邱", "方", "侯", "邹",
    "熊", "孟", "秦", "白", "江", "阎", "薛", "尹", "段", "雷",
    "黎", "史", "龙", "陶", "贺", "顾", "毛", "郝", "龚", "邵",
    "万", "钱", "严", "覃", "武", "莫", "孔", "汤", "向", "常",
    "温", "康", "施", "文", "牛", "樊", "葛", "邢", "安", "齐",
    "易", "乔", "伍", "庞", "颜", "倪", "庄", "聂", "章", "鲁",
  ]
}

fn is_common_surname(c: char) -> bool {
  common_surnames().iter().any(|s| s.chars().next() == Some(c))
}

/// 从简历首部区域检测无标签姓名（跳过 "个人简历" 等标题后的前几行）。
fn collect_names_from_header(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();
  let lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
  if lines.is_empty() {
    return spans;
  }

  let header_words = [
    "个人简历", "简历", "个人简介", "求职简历", "应聘简历",
    "resume", "cv", "curriculum vitae",
    "中文简历", "英文简历",
  ];
  let is_header = |s: &str| {
    let t = s.trim().to_ascii_lowercase();
    header_words.iter().any(|p| t == p.to_ascii_lowercase() || t.starts_with(&p.to_ascii_lowercase()))
  };

  // 跳过标题行，取前 6 行候选
  let candidates: Vec<&&str> = lines.iter().filter(|l| !is_header(l)).take(6).collect();

  for line in candidates {
    let t = line.trim();
    let char_count = t.chars().count();

    if char_count >= 2 && char_count <= 4 {
      let all_cjk = t.chars().all(|c| c >= '\u{4E00}' && c <= '\u{9FFF}' || c >= '\u{3400}' && c <= '\u{4DBF}');
      if all_cjk && is_common_surname(t.chars().next().unwrap()) {
        if let Some(pos) = text.find(t) {
          spans.push(Span { start: pos, end: pos + t.len(), text: t.to_string() });
        }
      }
    } else if char_count >= 3 && char_count <= 20 {
      let has_upper = t.chars().any(|c| c.is_ascii_uppercase());
      let has_bad = t.chars().any(|c| c == '@' || c.is_ascii_digit());
      let alpha_cnt = t.chars().filter(|c| c.is_ascii_alphabetic() || *c == ' ' || *c == '.' || *c == '-').count();
      if has_upper && !has_bad && (alpha_cnt as f64 / char_count.max(1) as f64) > 0.8 {
        if let Some(pos) = text.find(t) {
          spans.push(Span { start: pos, end: pos + t.len(), text: t.to_string() });
        }
      }
    }
  }
  spans
}

/// 全局扫描：姓氏 + 1~2 个 CJK 字符，受边界约束，避免匹配普通词汇。
fn collect_names_by_surname(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();
  let chars: Vec<(usize, char)> = text.char_indices().collect();
  let len = chars.len();
  if len < 2 {
    return spans;
  }

  let is_boundary = |idx: usize| -> bool {
    if idx >= len { return true; }
    let c = chars[idx].1;
    c.is_whitespace() || c.is_ascii_punctuation() || c.is_ascii_alphanumeric()
      || matches!(c, '，' | '。' | '、' | '：' | '；' | '（' | '）' | '【' | '】' | '《' | '》' | '/' | '｜' | '—' | '|')
  };

  let is_cjk = |c: char| -> bool {
    (c >= '\u{4E00}' && c <= '\u{9FFF}') || (c >= '\u{3400}' && c <= '\u{4DBF}')
  };

  for i in 0..len {
    let c = chars[i].1;
    if !is_cjk(c) || !is_common_surname(c) {
      continue;
    }
    if i > 0 && !is_boundary(i - 1) {
      continue;
    }
    let mut j = i + 1;
    while j < len && j < i + 4 && is_cjk(chars[j].1) {
      j += 1;
    }
    let name_len = j - i;
    if name_len < 2 || name_len > 3 {
      continue;
    }
    if j < len && !is_boundary(j) {
      continue;
    }

    let start = chars[i].0;
    let end = if j < len { chars[j].0 } else { text.len() };
    spans.push(Span { start, end, text: text[start..end].to_string() });
  }
  spans
}

// ── 收集所有 Span ────────────────────────────────────────────────────

fn collect_spans(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();

  for m in re_email().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for m in re_mobile_cn().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for m in re_landline().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for cap in re_labeled_phone().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_qq().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_wechat().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_name_labeled().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      let name = m.as_str().trim();
      if name.len() >= 2 && name.chars().count() <= 8 {
        spans.push(Span {
          start: m.start(),
          end: m.end(),
          text: name.to_string(),
        });
      }
    }
  }

  // P0: 首行推断 + 姓氏全局扫描
  spans.extend(collect_names_from_header(text));
  spans.extend(collect_names_by_surname(text));

  spans
}

fn pick_non_overlapping(mut spans: Vec<Span>) -> Vec<Span> {
  spans.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| a.start.cmp(&b.start)));
  let mut picked: Vec<Span> = Vec::new();
  'outer: for sp in spans {
    for p in &picked {
      if !(sp.end <= p.start || sp.start >= p.end) {
        continue 'outer;
      }
    }
    picked.push(sp);
  }
  picked.sort_by_key(|s| s.start);
  picked
}

/// 将 `text` 中检测到的敏感片段替换为 `__RM_PRIV_NNNN__`，并记录还原映射。`next_id` 在多次拼接脱敏时共用，避免占位符重复。
pub fn mask_sensitive_segments(text: &str, next_id: &mut u32) -> (String, PrivacyTokenMap) {
  if text.is_empty() {
    return (String::new(), PrivacyTokenMap::default());
  }
  let spans = pick_non_overlapping(collect_spans(text));
  if spans.is_empty() {
    return (text.to_string(), PrivacyTokenMap::default());
  }

  let mut pairs = Vec::new();
  let mut out = String::with_capacity(text.len() + spans.len() * 20);
  let mut last = 0usize;
  for sp in spans {
    if sp.start > last {
      out.push_str(&text[last..sp.start]);
    }
    let ph = format!("__RM_PRIV_{:04}__", *next_id);
    *next_id += 1;
    pairs.push((ph.clone(), sp.text.clone()));
    out.push_str(&ph);
    last = sp.end;
  }
  if last < text.len() {
    out.push_str(&text[last..]);
  }
  (out, PrivacyTokenMap { pairs })
}

/// 单段文本脱敏（独立计数从 0 开始）。
pub fn mask_sensitive_segments_single(text: &str) -> (String, PrivacyTokenMap) {
  let mut n = 0u32;
  mask_sensitive_segments(text, &mut n)
}

/// 将模型返回 JSON 等字符串中的占位符还原为原文。
pub fn unmask_sensitive_segments(s: &str, map: &PrivacyTokenMap) -> String {
  if map.pairs.is_empty() {
    return s.to_string();
  }
  let mut out = s.to_string();
  for (ph, orig) in map.pairs.iter().rev() {
    out = out.replace(ph.as_str(), orig);
  }
  out
}

/// 从原始文本中直接提取姓名（用于 LLM 解析后姓名为空时的后备方案）。
/// 复用 `collect_names_from_header`（首行推断）和 `collect_names_by_surname`（姓氏扫描）。
pub fn extract_name_from_text(text: &str) -> Option<String> {
  if text.is_empty() {
    return None;
  }
  // 只取姓名相关 span，不混入邮箱/电话等
  let mut spans = collect_names_from_header(text);
  spans.extend(collect_names_by_surname(text));

  // 按长度降序取非重叠项
  spans.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| a.start.cmp(&b.start)));
  let mut picked: Vec<Span> = Vec::new();
  for sp in spans {
    let overlapping = picked.iter().any(|p| !(sp.end <= p.start || sp.start >= p.end));
    if !overlapping {
      picked.push(sp);
    }
  }
  picked.sort_by_key(|s| s.start);

  // 优先返回中文姓名
  for sp in &picked {
    let t = sp.text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 2 && chars.len() <= 4
      && chars.iter().any(|c| *c >= '\u{4E00}' && *c <= '\u{9FFF}')
      && is_common_surname(chars[0])
    {
      return Some(t.to_string());
    }
  }
  // 回退：英文名
  for sp in &picked {
    let t = sp.text.trim();
    if t.chars().count() >= 2
      && t.chars().any(|c| c.is_ascii_uppercase())
      && t.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '.' || c == '-')
    {
      return Some(t.to_string());
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrip_email_and_name() {
    let raw = "姓名：王小明\n手机：13812345678\n邮箱 a@b.com 结尾";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(masked.contains("__RM_PRIV_"));
    assert!(!masked.contains("13812345678"));
    assert!(!masked.contains("a@b.com"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn header_name_inference() {
    let raw = "王小明\n男 | 28岁 | 本科\nJava 开发工程师\n\n工作经历：...";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(masked.contains("__RM_PRIV_"));
    assert!(!masked.contains("王小明"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn header_name_with_title_skip() {
    let raw = "个人简历\n\n李四\n男 | 25岁\n\n教育经历：...";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(!masked.contains("李四"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn surname_based_name_detection() {
    let raw = "候选人 张三 的简历，曾就职于阿里巴巴。";
    let (masked, _map) = mask_sensitive_segments_single(raw);
    // "张三" 应被脱敏（姓氏 "张" 在字典中）
    assert!(!masked.contains("张三"));
  }

  #[test]
  fn no_false_positive_on_common_word() {
    // "黄金" — "黄" 在姓氏表中但 "黄金" 是普通词汇，应在边界约束下不被误匹配
    let raw = "黄金时代的技术栈以 Java 为主。";
    let (masked, _map) = mask_sensitive_segments_single(raw);
    // "黄金" 应该还在（因为 "金" 后面跟 "时"，不在边界位置，不应匹配）
    assert!(masked.contains("黄金"));
  }

  #[test]
  fn extract_name_from_header() {
    assert_eq!(extract_name_from_text("王小明\n男 | 28岁 | 本科"), Some("王小明".into()));
  }

  #[test]
  fn extract_name_from_labeled() {
    assert_eq!(extract_name_from_text("姓名：赵六\n手机：13800000000"), Some("赵六".into()));
  }

  #[test]
  fn extract_name_fallback_empty() {
    assert_eq!(extract_name_from_text("仅测试没有姓名的文本内容"), None);
  }
}
