export const dashboard = {
  windowToday: "今天",
  window7d: "最近 7 天",
  window30d: "最近 30 天",
  windowAll: "全部",

  allProviders: "全部 Provider",
  allAgents: "全部 Agent",
  filterByProvider: "依 Provider 篩選",
  filterByAgent: "依 Agent 篩選",
  localSqlite: "資料保存在本機 SQLite",

  statRequests: "請求數",
  statTokens: "token",
  statEstCost: "預估費用",
  statAvgLatency: "平均延遲",
  atHubPrice: "按 Hub 價格",
  offPeakPremium: "尖峰溢價",
  offPeakPremiumTitle:
    "這個窗口的 token 若依各列的離峰價計費會是多少。用到的模型沒有尖離峰價時為 0；本來就在離峰時段也為 0。",
  tokensInOut: "輸入 {input} / 輸出 {output}",
  tokensTitle: "輸入 {input}（快取命中 {cache}，按 1/10 計費）· 輸出 {output}",

  usageTrend: "用量趨勢",
  metricRequests: "請求",
  metricTokens: "token",
  trendMetricRequests: "請求",
  trendMetricTokens: "token",
  trendAria: "用量趨勢 · {metric} · {n} 個時段",

  byProvider: "依 Provider",
  donutAria: "依 Provider · {n} 個請求",
  donutCenterLabel: "請求",
  notRoutingHere: "未路由至此：{reason}",
  blocked: "已封鎖",
  noTraffic: "此時間窗內沒有流量。",

  byAgent: "依 Agent",
  colAgent: "Agent",
  colRequests: "請求",
  colTokens: "token",
  colCost: "費用",
};
