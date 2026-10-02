export const logs = {
  title: "請求日誌",

  filterAll: "全部",
  filterOk: "成功",
  filterError: "錯誤",

  rangeAny: "不限時間",
  rangeToday: "今天",
  range7d: "最近 7 天",
  range30d: "最近 30 天",
  rangeCustom: "自訂…",
  rangeFallback: "全部時間",

  requestCount: "{n} 個請求",
  exportCsv: "匯出 CSV",
  exportTitle: "選擇日期範圍並寫入 CSV 檔",

  colTime: "時間",
  colAgent: "Agent",
  colProvider: "Provider",
  colModel: "模型",
  colStatus: "狀態",
  colLatency: "延遲",
  firstToken: "首字延遲",
  colThroughput: "tok/s",
  colTokens: "token",

  empty: "尚無請求記錄 — 經閘道轉送的流量會顯示在這裡",

  prev: "上一頁",
  next: "下一頁",
  pageIndicator: "{page} / {pages}",

  logTitle: "日誌 #{id}",

  detailSession: "工作階段",
  detailNotes: "閘道記錄：",
  dlpBadge: "憑證",
  detailCredentialWatch: "憑證監控：此請求中有 API 金鑰或私密金鑰離開了本機。",
  detailRequest: "請求",
  detailResponse: "回應",
  detailBodyTruncated: "（內文已截斷）",
  detailRequestHeaders: "請求標頭",
  detailResponseHeaders: "回應標頭",
  detailRequestBody: "請求內文",
  detailResponseBody: "回應內文",
  detailClientStream: "（用戶端可見的串流）",

  exportDialogTitle: "匯出日誌",
  dateRange: "日期範圍",
  exportNote: "範圍內的每個請求都會寫入，不只是畫面上的 {n} 筆。",
  exporting: "匯出中…",
  exported: "已匯出 {n} 列",
  exportCapped: "已截斷至 {n} 列 — 請縮小範圍以匯出其餘部分。",
};
