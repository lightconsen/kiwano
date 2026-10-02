export const strategy = {
  // ── Strategy labels ──
  single: "単一プライマリ",
  failover: "フェイルオーバー",
  roundrobin: "重み付きラウンドロビン",
  timewindow: "時間帯",
  quota: "クォータフォールバック",
  leastBusy: "最小負荷",

  // ── One-line description shown beside the select ──
  singleHint: "常にプライマリのプロバイダーを使用します",
  failoverHint: "障害時は待機系へ順に切り替え、復旧すると元に戻ります",
  roundrobinHint:
    "新しいセッションは重みに従って順に割り当てられます。セッションごとに固定され、上流のプロンプトキャッシュを維持します",
  timewindowHint:
    "セッションの開始先は候補のローカル時間帯で決まります。実行中のセッションはプロバイダーを維持し、どの時間帯にも一致しないリクエストはプライマリにフォールバックします",
  quotaHint:
    "その日のプライマリが下記の数値に達すると、それ以降に開始するセッションは待機系に送られます — 実行中のセッションはプロバイダーを維持します",
  leastBusyHint:
    "各リクエストは最も空いている正常な候補に送られます。実行中のセッションはプロバイダーを維持します",

  // ── Quota-fallback unit select ──
  unitRequestsDay: "リクエスト/日",
  unitTokensDay: "トークン/日",

  // ── Select trigger ──
  ariaFor: "{agent} のストラテジー",

  // ── Copy-another-agent's-route row ──
  replaceConfirmOne:
    "{mine} のルーティングを {source} のもの（{strategy}、{count} 候補）に置き換えますか？",
  replaceConfirmOther:
    "{mine} のルーティングを {source} のもの（{strategy}、{count} 候補）に置き換えますか？",
  replace: "置き換え",
  copyRouteFrom: "ルーティングのコピー元…",
  copyRouteAria: "他のエージェントから {agent} にルーティングをコピー",
  pickAgent: "エージェントを選択",
  candidateOne: "{agent} · {count} 候補",
  candidateOther: "{agent} · {count} 候補",


  // ── The agent's own ceiling (its own row, under the strategy) ──
  limitAdd: "期間を追加",
  limitRemoveAria: "この期間を取り除く",
  limitLabel: "上限",
  limitNone: "上限なし",
  limitEditAria: "{agent} の上限を編集",
  limitTitle: "{agent} の上限",
  limitPlaceholder: "なし",
  limitAmountAria: "上限値",
  limitClear: "クリア",
  limitUnitRequests: "リクエスト",
  limitUnitWanTokens: "1万トークン",
  limitPeriodDay: "毎日",
  limitPeriodWeek: "毎週",
  limitPeriodMonth: "毎月",
  limitPeriodYear: "毎年",
  limitPeriodAll: "累計",
  limitBody:
    "このエージェントが使用するすべてのプロバイダーを横断して計測され、あらゆるストラテジーで適用されます — single も例外ではありません。single の「プロバイダーは 1 つ」という規則は、いくら使ってよいかについては何も定めていません。1 つのエージェントに複数の期間を同時に設定でき、そのいずれかを超えると、その期間がリセットされるまでリクエストが拒否されます。",
  limitMoneyNote:
    "金額の上限は、価格表で価格を付けられるものだけを集計します。モデルに公開価格がないリクエストは計量されますが金額は算出されないため、上限には算入されません。",
  limitCurrencyNote:
    "金額の上限は、このエージェントのプロバイダーが請求に用いる通貨で指定します — 別の通貨での支出は、算入前に Hub のレートで換算されます。上限そのものは決して換算されません。",
};
