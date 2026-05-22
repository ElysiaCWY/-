//! 发往外部 LLM 前对姓名、电话、邮箱等做占位符脱敏，响应返回后再还原，降低隐私经模型侧泄露的风险。
//! 占位符仅存在于本进程内存中的映射表；不落盘、不单独加密传输（模型仍可见占位符串本身）。

/// 附加在发往模型的用户提示末尾：约束输出 JSON 中原样保留 `__RM_PRIV_NNNN__`。
pub const LLM_PLACEHOLDER_GUARD: &str = "\n【隐私】若上文含形如 __RM_PRIV_0000__ 的占位符（四位递增数字），你在返回的 JSON 字符串中必须 **原样** 保留这些占位符，不得改写为真实手机号、邮箱或姓名。\n";

use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, Default)]
pub struct PrivacyTokenMap {
  pairs: Vec<(String, String)>,
}

impl PrivacyTokenMap {
  pub fn is_empty(&self) -> bool {
    self.pairs.is_empty()
  }

  pub fn extend_map(&mut self, other: PrivacyTokenMap) {
    self.pairs.extend(other.pairs);
  }
}

#[derive(Clone)]
struct Span {
  start: usize,
  end: usize,
  text: String,
}

fn re_email() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?i)\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b").expect("email re")
  })
}

fn re_mobile_cn() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:\+?86[\s\-]?)?1[3-9]\d(?:[\s\-]?\d{4}){2}").expect("mobile re")
  })
}

fn re_landline() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"\b0\d{2,3}[\s\-]?\d{7,8}\b").expect("landline re"))
}

fn re_labeled_phone() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:电话|手机|Tel|Mobile|联系方式)\s*[:：]\s*([\d+\s\-–—]{8,22})").expect("labeled phone")
  })
}

fn re_qq() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"(?i)\bQQ\s*[:：]?\s*(\d{5,12})\b").expect("qq re"))
}

fn re_wechat() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| Regex::new(r"微信\s*[:：]\s*([^\s\n，,。；;、]{1,32})").expect("wechat re"))
}

fn re_name_labeled() -> &'static Regex {
  static RE: OnceLock<Regex> = OnceLock::new();
  RE.get_or_init(|| {
    Regex::new(r"(?:姓名|名字|本名|应聘人|求职者|申请人|候选人|Name|Candidate)\s*[:：]\s*([一-龥A-Za-z·•．.\s]{2,20})").expect("name labeled")
  })
}

// ── P0: 首行推断 + 姓氏字典 ────────────────────────────────────────

