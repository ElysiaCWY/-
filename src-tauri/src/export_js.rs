use crate::errors::AppError;
use crate::schema::ResumeData;
use std::fs;

pub fn write_resume_js(resume: &ResumeData, out_path: &str) -> Result<(), AppError> {
  let json = serde_json::to_string_pretty(resume)?;
  let content = format!(
    "// resume_data.js\n\
const resumeData = {json};\n\n\
// 导出数据（Node.js 环境使用）\n\
module.exports = resumeData;\n"
  );
  fs::write(out_path, content)?;
  Ok(())
}

