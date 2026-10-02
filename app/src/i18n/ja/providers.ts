export const providers = {
  // ── Segment bar and list header ──
  all: "すべて",
  counts: "{providers} プロバイダー · {agents} エージェントを紐付け済み",
  addProvider: "プロバイダーを追加",
  refreshTitle:
    "プロバイダー、プランのクォータ、インストール済みエージェントを更新します（5分間のクォータキャッシュを迂回）",

  // ── Per-agent onboarding (empty tab) ──
  takenOverTitle: "{agent} は引き継ぎ済み",
  takenOverBody:
    "ローカルのゲートウェイがこのエージェントのリクエストをルーティングしていますが、まだプロバイダーが紐付けられていません。リクエストの行き先を用意するため、プロバイダーを追加してください。",
  startManaging: "Kiwano で {agent} の管理を始める",
  startManagingBody:
    "Kiwano は現在の設定をバックアップし（あとからワンクリックで復元できます）、{agent} がすでに使っているプロバイダーを取り込み（他のエージェントと共有されます）、ローカルゲートウェイ経由でルーティングします。上流はそのままで、以後は即座に切り替えられます。",
  enableKiwano: "Kiwano を有効にする",
  enabling: "有効化中…",
  addProviderFirst: "先にプロバイダーを追加",
  oauthNote:
    "公式サブスクリプションでサインインしていますか（Claude ログイン、Codex ChatGPT）？公式の OAuth はまだプロキシできません。先にプロバイダーを手動で追加してください。",

  // ── Provider rows ──
  unbound: "未紐付け",
  inUse: "使用中",
  quotaFallback: "フォールバック",
  quotaFallbackTitle:
    "プライマリがクォータを超えたら最初に引き継ぎます。実際にどのフォールバックが処理するかは、ほかのプロバイダーがまだ到達可能かどうかで決まります",
  editProvider: "プロバイダーを編集",
  deleteProvider: "プロバイダーを削除",
  clickAgain: "もう一度クリックで確定",
  confirm: "確定",
  blocked: "ブロック中",
  notRoutingTitle: "ここへはルーティングしていません：{reason}",

  // ── The Status column's measurements ──
  latencyTraffic: "直近24時間における、このプロバイダー自身のリクエストの平均レイテンシ",
  latencyProbe:
    "到達性：ゲートウェイがエンドポイントに問い合わせた結果です。署名なしのリクエストなので、そこに何かが応答していることだけを示し、あなたのキーが機能するかは示しません · 確認 {time}",
  latencyTest:
    "あなたがテストしました — このプロバイダーのキーで実際のプロンプトを送信しました · 計測 {time}",
  unreachable: "応答なし",
  unreachableTitle: "最後に問い合わせたときにこのエンドポイントは応答しませんでした · 確認 {time}",
  refused: "キー拒否",
  refusedTitle: "エンドポイントは応答しましたが、このキーを拒否しました：{error} · 確認 {time}",

  // ── Usage / quota cell ──
  usagePlanLimits: "プラン · 上限 {windows} · このプロバイダーのプランクォータ使用率で判定{tokens}",
  usageWindowFive: "5時間枠 ≤ {percent}%",
  usageWindowWeekly: "週次枠 ≤ {percent}%",
  usageInOut: " · 入力 {input} · 出力 {output}",
  usagePlanQuota:
    "プラン · 今期 {used}/{limit} {unit}（{percent}%）· リセット {resets} · 入力 {input} · 出力 {output}",
  usageLocalInference: "ローカル推論 · コスト計測なし · オフラインで動作",
  usagePaygQuota:
    "従量課金 · 今期 {used} / {limit}（{percent}%）· 入力 {input}（キャッシュ {cache}、1/10 で課金）· 出力 {output} · レイテンシ {latency}",
  usagePaygTrend: "従量課金 · 上限未設定 · 直近7日間の使用量推移",
  planTemplateCached: "{template} · キャッシュ済み",
  planLimitFive: "5h ≤ {percent}%",
  planLimitWeekly: "週 ≤ {percent}%",
  unitReq: "件",
  unitTenKTok: "1万 tok",
  reqTokSuffix: "件 · {tokens} tok",
  inOutTokens: "入力 {input} · 出力 {output}",
  limitSuffix: "/ 上限 {limit}",
  tokensLatency: "{tokens} トークン · レイテンシ {latency}",
  reqSuffix: "· {requests} 件",

  // ── Column headers ──
  colProvider: "プロバイダー",
  colRole: "ストラテジーでの役割",
  colBoundAgents: "紐付け済みエージェント",
  colUsage: "使用量 / クォータ",
  colCache: "キャッシュ",
  cacheColTitle:
    "直近7日間のプロンプトキャッシュヒット率：キャッシュ読み取り ÷ 入力側トークン（新規入力 + キャッシュ読み取り + キャッシュ書き込み）",
  cacheHitTitle: "直近7日間で入力側トークンの {pct}% がキャッシュから供給されました（{read} / {denom}）",
  cacheNoData: "直近7日間に入力側トークンがないため、ヒット率を算出できません",
  colStatus: "ステータス",
  colPriority: "優先度",
  colActions: "操作",

  // ── Agent-tab role cell ──
  weight: "重み",
  weightTitle: "セッションは重みの比率に応じて候補間を回ります",
  windowStart: "時間帯の開始",
  windowEnd: "時間帯の終了",
  windowTitle:
    "この候補が処理するローカル時間帯です。0930 のように数字で入力します — 終了が開始より早いときは日付をまたぎます。両方をクリアすると時間帯を解除します",
  fallback: "フォールバック",
  fallbackTitle: "どの候補の時間帯にも一致しないときに処理します",
  noWindow: "時間帯なし",
  noWindowTitle: "時間帯が未設定 — この候補は選ばれません",
  primary: "プライマリ",
  primaryTitle: "このエージェントの現在のルート",
  standby: "スタンバイ #{index}",

  // ── Candidate row actions ──
  moveUp: "上に移動",
  moveDown: "下に移動",
  makePrimary: "プライマリにする",
  makePrimaryTitle: "プライマリにする — この候補をキューの先頭に移動します",
  // ── Deleting a provider, and parking one instead ──
  delRemovedFrom: "{agents} から削除されます",
  delPromotes: "{provider} が {agent} のプライマリになります",
  delEmpties: "{agent} にプロバイダーが残りません",
  disable: "無効化",
  enable: "有効化",
  disableTitle: "待機させます。すべてのルートから外れますが、行とキーは残ります",
  enableTitle: "紐付け先のルートに戻します",

  // ── Latency test (a prompt round trip, one per provider row) ──
  testLatency: "レイテンシをテスト",
  testLatencyTitle: "プロンプトを1回送って往復時間を計測します",
  testLatencyFailed: "失敗",

  // ── User-defined agents (migration v16) ──
  addAgentMenu: "エージェントを追加",
  addAgentMenuTitle: "自分で定義するか、Kiwano が見つけられないものを指定します",
  newAgent: "カスタムエージェントを作成",
  // ── Pointing Kiwano at a built-in it could not find ──
  notDetected: "このマシンでは見つかりません",
  addAgentTitle: "{agent} を追加",
  declaredAgentTitle: "{agent} のインストールディレクトリ",
  installDir: "インストールディレクトリ",
  installDirPlaceholder: "/opt/custom/bin",
  browse: "参照…",
  searchedDirs: "{count} 件のディレクトリを検索しましたが、見つかりません",
  checkingDir: "そのディレクトリを確認しています…",
  dirOk: "実行可能なファイルが見つかりました — {version}",
  checkAgain: "再確認",
  recheckMissing: "先ほどもう一度確認しましたが、依然として見つかりません。",
  recheckNoAnswer: "先ほどもう一度確認しましたが、今度はプローブが応答しませんでした。",
  checkAgainTitle: "このマシンでもう一度このエージェントを探します",
  foundTitle: "Kiwano が見つけました",
  foundBody:
    "このエージェントはここにインストールされているため、指定するものはありません。そのタブは、このダイアログの後ろにあるアプリ画面のタブバーにすでにあります。",
  addAgentNote:
    "このエージェントのコマンドがあるディレクトリを指定すると、Kiwano はそのコマンドが実行できることを確認します。それがこのエージェントのものかまでは確認しません（バージョン出力に共通の形式がないためです）。",
  declaredAgentNote:
    "Kiwano はここを指定されてこのエージェントを見つけました。保存時に確認するのは、そのコマンドが今も実行できるかどうかだけです。それがこのエージェントのものかまでは確認しません（バージョン出力に共通の形式がないためです）。",
  addAgent: "追加",
  alreadyDeclared: "追加済み",
  editAgentName: "エージェント名を編集",
  editAgentNote: "メモを編集",
  agentName: "名前",
  agentNamePlaceholder: "長いタスク",
  agentNote: "メモ",
  agentNotePlaceholder: "任意 — このルートの用途",
  agentProtocol: "プロトコル",
  agentProtocolUnset: "未指定",
  agentProtocolNote: "このルートに接続するクライアントが話すプロトコルです。表示のみで、これによるルーティング・検証・拒否は行いません。",
  agentIdNote:
    "id は名前から生成され、以後変わりません。ディスク上のものは何も変わりません。これはルートなので、設定ファイルを引き継ぐのではなく、クライアントをそのキーとともにゲートウェイへ向けます。",
  createAgent: "作成",
  routeEmptyTitle: "{agent} にはまだ候補がありません",
  routeEmptyBody:
    "以下で既存のプロバイダーを紐付けてください。このエージェントのキーで届いたリクエストは、候補の順にルーティングされます。",
  accessTab: "アクセス",
  accessEndpoint: "エンドポイント",
  accessKey: "API キー",
  accessNote:
    "このエンドポイントにこのキーで接続したクライアントは、このエージェントのストラテジーに従ってルーティングされます（/v1/messages または /v1/chat/completions）。",
  deleteAgent: "このエージェントを削除",
  deleteAgentConfirm: "もう一度クリックで削除",
  deleteAgentNote: "使用量の履歴は残ります。",


  // ── A built-in agent's own settings (the row above its table, and its dialog) ──
  agentSettingsFor: "{agent} の設定",
  agentGeneralTab: "一般",
  agentConfigFilesNote: "引き継ぎの前にバックアップし、引き継ぎをオフにすると復元します。",
  agentPlaceholderKey: "API キー",
  agentPlaceholderKeyTitle: "ゲートウェイが割り当てたプレースホルダーキー。リクエストの帰属識別に使います",
  agentAdditive: "共存 · マルチプロバイダー",
  agentDisable: "引き継ぎをオフにする",
  agentDisableConfirm: "もう一度クリックでオフ",
  agentDisableNote: "エージェント自身の設定を復元します。ルートとキーは残ります。",

  removeFromRouteAria: "ルートから削除",
  removeFromRoute: "このルートから削除（他のエージェントの紐付けはそのまま）",

  // ── Bind-another-provider row ──
  bindAria: "プロバイダーをルートに紐付け",
  allBound: "すべてのプロバイダーが紐付け済みです",
  bindExisting: "既存のプロバイダーを紐付け…",
  allInRoute: "すべてのプロバイダーがすでにこのルートにあります — 新しいものはヘッダーから追加してください",
  bindAnother: "このルートにもう1つプロバイダーを紐付けます — スタンバイとしてキューの末尾に加わります",

  // ── Empty list ──
  none: "プロバイダーがまだありません —",

  // ── Plan quota tiers ──
  tierFiveHour: "5時間枠",
  tierWeekly: "週次",
  tierMonthly: "月次",
};
