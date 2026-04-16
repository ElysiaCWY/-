mod docx;
mod pdf;

use crate::errors::AppError;
use std::path::Path;

pub fn extract_text_from_path(path: &str) -> Result<String, AppError> {
  let p = Path::new(path);
  let ext = p
    .extension()
    .and_then(|s| s.to_str())
    .unwrap_or("")
    .to_ascii_lowercase();

  match ext.as_str() {
    "pdf" => pdf::extract_pdf_text(p),
    "docx" => docx::extract_docx_text(p),
    "doc" => Err(AppError::msg(
      "暂不直接支持 .doc（老 Word）。建议用 Word 另存为 .docx 后再导入。",
    )),
    "png" | "jpg" | "jpeg" | "webp" => Err(AppError::msg(
      "图片/扫描件 OCR 尚未启用。当前版本请先将图片内容转为文本或 PDF 文本版后再导入。",
    )),
    _ => Err(AppError::msg("不支持的文件类型")),
  }
}

