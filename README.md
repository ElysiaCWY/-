# 简历管理（本地解析）

一个基于 Tauri 的 Windows 本地桌面应用，用于简历文本抽取、结构化解析、简历库管理与 **JD 结构化权重匹配**。

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
- `rusqlite`：SQLite（解析结果索引库，用于 JD 筛选）

## 目录结构（核心）

- `src-tauri/src/main.rs`：Tauri 命令注册与入口
- `src-tauri/src/extract/`：简历文件文本抽取
- `src-tauri/src/llm.rs`：调用本地模型进行结构化解析
- `src-tauri/src/jd.rs`：JD 需求结构化与加权评分（含旧版关键词 v1 接口）
- `src-tauri/src/storage.rs`：本地 JSON 存储、SQLite、解析归档与去重
- `src-tauri/src/schema.rs`：数据结构定义
- `src-tauri/src/validate.rs`：解析结果规范化
- `src-tauri/src/export_js.rs`：导出 `resume_data.js`
- `ui/dashboard.html`：前端单页入口（包含各功能区块）
- `ui/main.js`：前端交互与 Tauri 命令调用
- `ui/style.css`：界面样式
- `start.ps1` / `start.bat`：Windows 一键启动脚本
- `解析结果模板.json`：结构化输出字段模板（含 `basicInfo.contact` 等）

## 功能说明

### 1. 简历导入与文本抽取

后端命令：`extract_text(file_path)`

当前支持：

- `.pdf`
- `.docx`

当前不支持（会返回明确报错）：

- `.doc`（提示先转 `.docx`）
- 图片类 `.png/.jpg/.jpeg/.webp`（OCR 暂未启用）

### 2. 本地模型结构化解析

后端命令：`parse_resume(text, settings)`

解析策略：

- 基于 Ollama `/api/generate` 的两阶段解析：
  - 第一阶段：优先抽取稳定结构（公司/岗位/时间段/项目名等）
  - 第二阶段：在第一阶段骨架上补全描述细节
- 第二阶段失败时自动回退第一阶段结果，避免整条任务失败
- 提取 JSON 并转换为统一结构；`basicInfo` 支持联系方式字段 `contact`（模型亦可能输出 `phone`/`mobile` 等别名，会归一到 `contact`）
- 解析后做字段规范化（空值补全、索引重排、去空白、工作经历按公司合并等）

设置项：

- `llama_cli_path`
- `model_path`
- `threads`
- `temperature`

### 3. 结果导出与落盘

后端命令：

- `save_parsed_result_json(source_file, resume_obj)`
- `export_js(resume_obj, out_path)`（仍保留，按需手动导出）

默认行为（推荐流程）：

- 批量解析成功后，自动保存结构化 JSON 到本地固定目录
- 按岗位做智能归并后归档到子文件夹：
  - 由本地 AI 结合「已有目录 + 当前简历摘要」自动判断归并目录
  - 语义接近岗位会复用已有目录；无法归并时自动创建新目录
- 自动写入解析结果索引 `parsed-results/parsed-index.json`，便于追踪来源文件与导入日期
- **同步写入项目内 SQLite**（见下文「数据存储位置」），供 JD 筛选使用

**同人多版本去重（仅当能形成完整身份键时生效）**

- 身份键：`姓名 + 年龄 + 联系方式`（联系方式会做数字归一，便于比对）
- 三者均非空时，新导入会 **删除该人旧版本** 在简历库、解析索引、落盘 JSON、SQLite 中的记录，只保留最新一次解析结果

手动导出：

- 可继续导出 JavaScript 文件（示例 `resume_data.js`）
- 内容结构与项目内模板 `解析结果模板.json` 一致方向

### 4. 简历库管理

后端命令：

- `save_resume_to_library(source_file, resume_obj)`
- `list_resume_library()`
- `delete_resume_record(id)`

简历记录会落到本地 JSON（`%LOCALAPPDATA%/resume-manager/resumes.json`），按创建时间倒序展示。

删除简历时会 **联动清理**：解析索引、对应 `parsed-results` 下的 JSON 文件、以及 SQLite 中 `parsed_resumes` 相关行。

### 5. JD 管理与匹配

后端命令：