/// 常见中文姓氏（单姓 957 + 复姓 64，源自百家姓及扩展）
fn common_surnames() -> &'static [&'static str] {
  &[
    "丁", "万", "不", "丑", "世", "丘", "丙", "业", "丛", "东",
    "严", "中", "丰", "丹", "么", "义", "之", "乌", "乐", "乔",
    "乘", "乙", "乜", "九", "习", "书", "买", "乾", "于", "云",
    "亓", "五", "井", "亢", "亥", "京", "仁", "仆", "仇", "仉",
    "介", "仍", "从", "仙", "仝", "代", "令", "以", "仪", "仰",
    "仲", "仵", "任", "伊", "伍", "伏", "休", "伟", "伦", "伯",
    "似", "但", "位", "何", "佘", "余", "佛", "佟", "佴", "佼",
    "侍", "依", "侨", "侯", "俎", "保", "俞", "俟", "信", "修",
    "候", "倪", "偶", "傅", "储", "僧", "僪", "允", "元", "充",
    "兆", "光", "党", "全", "公", "六", "兰", "关", "兴", "其",
    "典", "养", "冀", "冉", "冒", "军", "农", "冯", "况", "冷",
    "冼", "凌", "凤", "凭", "出", "函", "刀", "刁", "刑", "刘",
    "刚", "初", "利", "别", "前", "剑", "剧", "力", "功", "务",
    "励", "劳", "势", "勇", "勤", "勾", "包", "化", "北", "匡",
    "区", "千", "华", "卑", "卓", "单", "南", "卜", "卞", "占",
    "卢", "卫", "卯", "印", "危", "却", "卷", "卿", "厉", "厍",
    "厚", "原", "及", "友", "双", "叔", "受", "古", "召", "可",
    "台", "史", "叶", "司", "合", "吉", "同", "后", "向", "吕",
    "吴", "吾", "告", "员", "周", "呼", "和", "咎", "咸", "哀",
    "哈", "唐", "唱", "商", "善", "喜", "喻", "嘉", "回", "国",
    "圣", "在", "圭", "坚", "城", "堂", "堵", "塔", "塞", "墨",
    "士", "壬", "声", "夏", "夔", "夕", "夙", "多", "大", "天",
    "夫", "夷", "奇", "奈", "奉", "奕", "奚", "妫", "始", "姒",
    "姓", "委", "姚", "姜", "姬", "威", "娄", "嬴", "孔", "字",
    "孙", "孛", "孝", "孟", "季", "学", "宁", "宇", "守", "安",
    "宋", "完", "宏", "宓", "宗", "官", "定", "宛", "宜", "宝",
    "实", "宣", "宦", "宫", "宰", "家", "容", "宿", "寇", "富",
    "寒", "寸", "寻", "寿", "封", "将", "尉", "少", "尔", "尚",
    "尤", "尧", "尹", "尾", "局", "居", "屈", "展", "屠", "山",
    "岑", "岳", "崇", "崔", "嵇", "巢", "左", "巧", "巨", "巩",
    "巫", "己", "巴", "市", "布", "帅", "师", "希", "帖", "帛",
    "席", "常", "干", "平", "年", "幸", "广", "庄", "庆", "库",
    "应", "庚", "府", "庞", "度", "康", "庹", "庾", "廉", "廖",
    "延", "建", "开", "弓", "弘", "张", "弥", "弭", "强", "归",
    "彤", "彭", "律", "後", "徐", "御", "徭", "德", "念", "忻",
    "怀", "性", "恭", "恽", "悉", "悟", "惠", "愈", "愚", "慈",
    "慎", "慕", "戈", "戊", "戎", "戏", "成", "战", "戚", "戢",
    "戴", "户", "房", "所", "扈", "才", "扬", "扶", "承", "抄",
    "抗", "折", "招", "拜", "拱", "捷", "掌", "接", "揭", "摩",
    "撒", "操", "支", "改", "敏", "敖", "敛", "敬", "文", "斋",
    "斐", "斛", "斯", "方", "於", "施", "旁", "旅", "旗", "无",
    "时", "旷", "昂", "昌", "明", "易", "昔", "昝", "星", "春",
    "是", "晁", "晋", "晏", "普", "景", "智", "暨", "暴", "曲",
    "曹", "曾", "有", "朋", "望", "本", "朱", "朴", "机", "权",
    "李", "杜", "杞", "束", "来", "杨", "杭", "松", "板", "析",
    "林", "枚", "枝", "柏", "柔", "查", "柯", "柳", "柴", "栋",
    "树", "栗", "校", "栾", "桂", "桐", "桑", "桓", "桥", "梁",
    "梅", "检", "森", "植", "楚", "楼", "樊", "檀", "欎", "次",
    "欧", "止", "步", "武", "歧", "殳", "殴", "段", "殷", "毋",
    "母", "毓", "毕", "毛", "水", "永", "求", "汉", "汗", "汝",
    "江", "池", "汤", "汪", "汲", "沃", "沈", "沐", "沙", "泉",
    "法", "波", "泣", "泥", "泰", "泷", "洋", "洛", "洪", "浑",
    "浦", "浮", "海", "涂", "淡", "淦", "清", "渠", "温", "游",
    "湛", "源", "滑", "滕", "满", "漆", "漫", "潘", "潜", "潭",
    "潮", "澄", "濮", "濯", "烟", "焉", "焦", "熊", "燕", "爱",
    "牛", "牟", "牢", "牧", "牵", "犁", "犹", "狂", "狄", "独",
    "玄", "玉", "王", "环", "班", "理", "琦", "琴", "瑞", "璩",
    "瓮", "甄", "甘", "生", "用", "甫", "田", "由", "甲", "申",
    "畅", "留", "疏", "登", "白", "百", "皇", "皋", "皮", "盈",
    "益", "盍", "盖", "盘", "盛", "相", "真", "眭", "睢", "督",
    "睦", "瞿", "矫", "石", "硕", "碧", "磨", "示", "礼", "祁",
    "祈", "祖", "祝", "祢", "祭", "禄", "福", "禚", "禹", "禽",
    "禾", "秋", "种", "秘", "秦", "称", "程", "税", "稽", "穆",
    "穰", "空", "窦", "章", "童", "竭", "端", "竹", "竺", "笃",
    "符", "笪", "第", "答", "简", "箕", "管", "籍", "米", "类",
    "粘", "粟", "糜", "系", "素", "索", "紫", "綦", "繁", "红",
    "纪", "纳", "纵", "线", "练", "终", "绍", "经", "绪", "续",
    "绳", "缑", "缪", "罕", "罗", "羊", "羽", "羿", "翁", "翟",
    "翠", "翦", "老", "考", "耿", "聂", "聊", "肇", "肥", "胡",
    "胥", "能", "脱", "腾", "臧", "舄", "舒", "舜", "良", "艾",
    "节", "芒", "芮", "花", "苌", "苍", "苏", "苑", "苗", "苟",
    "苦", "英", "茂", "范", "茅", "茆", "茹", "荀", "荆", "荣",
    "荤", "莘", "莫", "莱", "菅", "营", "萧", "萨", "葛", "董",
    "蒉", "蒋", "蒙", "蒯", "蒲", "蒿", "蓝", "蓟", "蓬", "蔚",
    "蔡", "蔺", "薄", "薛", "藏", "藤", "藩", "虎", "虞", "虢",
    "蚁", "蛮", "融", "衅", "行", "衡", "衣", "表", "衷", "袁",
    "袭", "裔", "裘", "裴", "褒", "褚", "覃", "解", "言", "訾",
    "詹", "謇", "计", "让", "许", "诗", "说", "诸", "诺", "谈",
    "谌", "谏", "谢", "谬", "谭", "谯", "谷", "豆", "象", "貊",
    "贝", "贡", "贯", "贰", "贲", "贵", "贸", "费", "贺", "贾",
    "资", "赏", "赖", "赛", "赤", "赧", "赫", "赵", "越", "路",
    "蹇", "蹉", "车", "载", "辉", "辛", "辜", "辟", "边", "达",
    "过", "运", "进", "连", "迟", "迮", "逄", "通", "速", "逢",
    "逮", "逯", "遇", "道", "邓", "邗", "邛", "邝", "邢", "那",
    "邬", "邰", "邱", "邴", "邵", "邶", "邸", "邹", "郁", "郎",
    "郏", "郑", "郗", "郜", "郝", "郦", "郭", "郯", "郸", "都",
    "鄂", "鄞", "鄢", "酆", "酒", "释", "野", "金", "针", "钊",
    "钞", "钟", "钦", "钭", "钮", "钱", "铁", "铎", "银", "锁",
    "锐", "错", "镇", "镜", "长", "门", "闪", "闫", "闭", "问",
    "闳", "闵", "闻", "闽", "闾", "阎", "阙", "阚", "阮", "阳",
    "阴", "阿", "陀", "陆", "陈", "陶", "隆", "隋", "随", "隐",
    "隗", "隽", "雀", "集", "雍", "雪", "零", "雷", "霍", "霜",
    "青", "靖", "革", "靳", "鞠", "韦", "韩", "韶", "项", "须",
    "顾", "顿", "频", "颜", "风", "飞", "饶", "首", "马", "驹",
    "骆", "骑", "高", "魏", "鱼", "鲁", "鲍", "鲜", "鹿", "麦",
    "麴", "麻", "黄", "黎", "齐", "龙", "龚",
    "万俟", "上官", "东方", "东郭", "东门", "乐正",
    "亓官", "令狐", "仲孙", "公冶", "公孙", "公羊",
    "公良", "公西", "凃肖", "单于", "南宫", "南门",
    "司寇", "司徒", "司空", "司马", "呼延", "壤驷",
    "夏侯", "太叔", "夹谷", "子车", "宇文", "宗政",
    "宰父", "尉迟", "左丘", "巫马", "微生", "慕容",
    "拓跋", "梁丘", "欧阳", "段干", "淳于", "漆雕",
    "澹台", "濮阳", "申屠", "百里", "皇甫", "端木",
    "第五", "羊舌", "荔菲", "西门", "诸葛", "谷梁",
    "赫连", "轩辕", "辗迟", "钟离", "锺离", "长孙",
    "闻人", "闾丘", "颛孙", "鲜于",
  ]
}

