export const app = {
  nav: {
    apps: "應用程式",
    models: "模型",
    dashboard: "儀表板",
    settings: "設定",
  },

  gateway: "閘道",

  today: "今日",
  requests: "次請求",
  tokens: "tokens",
  hubSynced: "Hub 剛剛同步",
  refreshScreen: "重新整理此畫面",
  refreshScreenTitle: "重新讀取此畫面顯示的資料 — 篩選條件、時間窗與捲動位置維持不變",

  usedPlanWindow: "方案窗口的 {pct}%",
  usedTokens: "{count}k tokens",
  usedRequests: "{count} 次請求",
  notifyPlanTitle: "Kiwano 方案上限",
  notifyCostTitle: "Kiwano 費用警示",
  notifyFeatureTitle: "Kiwano 警示",
  notifyPlanBody:
    "{provider} 的用量為 {used}，已達到 {limit}% 的上限 — 在用量回落至上限以下前將維持停用",
  notifyCostBody: "{provider} 本期已用 {used}，達到 {limit} 的上限 — 請留意支出",

  // Typed gateway events (pushed over /events; the tray entry is the click)
  notifyDlpTitle: "Kiwano 憑證監控",
  notifyLimitTitle: "Kiwano 已達限額",
  notifyAuthTitle: "Kiwano：Provider 金鑰無效",

  billing: {
    payg: "PAYG",
    plan: "方案",
    unl: "無限",
  },

  credentialBannerTitle: "最近的請求中有 API 金鑰或私密金鑰離開了本機。",
  credentialBannerCta: "查看該請求",
  credentialBannerDismiss: "關閉",
};
