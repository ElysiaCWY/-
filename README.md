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
- 模型调用：Ollama 本地服务（默认 `http://127.0.0.1:11434`）

关键依赖（后端）：

- `pdf-extract`：PDF 文本抽取
- `zip` + `quick-xml`：DOCX 文本抽取
- `serde` / `serde_json`：数据序列化
- `regex`：文本清洗与关键词处理
- `dirs`：本地数据目录定位
- `rusqlite`：SQLite（解析结果索引库，用于 JD 筛选）
- `log` + `env_logger`：控制台日志；解析相关行同时写入项目内 `logs/app.log`（见下文「日志」）

## 目录结构（核心）

- `src-tauri/src/main.rs`：Tauri 命令注册与入口
- `src-tauri/src/extract/`：简历文件文本抽取
- `src-tauri/src/llm.rs`：调用本地模型进行结构化解析（两阶段、结果归一与来源过滤）
- `src-tauri/src/jd.rs`：JD 需求结构化与加权评分（含旧版关键词 v1 接口）
- `src-tauri/src/storage.rs`：本地 JSON 存储、SQLite、解析归档与去重
- `src-tauri/src/schema.rs`：数据结构定义
- `src-tauri/src/validate.rs`：解析结果规范化
- `src-tauri/src/export_js.rs`：导出 `resume_data.js`
- `src-tauri/src/app_log.rs`：应用日志落盘（含 `resume_parse` 宏）
- `ui/dashboard.html`：前端单页入口（包含各功能区块）
- `ui/main.js`：前端交互与 Tauri 命令调用
- `ui/style.css`：界面样式
- `start.ps1` / `start.bat`：Windows 一键启动脚本
- `解析结果模板.json`：结构化输出字段模板（含 `basicInfo.contact` 等）；构建时会 **嵌入程序**，分发绿色包可不附带；若与 exe 同目录放置同名文件则 **覆盖内置模板**

## 功能说明

### 1. 简历导入与文本抽取

后端命令：`extract_text(file_path)`

当前支持：

- `.pdf`
- `.docx`

当前不支持（会返回明确报错）：

- `.doc`（提示先转 `.docx`）
- 图片类 `.png/.jpg/.jpeg/.webp`（OCR 暂未启用）

前端「导入与解析」文件选择器当前仅开放 **`.docx`、`.pdf`**，与上述能力一致。

### 2. 本地模型结构化解析

后端命令：`parse_resume(text, settings)`

解析策略：

- 基于 Ollama `/api/generate` 的 **两阶段** 解析：
  - **第一阶段**：优先抽取稳定结构（公司/岗位/时间段/项目名等）
  - **第二阶段**：在第一阶段骨架上补全描述细节
- **第二阶段失败**时自动回退第一阶段结果，避免整条任务失败
- **第二阶段若将工作经历条数变少**，会 **回退第一阶段的工作经历**（避免模型合并/删减导致只剩一条）
- 简历解析请求中 Ollama **`num_ctx` 为 32768**（可按机器与模型能力在 `llm.rs` 中调整）
- 提取 JSON 后做结构修复与归一（如 `basicInfo` 的 `phone`/`mobile` 等别名归一到 `contact`；证书字段若被模型输出为 `{name,period}` 对象数组，会合并为字符串以符合 `Vec<String>`）
- 解析结束后对来源文本做 **证据过滤**：项目经历在「项目名非空」时默认保留；工作经历在「公司或职位非空」时默认保留，减少原文与模型措辞略不一致时的误删

设置项（见 `app-config.json`）：

- `llamaCliPath`：Ollama 服务地址
- `modelPath`：Ollama 模型名
- `threads`：推理线程数
- `temperature`：采样温度

### 3. 结果导出与落盘

后端命令：

- `save_parsed_result_json(source_file, resume_obj)`
- `export_js(resume_obj, out_path)`（仍保留，按需手动导出）
- `export_resume_pdf` / `export_resume_pdf_from_json`：标准简历模板 **PDF** 导出（JD 页「生成标准简历」流程）

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

## 日志（排查解析与接口问题）

- **控制台**：通过 `env_logger` 输出；启动前可设置环境变量 **`RUST_LOG`**，例如 `RUST_LOG=resume_manager=info` 或 `resume_manager=debug`（Windows PowerShell：`$env:RUST_LOG="resume_manager=debug"`）。
- **文件**：`<项目根>/logs/app.log`（项目根与 `app-config.json` 所在目录一致）。
  - 前端 `appLog` 与部分后端逻辑会追加写入该文件。
  - **简历解析**相关行带前缀 `resume_parse:` / `parse_resume:`，且与控制台 `log` **双写**，便于在无控制台窗口的打包版中排查。

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
- `logs/app.log`：应用与解析诊断日志（见上文「日志」）

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

## 打包与分发给他人（绿色使用）

目标：对方 **不需要** 安装 Node.js、Rust 或克隆本仓库，解压（或安装）后即可使用。

### 发布方：如何构建

在项目根目录执行（需已安装 Node 与 Rust，仅构建机需要）：

```bash
npm install
npm run tauri build
```

构建成功后：

