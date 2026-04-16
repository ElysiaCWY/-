# 简历管理（本地解析）

一个基于 Tauri 的 Windows 本地桌面应用，用于简历文本抽取、结构化解析、简历库管理与 JD 规则匹配。

项目目标：
- 数据尽量不出本机
- 在低配机器上可运行
- 优先提供可落地的离线流程（导入 -> 解析 -> 入库 -> 筛选 -> 导出）

## 技术栈

- 桌面框架：Tauri 1.x
- 后端：Rust 2021
- 前端：原生 HTML + CSS + JavaScript
- 模型调用：Ollama 本地服务（`http://127.0.0.1:11434`）

关键依赖（后端）：
- `pdf-extract`：PDF 文本抽取
- `zip` + `quick-xml`：DOCX 文本抽取
- `serde` / `serde_json`：数据序列化
- `regex`：文本清洗与关键词处理
- `dirs`：本地数据目录定位

## 目录结构（核心）

- `src-tauri/src/main.rs`：Tauri 命令注册与入口
- `src-tauri/src/extract/`：简历文件文本抽取
- `src-tauri/src/llm.rs`：调用本地模型进行结构化解析
- `src-tauri/src/jd.rs`：JD 关键词匹配打分（v1）
- `src-tauri/src/storage.rs`：本地 JSON 存储
- `src-tauri/src/schema.rs`：数据结构定义
- `src-tauri/src/validate.rs`：解析结果规范化
- `src-tauri/src/export_js.rs`：导出 `resume_data.js`
- `ui/dashboard.html`：前端单页入口（包含各功能区块）
- `ui/main.js`：前端交互与 Tauri 命令调用
- `ui/style.css`：界面样式
- `start.ps1` / `start.bat`：Windows 一键启动脚本

## 功能说明

## 1. 简历导入与文本抽取

后端命令：`extract_text(file_path)`

当前支持：
- `.pdf`
- `.docx`

当前不支持（会返回明确报错）：
- `.doc`（提示先转 `.docx`）
- 图片类 `.png/.jpg/.jpeg/.webp`（OCR 暂未启用）

## 2. 本地模型结构化解析

后端命令：`parse_resume(text, settings)`

解析策略：
- 基于 Ollama `/api/generate` 的两阶段解析：
	- 第一阶段：优先抽取稳定结构（公司/岗位/时间段/项目名）
	- 第二阶段：在第一阶段骨架上补全描述细节
- 第二阶段失败时自动回退第一阶段结果，避免整条任务失败
- 提取 JSON 并转换为统一结构
- 解析后做字段规范化（空值补全、索引重排、去空白、工作经历去重合并）

设置项：
- `llama_cli_path`
- `model_path`
- `threads`
- `temperature`

## 3. 结果导出与落盘

后端命令：
- `save_parsed_result_json(source_file, resume_obj)`
- `export_js(resume_obj, out_path)`（仍保留，按需手动导出）

默认行为（推荐流程）：
- 批量解析成功后，自动保存结构化 JSON 到本地固定目录
- 自动写入解析结果索引，便于追踪来源文件与导入日期

手动导出：
- 可继续导出 JavaScript 文件（示例 `resume_data.js`）
- 内容结构与项目内模板 `解析结果模板.json` 一致方向

## 4. 简历库管理

后端命令：
- `save_resume_to_library(source_file, resume_obj)`
- `list_resume_library()`
- `delete_resume_record(id)`

简历记录会落到本地 JSON，按创建时间倒序展示。

## 5. JD 管理与匹配

后端命令：
- `save_jd_record(title, text)`
- `list_jd_records()`
- `jd_score_v1(resume_obj, jd_text)`

当前为规则快筛（v1）：
- 从 JD 中提取中英文关键词
- 在结构化简历扁平文本中做包含匹配
- 基于关键词类别做简单加权打分

## 6. 配置文件管理

当前版本已移除设置界面，模型路径与运行参数统一放在项目根目录：
- `app-config.json`

字段说明：
- `llamaCliPath`：Ollama 服务地址（默认 `http://127.0.0.1:11434`）
- `modelPath`：Ollama 模型名（例如 `qwen2.5:3b`）
- `threads`：推理线程数
- `temperature`：采样温度

## 数据存储位置

应用数据目录：
- `%LOCALAPPDATA%/resume-manager/`

其中包括：
- `resumes.json`：简历库数据
- `jds.json`：JD 记录
- `parsed-results/`：每次解析成功后自动保存的 JSON 结果目录
- `parsed-results/parsed-index.json`：解析结果索引（含来源文件、候选人名、导入日期、JSON 路径）

项目根目录文件：
- `app-config.json`：模型路径与运行参数

## 运行方式（Windows）

前置条件：
- Node.js（LTS）
- Rust toolchain（rustup/cargo）

推荐启动：
1. 双击 `start.bat`，或在 PowerShell 执行 `./start.ps1`
2. 首次运行会自动 `npm install`
3. 脚本会启动 `npm run tauri:dev`

也可手动执行：
- `npm run tauri:dev`

## 使用流程（建议）

1. 编辑项目根目录 `app-config.json`，填写 `llamaCliPath` 与 `modelPath`
2. 到“导入与解析”批量导入简历文件并抽取文本
3. 点击“开始批量解析”
4. 系统自动执行：解析 -> 入库 -> 保存解析 JSON（固定目录）
5. 到“简历库”按姓名/学历/技能筛选，查看结构化详情或删除记录
6. 在“JD 管理筛选”粘贴 JD，计算匹配分
7. 需要时手动导出 `resume_data.js`

## 前端交互更新（当前版本）

- 解析区：
	- 去除旧版预览区，改为队列化批处理与进度展示
	- 支持导入后连续批量解析
- 简历库：
	- 列字段调整为：姓名/性别/年龄/最高学历/岗位/工作年限/导入日期/查看详情/删除
	- 支持删除简历记录
- 详情页：
	- 新增独立详情页面（基础信息卡 + 工作经历表 + 项目经历表）
	- 详情内容做 HTML 转义处理，避免渲染注入

## 当前已知限制

- OCR 尚未启用：图片简历不能直接解析
- `.doc` 尚不直接支持
- JD 匹配当前是规则版（非深度语义匹配）
- 模板导出 Word/PDF 在前端仍为占位交互
- 长简历在本地模型上仍可能偶发输出不完整（已做两阶段与回退兜底）
- 当前前端为单页结构，统一在 `dashboard.html` + `main.js` 中完成页面切换与交互

## 后续建议

- 接入离线 OCR（如 PaddleOCR/Tesseract）以支持图片与扫描件
- 增加 `.doc` 转换链路（本地转换后再抽取）
- JD 匹配升级为“规则 + 模型重排”混合评分
- 为模板导出接入真实 Word/PDF 生成能力
- 增加日志与错误诊断页，便于排查模型路径与解析失败问题
