export const dashboard = {
  windowToday: "今日",
  window7d: "過去7日間",
  window30d: "過去30日間",
  windowAll: "すべて",

  allProviders: "すべてのプロバイダー",
  allAgents: "すべてのエージェント",
  filterByProvider: "プロバイダーで絞り込む",
  filterByAgent: "エージェントで絞り込む",
  localSqlite: "データはローカルの SQLite に保存されます",

  statRequests: "リクエスト",
  statTokens: "トークン",
  statEstCost: "推定コスト",
  statAvgLatency: "平均レイテンシ",
  atHubPrice: "Hub 価格で",
  offPeakPremium: "ピーク時割増",
  offPeakPremiumTitle:
    "このウィンドウのトークンを、それぞれの行のオフピーク料金で計算した場合の金額です。使ったモデルに時間帯別料金がなければ 0、すでにオフピークだった場合も 0 になります。",
  tokensInOut: "入力 {input} / 出力 {output}",
  tokensTitle: "入力 {input}（キャッシュヒット {cache}、1/10 で課金）· 出力 {output}",

  usageTrend: "使用量の推移",
  metricRequests: "リクエスト",
  metricTokens: "トークン",
  trendMetricRequests: "リクエスト",
  trendMetricTokens: "トークン",
  trendAria: "使用量の推移 · {metric} · {n} 個のバケット",

  byProvider: "プロバイダー別",
  donutAria: "プロバイダー別 · {n} リクエスト",
  donutCenterLabel: "リクエスト",
  notRoutingHere: "ここにはルーティングされていません：{reason}",
  blocked: "ブロック中",
  noTraffic: "この期間にトラフィックはありません。",

  byAgent: "エージェント別",
  colAgent: "エージェント",
  colRequests: "リクエスト",
  colTokens: "トークン",
  colCost: "コスト",
};
