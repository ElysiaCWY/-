// resume_data.js
const resumeData = {
  // ==================== 1. 基础信息 ====================
  basicInfo: {
    name: "", // 姓名
    age: "", // 年龄
    gender: "", // 性别
    education: [
      // 教育背景列表，如有多个请复制下方对象继续添加
      {
        school: "", // 学校名称
        major: "", // 专业
        degree: "", // 学历（如：本科、硕士）
        period: "" // 就读时间（如：2016.09 - 2020.06）
      }
    ],
    skills: [], // 技能列表（如：["Python", "Java"]）
    certificates: [] // 证书列表（如：["英语六级", "PMP"]）
  },

  // ==================== 2. 工作经历 ====================
  // 规则：按时间倒序排列，"1" 代表最新/最近一段工作经历
  // 新增经历请顺延序号（"2", "3"...）
  workExperience: {
    "1": {
      company: "", // 公司名称
      position: "", // 职位
      period: "", // 工作时间（如：2022.03 - 至今）
      description: "" // 工作内容描述
    }
    // 示例：添加第二段经历
    // "2": {
    //   company: "",
    //   position: "",
    //   period: "",
    //   description: ""
    // }
  },

  // ==================== 3. 项目经历 ====================
  // 规则：按时间倒序或重要性排列，"1" 代表最新/最重要的项目
  // 新增项目请顺延序号（"2", "3"...）
  projectExperience: {
    "1": {
      projectName: "", // 项目名称
      projectDescription: "", // 项目描述（背景、职责等）
      projectAchievements: "" // 项目成果/业绩
    }
    // 示例：添加第二个项目
    // "2": {
    //   projectName: "",
    //   projectDescription: "",
    //   projectAchievements: ""
    // }
  }
};

// 导出数据（Node.js 环境使用）
module.exports = resumeData;