fn is_common_surname(c: char) -> bool {
  common_surnames().iter().any(|s| s.chars().next() == Some(c))
}

/// 从简历首部区域检测无标签姓名（跳过 "个人简历" 等标题后的前几行）。
fn collect_names_from_header(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();
  let lines: Vec<&str> = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
  if lines.is_empty() {
    return spans;
  }

  let header_words = [
    "个人简历", "简历", "个人简介", "求职简历", "应聘简历",
    "resume", "cv", "curriculum vitae",
    "中文简历", "英文简历",
  ];
  let is_header = |s: &str| {
    let t = s.trim().to_ascii_lowercase();
    header_words.iter().any(|p| t == p.to_ascii_lowercase() || t.starts_with(&p.to_ascii_lowercase()))
  };

  // 跳过标题行，取前 6 行候选
  let candidates: Vec<&&str> = lines.iter().filter(|l| !is_header(l)).take(6).collect();

  for line in candidates {
    let t = line.trim();
    let char_count = t.chars().count();

    if char_count >= 2 && char_count <= 4 {
      let all_cjk = t.chars().all(|c| c >= '\u{4E00}' && c <= '\u{9FFF}' || c >= '\u{3400}' && c <= '\u{4DBF}');
      if all_cjk && is_common_surname(t.chars().next().unwrap()) {
        if let Some(pos) = text.find(t) {
          spans.push(Span { start: pos, end: pos + t.len(), text: t.to_string() });
        }
      }
    } else if char_count >= 3 && char_count <= 20 {
      let has_upper = t.chars().any(|c| c.is_ascii_uppercase());
      let has_bad = t.chars().any(|c| c == '@' || c.is_ascii_digit());
      let alpha_cnt = t.chars().filter(|c| c.is_ascii_alphabetic() || *c == ' ' || *c == '.' || *c == '-').count();
      if has_upper && !has_bad && (alpha_cnt as f64 / char_count.max(1) as f64) > 0.8 {
        if let Some(pos) = text.find(t) {
          spans.push(Span { start: pos, end: pos + t.len(), text: t.to_string() });
        }
      }
    }
  }
  spans
}

