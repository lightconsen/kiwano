export const addProvider = {
  // ── Dialog chrome ──
  titleAdd: "プロバイダーを追加",
  titleEdit: "プロバイダーを編集",

  // ── Mode switch ──
  fromModels: "カタログから",
  fromModelsNamed: "カタログから：{name}",
  custom: "カスタム",

  // ── Catalog picker ──
  searchCatalog: "カタログを検索…",
  noMatching: "一致するプロバイダーがありません",
  change: "変更",

  // ── Form fields ──
  name: "名前",
  protocol: "プロトコル",
  protoOpenai: "OpenAI 互換",
  protoAnthropic: "Anthropic",
  protoGemini: "Gemini",

  apiKey: "API キー",
  keyKeep: "空欄にすると現在のキーを保持します",
  keyLocal: "本機に保存され、読み取れるのはあなただけです",
  showHideKey: "キーの表示 / 非表示",

  endpointUrl: "エンドポイント URL",
  endpointHintShelf: "カタログから · プロトコルごとに1つ · API キーを共用",
  addEndpoint: "エンドポイントを追加",
  removeEndpoint: "このエンドポイントを削除",
  endpointHint: "プロトコルごとに1つ · API キーを共用",
  test: "テスト",

  // ── Inline probe verdicts (full detail lives in the tooltip) ──
  probeOk: "OK",
  probeAuth: "認証",
  probeUnsupported: "404",
  probeError: "エラー",
  probeUnreachable: "応答なし",

  // ── Default model ──
  defaultModel: "デフォルトモデル",
  fetch: "取得",
  fetching: "取得中…",
  errorNoKey: "先に API キーを入力してください — プロバイダーは匿名のモデル一覧を拒否します",
  errorNoModels: "エンドポイントがモデルを返しませんでした",
  errorTemplate:
    "先にエンドポイントのパラメータを入力してください — URL に未入力のプレースホルダーが残っています",
  modelIdPlaceholder: "モデル ID",

  // ── Billing ──
  billing: "課金",
  billingPlan: "プラン",
  billingPayg: "従量課金",
  billingUnlimited: "無制限",
  billingFixed: "このプロバイダーでは固定",
  billingFromCatalog: "カタログから",
  billingUnknown: "認識できない課金タグ「{billing}」 · モードを選択してください",
  billingBoth: "このベンダーは両方の課金方式があります — このプロバイダーがどちらかを選択してください",

  // ── Plan-mode percent limits ──
  usageLimits: "使用量の上限",
  usageLimitsHint: "任意 · 各プランウィンドウの割合（%）",
  fiveHourWindow: "5時間ウィンドウ",
  weeklyWindow: "週間ウィンドウ",
  planLimitsBody:
    "あるウィンドウのプランクォータ使用率がこの割合に達すると、Kiwano はこのプロバイダーへのルーティングを停止し、使用量が下回ると再開します。空欄にすると、そのウィンドウは無制限になります。",
  planLimitRange: "1〜100、空欄なら上限なし",

  // ── Payg spending limit ──
  spendingLimit: "支出上限",
  spendingLimitHint: "任意 · 空欄にすると使用量の推移を表示します",
  currencyTitle: "{currency} — このプロバイダーが請求する通貨",

  // ── Declared prices ──
  prices: "価格",
  pricesHint: "任意 · 100万トークンあたり、{currency} 建て",
  priceModel: "モデル",
  priceModelPlaceholder: "モデル ID",
  priceIn: "入力",
  priceOut: "出力",
  priceCacheRead: "キャッシュ読み取り",
  priceCacheWrite: "キャッシュ書き込み",
  addPrice: "モデルを追加",
  removePrice: "このモデルの価格を削除",
  pricesBody:
    "このプロバイダーの料金です。この数値はリクエストのコスト計算に使われ、支出上限もこれを基準に測られるため、Hub の価格より優先されます。記載しなかったモデルは Hub の表から価格が引かれ、引けなければ未価格として記録されます。キャッシュ料金を空欄にすると、その項目は課金されません。",
  priceInvalid: "すべての料金は 0 以上の数値にしてください",

  // ── Plan, but nothing that can be asked ──
  noQuotaEndpoint:
    "クォータ用エンドポイントなし · このベンダーは、あなたのキーだけではプラン使用量を読み取る方法を公開していません",

  // ── Unlimited ──
  noQuotaConfig: "クォータ設定なし · 一覧では使用量情報を表示しません",

  // ── Agent binding ──
  bindAgents: "保存後にエージェントへバインド",
  selectAgents: "エージェントを選択…",

  // ── Rotating keys (edit mode) ──
  rotatingKeys: "キーのローテーション",
  rotatingKeysHint: "プライマリキーとローテーション · レート制限を回避",
  deleteKey: "キーを削除",
  addKeyPlaceholder: "sk-… キーを追加",
  labelPlaceholder: "ラベル",

  // ── Advanced forwarding settings ──
  advanced: "詳細（タイムアウト / リトライ / ヘッダー）",
  timeoutLabel: "タイムアウト（秒）",
  retriesLabel: "リトライ回数",
  advancedBody:
    "空欄 = ゲートウェイの既定値 · タイムアウトは応答ヘッダーまでの時間に上限を設けるもので、進行中のストリームは打ち切りません · リトライはフェイルオーバー前にこのプロバイダーへ適用されます",
  customHeaders: "カスタムヘッダー",
  customHeadersHint: "最後にマージ · API キーのヘッダーを上書きできます",
  customHeadersAuthWarning:
    "{name} は、ゲートウェイがこのプロバイダーに注入する認証情報を上書きします。このエンドポイントがそれを要求する場合にのみ残してください。",
  headerNamePlaceholder: "Header-Name",
  headerValuePlaceholder: "値",
  removeHeader: "ヘッダーを削除",
  addHeader: "ヘッダーを追加",

  // ── Footer ──
  saving: "保存中…",
  saveEnable: "保存して有効化",

  // ── Plan-query templates ──
  tplKimi: "Kimi（月額プラン）",
  tplZhipuPersonal: "Zhipu GLM（個人向け）",
  tplZhipuTeam: "Zhipu GLM（チーム）",
  tplMinimax: "MiniMax",
  tplZenmux: "ZenMux",
  tplOpencodeGo: "OpenCode Go",
  tplVolcengine: "Volcengine Ark",
  fieldOrgId: "組織 ID",
  fieldProjectId: "プロジェクト ID",
  fieldQuotaUrl: "使用量エンドポイント URL",
  fieldAccessKeyId: "AccessKey ID",
  fieldSecretAccessKey: "Secret AccessKey",
};
