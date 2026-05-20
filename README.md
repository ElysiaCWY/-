# 简历管理（本地解析）

基于 Tauri 的 Windows 本地桌面应用，用于简历文本抽取、结构化解析、简历库管理与 JD 智能匹配筛选。

## 技术栈

- 桌面框架：Tauri 1.x
- 后端：Rust 2021
- 前端：原生 HTML + CSS + JavaScript（ES Module）
- 存储：SQLite（`data/resumes.db`），纯 SQLite 架构
- 模型调用：Ollama、LM Studio、DeepSeek、阿里云 DashScope、火山方舟（豆包）等 OpenAI 兼容接口

关键后端依赖：`pdf-extract`、`zip` + `quick-xml`、`serde` / `serde_json`、`regex`、`rusqlite`、`dirs`、`log` + `env_logger`

## 快速开始

```bash
git clone https://github.com/ElysiaCWY/Resume-database.git
cd Resume-database
npm install
npm run tauri:dev
```

构建发布版：`npm run tauri build`，产物在 `src-tauri/target/release/`。

## 主要功能

| 模块 | 说明 |
|------|------|
| 导入与解析 | 批量导入 PDF/DOCX → 抽取文本 → 调用 LLM 结构化解析 → 自动入库 |
| 简历库 | 按姓名/学历/技能筛选，查看结构化详情，支持多选删除（联动清理） |
| JD 管理筛选 | 多 JD 管理 → 提取结构化需求 → SQL 预筛 → HR 模型精排 → 候选人排名 |
| 标准简历 PDF | 可选多候选人批量导出，内置纯文本 PDF + 可选 JSON Resume 主题版 |
| Word 转 PDF | 批量将 `.docx` 转为 PDF |
| AI 配置 | 界面化管理模型提供商、API Key、推理参数等 |
| Token 消耗 | 自动统计所有 LLM 调用的 token 消耗，按日/按模型查看 |

## 配置（`app-config.json`）

| 字段 | 说明 | 默认值 |
|------|------|--------|
| `llmProvider` | 模型提供商：`ollama`、`lmstudio`、`deepseek`、`dashscope`、`qwen`、`doubao`、`ark`、`volcengine` | `ollama` |
| `llamaCliPath` | Ollama 服务地址（或 LM Studio 地址） | `http://127.0.0.1:11434` |
| `modelPath` | 模型名（如 `qwen2.5:3b`）或本地 `.gguf` 路径 | — |
| `llmApiKey` | 云端 API 密钥（DeepSeek / DashScope / 火山方舟等） | — |
| `threads` | 推理线程数（1–64） | 4 |
| `temperature` | 采样温度（0–2） | 0.0 |
| `cloudMaxOutputTokens` | 云端模型最大输出 token 数（≥2048 生效） | 不限 |
| `disableThinking` | 禁用云端模型思考/推理模式（DeepSeek R1、Qwen3 等） | false |

也可通过环境变量设置 API Key：`DEEPSEEK_API_KEY`、`DASHSCOPE_API_KEY`、`ARK_API_KEY`。

## 数据存储

**所有数据统一存储在项目根目录下，纯 SQLite 单库架构：**

| 位置 | 说明 |
|------|------|
| `data/resumes.db` | 主数据库：简历库（`resume_library`）、JD 记录（`jd_records`）、解析结果（`parsed_resumes`）、Token 消耗（`token_usage`） |
| `app-config.json` | AI 配置文件 |
| `logs/app.log` | 应用与解析诊断日志 |

旧版本中的 `resumes.json`、`jds.json`、`parsed-index.json` 等 JSON 文件已自动迁移到 SQLite 并重命名为 `.bak`（首次启动自动完成，幂等安全）。

## JD 筛选流程

1. **提取 JD 结构化需求**（1 次 LLM 调用）：从岗位名 + JD 文本提取必备技能、加分技能、工作/项目关键词、学历门槛、年限要求
2. **SQL 预过滤**：按最低学历和年限在 `parsed_resumes` 表中快速筛选
3. **加权初筛**：对预筛结果按关键词匹配计算结构化得分并排序，取前 N 名（默认 25，可调至 200）
4. **HR 模型精排**（8 线程并发）：对初筛候选人逐一调用 LLM 深度评估，输出总分、五项分项分（技能/年限/学历/工作/项目）及评估依据（rationale）

精排提示词包含：硬性条件锚定、逐项扣分规则（缺一项必备技能 -5 分）、评分锚定标准（80-100/60-79/40-59/0-39 四段）、批量尺度校准。

## 同人去重

依赖 **姓名 + 年龄 + 联系方式** 三者齐全时自动触发。新导入会删除该人旧版本在简历库及解析结果中的全部记录。

## 使用流程

1. 编辑 `app-config.json` 或在「AI 配置」页面设置模型与参数
2. 「导入与解析」→ 批量导入 docx/pdf → 开始批量解析
3. 解析完成自动入库，可在「简历库」查看/筛选/删除
4. 「JD 管理筛选」→ 新建 JD → 输入岗位名和 JD 文本 → 计算匹配分
5. 匹配结果可按最低分过滤，勾选候选人导出标准简历 PDF

## 已知限制

- 不支持 `.doc` 与图片 OCR
- 同人去重依赖姓名 + 年龄 + 联系方式三者齐全
- 超长简历受模型上下文限制，极端情况可能不完整
- 需要本地或云端 LLM 服务

## 后续建议

- 接入离线 OCR（PaddleOCR/Tesseract）支持扫描件
- `.doc` 转换链路
- 联系方式缺失时的降级去重
- 分段解析策略适配超长简历