/// 全局扫描：姓氏 + 1~2 个 CJK 字符，受边界约束，避免匹配普通词汇。
fn collect_names_by_surname(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();
  let chars: Vec<(usize, char)> = text.char_indices().collect();
  let len = chars.len();
  if len < 2 {
    return spans;
  }

  let is_boundary = |idx: usize| -> bool {
    if idx >= len { return true; }
    let c = chars[idx].1;
    c.is_whitespace() || c.is_ascii_punctuation() || c.is_ascii_alphanumeric()
      || matches!(c, '，' | '。' | '、' | '：' | '；' | '（' | '）' | '【' | '】' | '《' | '》' | '/' | '｜' | '—' | '|')
  };

  let is_cjk = |c: char| -> bool {
    (c >= '\u{4E00}' && c <= '\u{9FFF}') || (c >= '\u{3400}' && c <= '\u{4DBF}')
  };

  for i in 0..len {
    let c = chars[i].1;
    if !is_cjk(c) || !is_common_surname(c) {
      continue;
    }
    if i > 0 && !is_boundary(i - 1) {
      continue;
    }
    let mut j = i + 1;
    while j < len && j < i + 4 && is_cjk(chars[j].1) {
      j += 1;
    }
    let name_len = j - i;
    if name_len < 2 || name_len > 3 {
      continue;
    }
    if j < len && !is_boundary(j) {
      continue;
    }

    let start = chars[i].0;
    let end = if j < len { chars[j].0 } else { text.len() };
    spans.push(Span { start, end, text: text[start..end].to_string() });
  }
  spans
}

