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
    Regex::new(r"(?:姓名|名字|本名)\s*[:：]\s*([\u4e00-\u9fa5·•．.\s]{2,12})").expect("name labeled")
  })
}

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
}
