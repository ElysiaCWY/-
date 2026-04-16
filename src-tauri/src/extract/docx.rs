use crate::errors::AppError;
use quick_xml::events::Event;
use quick_xml::Reader;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use zip::ZipArchive;

pub fn extract_docx_text(path: &Path) -> Result<String, AppError> {
  let f = File::open(path)?;
  let mut zip = ZipArchive::new(f).map_err(|e| AppError::msg(format!("DOCX 读取失败：{e}")))?;
  let mut doc = zip
    .by_name("word/document.xml")
    .map_err(|_| AppError::msg("DOCX 缺少 word/document.xml"))?;

  let mut xml = String::new();
  doc.read_to_string(&mut xml)?;
  Ok(extract_text_from_document_xml(&xml))
}

fn extract_text_from_document_xml(xml: &str) -> String {
  let mut reader = Reader::from_str(xml);
  reader.trim_text(true);

  let mut buf = Vec::new();
  let mut out = String::new();
  let mut in_text = false;

  loop {
    match reader.read_event_into(&mut buf) {
      Ok(Event::Start(e)) => {
        if e.name().as_ref().ends_with(b"t") {
          in_text = true;
        }
        if e.name().as_ref().ends_with(b"p") {
          if !out.ends_with('\n') && !out.is_empty() {
            out.push('\n');
          }
        }
      }
      Ok(Event::End(e)) => {
        if e.name().as_ref().ends_with(b"t") {
          in_text = false;
        }
      }
      Ok(Event::Text(t)) => {
        if in_text {
          if let Ok(s) = t.unescape() {
            out.push_str(&s);
          }
        }
      }
      Ok(Event::Eof) => break,
      Err(_) => break,
      _ => {}
    }
    buf.clear();
  }

  out.replace("\r\n", "\n")
    .replace('\u{00A0}', " ")
    .lines()
    .map(|l| l.trim_end())
    .collect::<Vec<_>>()
    .join("\n")
    .trim()
    .to_string()
}