// ── 收集所有 Span ────────────────────────────────────────────────────

fn collect_spans(text: &str) -> Vec<Span> {
  let mut spans: Vec<Span> = Vec::new();

  for m in re_email().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for m in re_mobile_cn().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for m in re_landline().find_iter(text) {
    spans.push(Span {
      start: m.start(),
      end: m.end(),
      text: m.as_str().to_string(),
    });
  }
  for cap in re_labeled_phone().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_qq().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_wechat().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      spans.push(Span {
        start: m.start(),
        end: m.end(),
        text: m.as_str().to_string(),
      });
    }
  }
  for cap in re_name_labeled().captures_iter(text) {
    if let Some(m) = cap.get(1) {
      let name = m.as_str().trim();
      if name.len() >= 2 && name.chars().count() <= 8 {
        spans.push(Span {
          start: m.start(),
          end: m.end(),
          text: name.to_string(),
        });
      }
    }
  }

  // P0: 首行推断 + 姓氏全局扫描
  spans.extend(collect_names_from_header(text));
  spans.extend(collect_names_by_surname(text));

  spans
}

fn pick_non_overlapping(mut spans: Vec<Span>) -> Vec<Span> {
  spans.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| a.start.cmp(&b.start)));
  let mut picked: Vec<Span> = Vec::new();
  'outer: for sp in spans {
    for p in &picked {
      if !(sp.end <= p.start || sp.start >= p.end) {
        continue 'outer;
      }
    }
    picked.push(sp);
  }
  picked.sort_by_key(|s| s.start);
  picked
}

/// 将 `text` 中检测到的敏感片段替换为 `__RM_PRIV_NNNN__`，并记录还原映射。`next_id` 在多次拼接脱敏时共用，避免占位符重复。
pub fn mask_sensitive_segments(text: &str, next_id: &mut u32) -> (String, PrivacyTokenMap) {
  if text.is_empty() {
    return (String::new(), PrivacyTokenMap::default());
  }
  let spans = pick_non_overlapping(collect_spans(text));
  if spans.is_empty() {
    return (text.to_string(), PrivacyTokenMap::default());
  }

  let mut pairs = Vec::new();
  let mut out = String::with_capacity(text.len() + spans.len() * 20);
  let mut last = 0usize;
  for sp in spans {
    if sp.start > last {
      out.push_str(&text[last..sp.start]);
    }
    let ph = format!("__RM_PRIV_{:04}__", *next_id);
    *next_id += 1;
    pairs.push((ph.clone(), sp.text.clone()));
    out.push_str(&ph);
    last = sp.end;
  }
  if last < text.len() {
    out.push_str(&text[last..]);
  }
  (out, PrivacyTokenMap { pairs })
}

/// 单段文本脱敏（独立计数从 0 开始）。
pub fn mask_sensitive_segments_single(text: &str) -> (String, PrivacyTokenMap) {
  let mut n = 0u32;
  mask_sensitive_segments(text, &mut n)
}

/// 将模型返回 JSON 等字符串中的占位符还原为原文。
pub fn unmask_sensitive_segments(s: &str, map: &PrivacyTokenMap) -> String {
  if map.pairs.is_empty() {
    return s.to_string();
  }
  let mut out = s.to_string();
  for (ph, orig) in map.pairs.iter().rev() {
    out = out.replace(ph.as_str(), orig);
  }
  out
}

