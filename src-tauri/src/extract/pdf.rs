use crate::errors::AppError;
use std::path::Path;

pub fn extract_pdf_text(path: &Path) -> Result<String, AppError> {
  let text = pdf_extract::extract_text(path)
    .map_err(|e| AppError::msg(format!("PDF 文本抽取失败：{e}")))?;
  Ok(cleanup_text(&text))
}

fn cleanup_text(s: &str) -> String {
  s.replace("\r\n", "\n")
    .replace('\u{00A0}', " ")
    .lines()
    .map(|l| l.trim_end())
    .collect::<Vec<_>>()
    .join("\n")
    .trim()
    .to_string()
}

