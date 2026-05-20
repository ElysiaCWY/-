import { $, countImportedToday } from "./utils.js";

// ── DOM element references ──

export const pageTitle = $("pageTitle");
export const currentFile = $("currentFile");
export const parseProgressText = $("parseProgressText");
export const parseQueue = $("parseQueue");
export const recentList = $("recentList");
export const libraryTableBody = $("libraryTableBody");
export const libraryTotalCount = $("libraryTotalCount");
export const libraryKeyword = $("libraryKeyword");
export const libraryDegree = $("libraryDegree");
export const libraryYears = $("libraryYears");
export const librarySkill = $("librarySkill");
export const libraryFilterBtn = $("libraryFilterBtn");
export const libraryResetBtn = $("libraryResetBtn");
export const libraryDeleteSelectedBtn = $("libraryDeleteSelectedBtn");
export const librarySelectAll = $("librarySelectAll");
export const resumeDetailContent = $("resumeDetailContent");
export const resumeDetailBasic = $("resumeDetailBasic");
export const resumeDetailEduBody = $("resumeDetailEduBody");
export const resumeDetailWorkBody = $("resumeDetailWorkBody");
export const resumeDetailProjectBody = $("resumeDetailProjectBody");
export const resumeDetailJdSummary = $("resumeDetailJdSummary");
export const resumeDetailBack = $("resumeDetailBack");
export const jdCompareBack = $("jdCompareBack");
export const jdInput = $("jdInput");
export const jdPositionInput = $("jdPositionInput");
export const jdMinScore = $("jdMinScore");
export const jdResultTable = $("jdResultTable");
export const jdCompareBtn = $("jdCompareBtn");
export const jdCompareReportContent = $("jdCompareReportContent");
export const jdList = $("jdList");
export const btnNewJd = $("btnNewJd");
export const jdFilterProgressWrap = $("jdFilterProgressWrap");
export const jdFilterProgressPhase = $("jdFilterProgressPhase");
export const jdFilterProgressFrac = $("jdFilterProgressFrac");
export const jdFilterProgressBar = $("jdFilterProgressBar");
export const scoreOut = $("scoreOut");
export const tplCandidateList = $("tplCandidateList");
export const tplName = $("tplName");
export const tplGender = $("tplGender");
export const tplAge = $("tplAge");
export const tplContact = $("tplContact");
export const tplSkill = $("tplSkill");
export const tplExportPdf = $("tplExportPdf");
export const tplRegenerate = $("tplRegenerate");
export const basicInfoCard = $("basicInfoCard");
export const skillsTags = $("skillsTags");
export const btnImport = $("btnImport");
export const btnParse = $("btnParse");
export const btnExport = $("btnExport");
export const btnScore = $("btnScore");
export const btnClear = $("btnClear");
export const useBatchSwitch = $("useBatchSwitch");
export const aiLlmProvider = $("aiLlmProvider");
export const aiModelPath = $("aiModelPath");
export const aiLlamaCliPath = $("aiLlamaCliPath");
export const aiLlmApiKey = $("aiLlmApiKey");
export const aiThreads = $("aiThreads");
export const aiTemperature = $("aiTemperature");
export const aiCloudMaxOutputTokens = $("aiCloudMaxOutputTokens");
export const aiDisableThinking = $("aiDisableThinking");
export const aiSettingsPathHint = $("aiSettingsPathHint");
export const aiSettingsSave = $("aiSettingsSave");
export const aiSettingsReload = $("aiSettingsReload");
export const word2pdfInputDir = $("word2pdfInputDir");
export const word2pdfOutputDir = $("word2pdfOutputDir");
export const word2pdfBrowseIn = $("word2pdfBrowseIn");
export const word2pdfBrowseOut = $("word2pdfBrowseOut");
export const word2pdfStart = $("word2pdfStart");
export const word2pdfDefaults = $("word2pdfDefaults");
export const word2pdfPhase = $("word2pdfPhase");
export const word2pdfFraction = $("word2pdfFraction");
export const word2pdfBar = $("word2pdfBar");
export const word2pdfLog = $("word2pdfLog");
export const word2pdfSummary = $("word2pdfSummary");

// Quick action buttons (always visible in topbar/dashboard)
export const quickImport = $("quickImport");
export const quickNewJd = $("quickNewJd");
export const quickBatchParse = $("quickBatchParse");
export const quickExport = $("quickExport");
export const gotoParse = $("gotoParse");
export const gotoJd = $("gotoJd");
export const gotoTemplate = $("gotoTemplate");

// Stats
export const statTotal = $("statTotal");
export const statToday = $("statToday");
export const statPending = $("statPending");
export const statMatching = $("statMatching");
export const tokenSummaryContent = $("tokenSummaryContent");
export const tokenStatsTotal = $("tokenStatsTotal");
export const tokenStatsDaily = $("tokenStatsDaily");
export const tokenStatsModel = $("tokenStatsModel");
export const tokenStatsDays = $("tokenStatsDays");

// ── Global mutable state ──

export let importQueue = [];
export const importedPath = { value: null };
export const selectedQueueIndex = { value: -1 };
export const lastResumeObj = { value: null };
export let lastJdScore = null;
export let libraryRecords = [];
export let filteredLibraryRecords = [];
export let jdRecords = [];
export let appSettings = {
  llama_cli_path: "",
  model_path: "",
  threads: 4,
  temperature: 0.1,
  llm_provider: "ollama",
  llm_api_key: "",
  cloud_max_output_tokens: null,
};
export let lastJdRankingRows = [];
export const lastJdMatchCount = { value: 0 };
export let selectedLibraryIds = new Set();
export let selectedTemplateKeys = new Set();
export const resumeDetailBackTarget = { value: "library" };
export const importInProgress = { value: false };

// ── Page config ──

export const TITLE_MAP = {
  dashboard: "首页概览",
  library: "简历库",
  parse: "导入与解析",
  word2pdf: "Word 转 PDF",
  settings: "AI 配置（app-config.json）",
  jd: "JD 管理筛选",
  resumeDetail: "简历详情",
  jdCompareReport: "候选人对比报告",
  tokenStats: "Token 消耗",
};

export const PAGE_KEYS = new Set(Object.keys(TITLE_MAP));

// ── Shared helper functions ──

export function settings() {
  return appSettings;
}

export function pendingParseQueueCount() {
  return importQueue.filter((x) => !x.error && x.text && x.status !== "已完成").length;
}

export function updateStats() {
  if (!statTotal || !statToday || !statPending || !statMatching) return;
  statTotal.textContent = String(libraryRecords.length);
  statToday.textContent = String(countImportedToday(libraryRecords));
  statPending.textContent = String(pendingParseQueueCount());
  statMatching.textContent = String(lastJdMatchCount.value);
}
