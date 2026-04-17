use crate::errors::AppError;
use printpdf::ops::PdfFontHandle;
use printpdf::{Mm, Op, ParsedFont, PdfDocument, PdfPage, PdfSaveOptions, Point, Pt, TextItem};
use regex::Regex;
use std::fs::{self, File};
use std::io::BufWriter;
use std::sync::OnceLock;

fn try_load_windows_cjk_font(doc: &mut PdfDocument) -> Option<PdfFontHandle> {
  // 优先选择常见中文字体，避免中文内容在 PDF 中乱码。
  // 对 TTC 字体尝试前几个 face index，提高兼容性。
  let candidates = [
    ("C:\\Windows\\Fonts\\msyh.ttc", true),
    ("C:\\Windows\\Fonts\\msyh.ttf", false),
    ("C:\\Windows\\Fonts\\simhei.ttf", false),
    ("C:\\Windows\\Fonts\\simsun.ttc", true),
  ];
  for (path, is_collection) in candidates {
    let bytes = match fs::read(path) {
      Ok(v) => v,
      Err(_) => continue,
    };
    let indexes: &[usize] = if is_collection { &[0, 1, 2] } else { &[0] };
    for idx in indexes {
      let mut warnings = Vec::new();
      if let Some(parsed) = ParsedFont::from_bytes(&bytes, *idx, &mut warnings) {
        let font_id = doc.add_font(&parsed);
        return Some(PdfFontHandle::External(font_id));
      }
    }
  }
  None
}

#[derive(Clone, Copy)]
enum LineKind {
  H1,
  H2,
  Normal,
}

#[derive(Clone)]
struct RenderLine {
  text: String,
  kind: LineKind,
}

fn char_width_units(ch: char) -> f32 {
  if ch == ' ' {
    return 0.35;
  }
  if ch.is_ascii() {
    if ch.is_ascii_punctuation() {
      return 0.45;
    }
    return 0.55;
  }
  // CJK 字符通常占满一个字宽。
  1.0
}

fn wrap_by_units(text: &str, max_units: f32) -> Vec<String> {
  if text.trim().is_empty() {
    return vec![String::new()];
  }

  let mut out: Vec<String> = Vec::new();
  let mut current = String::new();
  let mut used = 0.0f32;
  for ch in text.chars() {
    let w = char_width_units(ch);
    if used + w > max_units && !current.is_empty() {
      out.push(current.trim_end().to_string());
      current.clear();
      used = 0.0;
    }
    current.push(ch);
    used += w;
  }
  if !current.is_empty() {
    out.push(current.trim_end().to_string());
  }
  out
}

fn numbered_marker_regex() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    // 只匹配“短编号”分段（如 1、 2.），避免把年份 2018.06 误识别成编号。
    Regex::new(r"(?:^|[\s；;，,。])(?P<num>\d{1,2})\s*[、.．]")
      .expect("invalid numbered marker regex")
  })
}

fn time_range_regex() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"^\s*\d{4}(?:[./-]\d{1,2})?\s*[-~—至]\s*(?:\d{4}(?:[./-]\d{1,2})?|至今)\s*$")
      .expect("invalid time range regex")
  })
}

fn looks_like_time_range(s: &str) -> bool {
  time_range_regex().is_match(s.trim())
}

fn preprocess_lines_for_time_tail(content: &str) -> Vec<String> {
  let mut out: Vec<String> = Vec::new();
  for raw in content.lines() {
    let trimmed = raw.trim();
    if looks_like_time_range(trimmed) && !out.is_empty() {
      if let Some(prev) = out.last_mut() {
        let prev_trimmed = prev.trim_end();
        if prev_trimmed.ends_with('/')
          || prev_trimmed.ends_with('／')
          || prev_trimmed.ends_with('-')
          || prev_trimmed.contains(" / ")
          || prev_trimmed.contains("／")
        {
          let merged = format!("{} {}", prev_trimmed, trimmed);
          *prev = merged;
          continue;
        }
      }
    }
    out.push(raw.trim_end().to_string());
  }
  out
}

/// 对“公司 / 岗位 / 时间”这类尾部为时间段的文本做保护：
/// 自动保留“ / 时间”在同一行，避免时间单独折到下一行。
fn wrap_preserve_trailing_time(text: &str, max_units: f32) -> Vec<String> {
  let normalized = text.replace('／', "/");
  let slash_count = normalized.matches('/').count();
  // 强规则：公司/岗位/时间结构优先保证不拆分，避免时间换行。
  if slash_count >= 2 && (normalized.contains("至今") || time_range_regex().is_match(&normalized)) {
    return vec![text.trim().to_string()];
  }

  let parts: Vec<&str> = normalized.split('/').map(|x| x.trim()).collect();
  if parts.len() < 3 || !text.contains('/') {
    return wrap_by_units(text, max_units);
  }
  let tail = parts.last().map(|x| x.trim()).unwrap_or("");
  if !looks_like_time_range(tail) {
    return wrap_by_units(text, max_units);
  }

  let head = parts[..parts.len() - 1].join(" / ");
  let tail_text = format!(" / {}", tail);
  let tail_units: f32 = tail_text.chars().map(char_width_units).sum();
  let head_units_limit = (max_units - tail_units).max(10.0);

  let mut wrapped = wrap_by_units(&head, head_units_limit);
  if wrapped.is_empty() {
    wrapped.push(String::new());
  }
  if let Some(last) = wrapped.last_mut() {
    last.push_str(&tail_text);
  }
  wrapped
}