/// 从原始文本中直接提取姓名（用于 LLM 解析后姓名为空时的后备方案）。
/// 复用 `collect_names_from_header`（首行推断）和 `collect_names_by_surname`（姓氏扫描）。
pub fn extract_name_from_text(text: &str) -> Option<String> {
  if text.is_empty() {
    return None;
  }
  // 只取姓名相关 span，不混入邮箱/电话等
  let mut spans = collect_names_from_header(text);
  spans.extend(collect_names_by_surname(text));

  // 按长度降序取非重叠项
  spans.sort_by(|a, b| b.text.len().cmp(&a.text.len()).then_with(|| a.start.cmp(&b.start)));
  let mut picked: Vec<Span> = Vec::new();
  for sp in spans {
    let overlapping = picked.iter().any(|p| !(sp.end <= p.start || sp.start >= p.end));
    if !overlapping {
      picked.push(sp);
    }
  }
  picked.sort_by_key(|s| s.start);

  // 优先返回中文姓名
  for sp in &picked {
    let t = sp.text.trim();
    let chars: Vec<char> = t.chars().collect();
    if chars.len() >= 2 && chars.len() <= 4
      && chars.iter().any(|c| *c >= '\u{4E00}' && *c <= '\u{9FFF}')
      && is_common_surname(chars[0])
    {
      return Some(t.to_string());
    }
  }
  // 回退：英文名
  for sp in &picked {
    let t = sp.text.trim();
    if t.chars().count() >= 2
      && t.chars().any(|c| c.is_ascii_uppercase())
      && t.chars().all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '.' || c == '-')
    {
      return Some(t.to_string());
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn roundtrip_email_and_name() {
    let raw = "姓名：王小明\n手机：13812345678\n邮箱 a@b.com 结尾";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(masked.contains("__RM_PRIV_"));
    assert!(!masked.contains("13812345678"));
    assert!(!masked.contains("a@b.com"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn header_name_inference() {
    let raw = "王小明\n男 | 28岁 | 本科\nJava 开发工程师\n\n工作经历：...";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(masked.contains("__RM_PRIV_"));
    assert!(!masked.contains("王小明"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn header_name_with_title_skip() {
    let raw = "个人简历\n\n李四\n男 | 25岁\n\n教育经历：...";
    let (masked, map) = mask_sensitive_segments_single(raw);
    assert!(!masked.contains("李四"));
    let back = unmask_sensitive_segments(&masked, &map);
    assert_eq!(back, raw);
  }

  #[test]
  fn surname_based_name_detection() {
    let raw = "候选人 张三 的简历，曾就职于阿里巴巴。";
    let (masked, _map) = mask_sensitive_segments_single(raw);
    // "张三" 应被脱敏（姓氏 "张" 在字典中）
    assert!(!masked.contains("张三"));
  }

  #[test]
  fn no_false_positive_on_common_word() {
    // "黄金" — "黄" 在姓氏表中但 "黄金" 是普通词汇，应在边界约束下不被误匹配
    let raw = "黄金时代的技术栈以 Java 为主。";
    let (masked, _map) = mask_sensitive_segments_single(raw);
    // "黄金" 应该还在（因为 "金" 后面跟 "时"，不在边界位置，不应匹配）
    assert!(masked.contains("黄金"));
  }

  #[test]
  fn extract_name_from_header() {
    assert_eq!(extract_name_from_text("王小明\n男 | 28岁 | 本科"), Some("王小明".into()));
  }

  #[test]
  fn extract_name_from_labeled() {
    assert_eq!(extract_name_from_text("姓名：赵六\n手机：13800000000"), Some("赵六".into()));
  }

  #[test]
  fn extract_name_fallback_empty() {
    assert_eq!(extract_name_from_text("仅测试没有姓名的文本内容"), None);
  }
}
