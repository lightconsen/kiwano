export const shelf = {
  chipAll: "全部",
  chipOfficial: "官方",
  chipAggregate: "聚合",
  chipThird: "第三方",
  chipFree: "免费档",
  /** `local` 分类徽章:筛选 chip 里没有它,所以单独给一个键。 */
  tagLocal: "本地",

  billingAny: "不限计费",
  billingPlan: "套餐",
  billingPayg: "按量付费",
  billingUnl: "无限量",

  probeOk: "正常",
  probeAuth: "需要密钥",
  probeUnsupported: "不支持",
  probeError: "错误",
  probeUnreachable: "无法连接",
  probeFailed: "探测失败",

  colName: "名称",
  colProtocol: "协议",
  colCategory: "分类",
  colPrice: "价格",
  colActions: "操作",

  test: "测试",
  endpoints: "端点",
  billing: "计费",
  added: "已添加",
  alreadyAdded: "已添加过",
  connect: "+ 连接",
  add: "+ 添加",

  fromHub: "来自 Kiwano Hub · {n} 个 Provider",
  searchPlaceholder: "搜索 Provider…",
  refreshAria: "从 Hub 刷新",
  refreshTitle: "从 Kiwano Hub 获取最新目录",
  noMatches: "没有匹配的 Provider",
  /** 与 noMatches 刻意分开:目录为空和「筛选没命中」是两回事,后者才是用户自己造成的。
   *  已无内置回落,所以从未连上 Hub 的机器看到的是这一句。 */
  notSynced: "尚未获取目录 —— 从 Kiwano Hub 获取",
};