/// 将“1、2、/1.2.”这类编号切成多个子段落，便于后续自动换行时保持编号可读性。
fn split_numbered_sections(text: &str) -> Vec<String> {
  let re = numbered_marker_regex();
  let mut starts: Vec<usize> = re
    .captures_iter(text)
    .filter_map(|c| c.name("num").map(|m| m.start()))
    .filter(|x| *x > 0)
    .collect();
  if starts.is_empty() {
    return vec![text.trim().to_string()];
  }
  starts.sort_unstable();
  starts.dedup();

  let mut out = Vec::new();
  let mut cursor = 0usize;
  for start in starts {
    if start <= cursor || start > text.len() {
      continue;
    }
    let seg = text[cursor..start].trim();
    if !seg.is_empty() {
      out.push(seg.to_string());
    }
    cursor = start;
  }
  if cursor < text.len() {
    let tail = text[cursor..].trim();
    if !tail.is_empty() {
      out.push(tail.to_string());
    }
  }
  if out.is_empty() {
    vec![text.trim().to_string()]
  } else {
    out
  }
}

fn parse_and_wrap_lines(content: &str) -> Vec<RenderLine> {
  let mut lines: Vec<RenderLine> = Vec::new();
  for raw in preprocess_lines_for_time_tail(content) {
    let line = raw.trim_end();
    if line.trim().is_empty() {
      lines.push(RenderLine {
        text: String::new(),
        kind: LineKind::Normal,
      });
      continue;
    }

    if let Some(rest) = line.strip_prefix("# ") {
      for x in wrap_by_units(rest.trim(), 32.0) {
        lines.push(RenderLine {
          text: x,
          kind: LineKind::H1,
        });
      }
      continue;
    }

    if let Some(rest) = line.strip_prefix("## ") {
      for x in wrap_by_units(rest.trim(), 40.0) {
        lines.push(RenderLine {
          text: x,
          kind: LineKind::H2,
        });
      }
      continue;
    }

    if let Some(rest) = line.strip_prefix("  - ") {
      let sections = split_numbered_sections(rest);
      for section in sections {
        let wrapped = wrap_preserve_trailing_time(&section, 39.0);
        for (idx, x) in wrapped.into_iter().enumerate() {
          lines.push(RenderLine {
            text: if idx == 0 { format!("    - {}", x) } else { format!("      {}", x) },
            kind: LineKind::Normal,
          });
        }
      }
      continue;
    }

    if let Some(rest) = line.strip_prefix("- ") {
      let sections = split_numbered_sections(rest);
      for (section_idx, section) in sections.into_iter().enumerate() {
        let wrapped = wrap_preserve_trailing_time(&section, 43.0);
        for (idx, x) in wrapped.into_iter().enumerate() {
          lines.push(RenderLine {
            text: if section_idx == 0 && idx == 0 {
              format!("• {}", x)
            } else {
              format!("  {}", x)
            },
            kind: LineKind::Normal,
          });
        }
      }
      continue;
    }

    let sections = split_numbered_sections(line);
    for section in sections {
      for x in wrap_by_units(&section, 45.0) {
        lines.push(RenderLine {
          text: x,
          kind: LineKind::Normal,
        });
      }
    }
  }
  lines
}

fn line_height_for(kind: LineKind) -> f32 {
  match kind {
    LineKind::H1 => 26.0,
    LineKind::H2 => 19.0,
    LineKind::Normal => 14.0,
  }
}

fn font_size_for(kind: LineKind) -> f32 {
  match kind {
    LineKind::H1 => 22.0,
    LineKind::H2 => 14.0,
    LineKind::Normal => 11.0,
  }
}

pub fn write_resume_pdf(content: &str, out_path: &str) -> Result<(), AppError> {
  let mut doc = PdfDocument::new("标准简历");
  let font = try_load_windows_cjk_font(&mut doc)
    .ok_or_else(|| AppError::msg("未找到可用中文字体，无法生成不乱码 PDF。请确认系统存在微软雅黑/黑体/宋体字体。"))?;
  let lines = parse_and_wrap_lines(content);
  let page_text_height = 760.0f32;

  let mut pages: Vec<Vec<RenderLine>> = Vec::new();
  let mut current_page: Vec<RenderLine> = Vec::new();
  let mut used_height = 0.0f32;
  for line in lines {
    let mut h = line_height_for(line.kind);
    if matches!(line.kind, LineKind::H1 | LineKind::H2) {
      h += 4.0;
    }
    if used_height + h > page_text_height && !current_page.is_empty() {
      pages.push(current_page);
      current_page = Vec::new();
      used_height = 0.0;
    }
    used_height += h;
    current_page.push(line);
  }
  if !current_page.is_empty() {
    pages.push(current_page);
  }

  for page_lines in pages {
    let mut ops: Vec<Op> = Vec::new();
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
      pos: Point::new(Mm(16.0), Mm(284.0)),
    });
    for line in page_lines {
      let line_height = line_height_for(line.kind);
      let extra_gap = if matches!(line.kind, LineKind::H1 | LineKind::H2) { 4.0 } else { 0.0 };
      ops.push(Op::SetFont {
        font: font.clone(),
        size: Pt(font_size_for(line.kind)),
      });
      ops.push(Op::SetLineHeight { lh: Pt(line_height + extra_gap) });
      ops.push(Op::ShowText {
        items: vec![TextItem::Text(line.text)],
      });
      ops.push(Op::AddLineBreak);
    }
    ops.push(Op::EndTextSection);
    doc.pages.push(PdfPage::new(Mm(210.0), Mm(297.0), ops));
  }

  let file = File::create(out_path)?;
  let mut writer = BufWriter::new(file);
  let mut warnings = Vec::new();
  doc.save_writer(&mut writer, &PdfSaveOptions::default(), &mut warnings);
  Ok(())
}
