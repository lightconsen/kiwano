export const app = {
  nav: {
    apps: "アプリ",
    models: "モデル",
    dashboard: "ダッシュボード",
    settings: "設定",
  },

  // Header
  gateway: "ゲートウェイ",

  // Footer status bar
  today: "今日",
  requests: "リクエスト",
  tokens: "トークン",
  hubSynced: "Hub と同期しました",
  refreshScreen: "この画面を更新",
  refreshScreenTitle:
    "この画面に表示している内容を読み直します — フィルター、期間、スクロール位置はそのままです",

  // Cost-alert notification
  usedPlanWindow: "プラン期間の {pct}%",
  usedTokens: "{count}k トークン",
  usedRequests: "{count} リクエスト",
  notifyPlanTitle: "Kiwano プラン上限",
  notifyCostTitle: "Kiwano コスト警告",
  notifyFeatureTitle: "Kiwano アラート",
  notifyPlanBody:
    "{provider} は {used} となり、{limit}% の上限に達しました — 使用量が上限を下回るまで無効になります",
  notifyCostBody:
    "{provider} は今期 {used} を使用し、上限 {limit} に達しました — 支出にご注意ください",

  // Typed gateway events
  notifyDlpTitle: "Kiwano 認証情報の監視",
  notifyLimitTitle: "Kiwano 上限に達しました",
  notifyAuthTitle: "Kiwano: プロバイダーのキーが無効です",

  // Billing chips
  billing: {
    payg: "PAYG",
    plan: "プラン",
    unl: "無制限",
  },

  // Credential-watch banner
  credentialBannerTitle: "最近のリクエストで API キーまたは秘密キーが本機の外に送信されました。",
  credentialBannerCta: "リクエストを表示",
  credentialBannerDismiss: "閉じる",
};