- `save_jd_record(title, text)`
- `list_jd_records()`
- `jd_score_v1(resume_obj, jd_text)`：旧版关键词快筛（保留兼容）
- `jd_filter_by_keywords(position, jdText, limit)`：**当前 JD 页「计算匹配分」使用的主流程**
  - 用本地模型从「岗位 + JD 文本」提取结构化需求（学历门槛、年限、技能与工作/项目关键词等）
  - 在 SQLite 中先做 **SQL 预过滤**（岗位、最低年限、最低学历）
  - 再对候选人计算 **加权总分**（分项 0–100，最终总分 0–100）  
    `总分 = 技能×0.3 + 年限×0.2 + 学历×0.1 + 工作经历×0.2 + 项目×0.2`  
    其中 **年限分项** 使用平滑函数（非阶梯分段）
- `jd_filter_by_model(...)`：可选的纯模型打分路径（与结构化加权并行存在，按需使用）

前端在匹配结果中会展示总分及分项简写（技/年/学/工/项）。

### 6. 配置文件管理

当前版本已移除设置界面，模型路径与运行参数统一放在项目根目录：

- `app-config.json`

字段说明：

- `llamaCliPath`：Ollama 服务地址（默认 `http://127.0.0.1:11434`）
- `modelPath`：Ollama 模型名（例如 `qwen2.5:3b`）
- `threads`：推理线程数
- `temperature`：采样温度

## 数据存储位置

**应用数据目录（仍为系统本地目录）：**

- `%LOCALAPPDATA%/resume-manager/`
  - `resumes.json`：简历库数据
  - `jds.json`：JD 记录

**项目根目录：**

- `data/resumes.db`：**解析结果索引库（SQLite）**，表 `parsed_resumes` 存候选人摘要字段（姓名、年龄、联系方式、岗位、学历、年限数值、技能与工作/项目文本等），供 JD 筛选查询与评分
- 首次使用若仅在旧位置存在 `resumes.db`，会自动 **复制迁移** 到 `data/resumes.db`（旧文件不强制删除）
- `parsed-results/`：每次解析成功后自动保存的结构化 JSON（按岗位子文件夹归档）
- `parsed-results/parsed-index.json`：解析结果索引（含来源文件、候选人名、导入日期、JSON 路径等）

**项目根目录文件：**

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
2. 到「导入与解析」批量导入简历文件并抽取文本
3. 点击「开始批量解析」
4. 系统自动执行：解析 -> 简历库入库 -> 保存解析 JSON -> **写入/更新 `data/resumes.db`**
5. 到「简历库」按姓名/学历/技能筛选，查看结构化详情或删除记录
6. 在「JD 管理筛选」输入岗位并粘贴 JD，计算结构化匹配分（先 SQL 预过滤再加权排序）
7. 需要时手动导出 `resume_data.js`

## 前端交互更新（当前版本）

- 解析区：
  - 队列化批处理与进度展示
  - 支持导入后连续批量解析
- 简历库：
  - 列字段：姓名/性别/年龄/最高学历/岗位/工作年限/导入日期/查看详情/删除
  - 支持删除简历记录（联动清理索引与 SQLite）
- 详情页：
  - 独立详情页面（基础信息卡 + 工作经历表 + 项目经历表）
  - 详情内容做 HTML 转义处理，避免渲染注入
- JD 筛选：
  - 匹配结果展示总分及分项分数简写

## 当前已知限制

- OCR 尚未启用：图片简历不能直接解析
- `.doc` 尚不直接支持
- JD 匹配依赖本地模型提取结构化需求，模型或配置异常时可能影响筛选质量
- 同人去重依赖 **姓名+年龄+联系方式** 三者齐全；若简历未解析出联系方式，则不会触发该去重逻辑
- 模板导出 Word/PDF 在前端仍为占位交互
- 长简历在本地模型上仍可能偶发输出不完整（已做两阶段与回退兜底）
- 当前前端为单页结构，统一在 `dashboard.html` + `main.js` 中完成页面切换与交互

## 后续建议

- 接入离线 OCR（如 PaddleOCR/Tesseract）以支持图片与扫描件
- 增加 `.doc` 转换链路（本地转换后再抽取）
- 联系方式缺失时的降级去重策略（可配置）
- 为模板导出接入真实 Word/PDF 生成能力
- 增加日志与错误诊断页，便于排查模型路径与解析失败问题