- 可执行文件一般在 `src-tauri/target/release/resume-manager.exe`
- 若生成安装包，还会在 `src-tauri/target/release/bundle/` 下出现 NSIS / MSI 等（可按需选用「安装版」分发）

### 对方机器上的前置条件

- **Windows 10/11**（与当前打包目标一致）
- **Ollama**：本应用解析与 JD 结构化依赖本地大模型，对方需自行安装 [Ollama](https://ollama.com/)，并 `pull` 你在 `app-config.json` 里配置的模型（例如 `qwen2.5:3b`）。应用无法把模型打进安装包，这一点与「完全离线单文件」不同，需提前说明。

### 建议随程序一并提供的文件（最小绿色包）

将下面内容放在 **同一文件夹**（或与 `resume-manager.exe` 的父级目录匹配程序查找逻辑，见下），再打成 zip 发给对方即可：

| 文件 | 说明 |
|------|------|
| `resume-manager.exe` | 构建产物 |
| `app-config.json` | 必填；可预填 `llamaCliPath`（默认 `http://127.0.0.1:11434`）与 `modelPath`，对方按需修改 |
| `解析结果模板.json` | **可选**；未提供时使用 **内置模板**（与仓库中该文件一致）。仅当需要自定义字段说明或改模板时再放在 exe 同目录 |

程序会通过 **`app-config.json` 所在目录** 作为「项目根」，自动创建 `data/`、`parsed-results/`、`logs/` 等，无需手工建目录。

**说明**：简历库与 JD 列表仍保存在系统目录 `%LOCALAPPDATA%/resume-manager/`，与是否绿色包无关；解析结果库与归档 JSON 在项目根下。

### 标准简历 PDF（可选）

- **无 Node 环境**：导出 PDF 时会使用内置纯文本排版（`printpdf`），一般可直接生成；中文显示依赖对方系统已安装常见中文字体。
- **JSON Resume 主题版（排版更接近网页模板）**：程序已 **内置** `jsonresume-theme-local`（与仓库一致）。当「项目根」下已执行过 `npm install`（存在 `node_modules/.bin/resume` 或 `resume.cmd`）且本地 **尚未** 自带主题目录时，首次导出 PDF 会自动在项目根 **释放** 内置主题；若你自行放置或修改了 `jsonresume-theme-local/`，则 **不会覆盖**。仍须本地 `resume-cli`（及 Puppeteer 等依赖）才能走该路径；若未满足，程序会自动回退到上文纯文本 PDF。

### 对方怎么用

1. 解压到你的文件夹，确认同目录有 `app-config.json`（模板已内置，一般无需再带 `解析结果模板.json`）
2. 安装并启动 Ollama，拉取配置中的模型
3. 双击 `resume-manager.exe`

## 使用流程（建议）

1. 编辑项目根目录 `app-config.json`，填写 `llamaCliPath` 与 `modelPath`
2. 到「导入与解析」批量导入 **docx / pdf** 并抽取文本
3. 点击「开始批量解析」
4. 系统自动执行：解析 -> 简历库入库 -> 保存解析 JSON -> **写入/更新 `data/resumes.db`**
5. 到「简历库」按姓名/学历/技能筛选，查看结构化详情或删除记录
6. 在「JD 管理筛选」输入岗位并粘贴 JD，计算结构化匹配分（先 SQL 预过滤再加权排序）
7. 需要时手动导出 `resume_data.js`，或在 JD 页使用 **标准简历模板导出 PDF**

## 前端交互更新（当前版本）

- 解析区：
  - 队列化批处理与进度展示
  - 支持导入后连续批量解析
  - 导入类型说明与选择器为 **docx、pdf**
- 简历库：
  - 列字段：姓名/性别/年龄/最高学历/岗位/工作年限/导入日期/查看详情/删除
  - 支持删除简历记录（联动清理索引与 SQLite）
- 详情页：
  - 独立详情页面（基础信息卡 + 工作经历表 + 项目经历表）
  - 详情内容做 HTML 转义处理，避免渲染注入
- JD 筛选：
  - 匹配结果展示总分及分项分数简写
  - 标准简历模板支持 **导出 PDF**（多选候选人时批量落盘到所选目录）

## 当前已知限制

- OCR 尚未启用：图片简历不能直接解析
- `.doc` 尚不直接支持（需先另存为 `.docx`）
- JD 匹配依赖本地模型提取结构化需求，模型或配置异常时可能影响筛选质量
- 同人去重依赖 **姓名+年龄+联系方式** 三者齐全；若简历未解析出联系方式，则不会触发该去重逻辑
- 超长简历仍受 **模型上下文长度** 与输出稳定性影响；已使用较大 `num_ctx`、两阶段与工作经历条数回退等兜底，极端长文仍可能不完整
- 当前前端为单页结构，统一在 `dashboard.html` + `main.js` 中完成页面切换与交互

## 后续建议

- 接入离线 OCR（如 PaddleOCR/Tesseract）以支持图片与扫描件
- 增加 `.doc` 转换链路（本地 LibreOffice / Word 转换后再抽取）
- 联系方式缺失时的降级去重策略（可配置）
- 按需继续调大 `num_ctx` 或分段解析策略，以适配更长简历
