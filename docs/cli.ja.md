# `kiwano` コマンドライン

`kiwano` はデスクトップアプリなしでプロバイダー、エージェント、ルートを管理します。
アプリとゲートウェイが使うのと同じ SQLite ストア、およびステータスとホットリロード用の
ゲートウェイの管理プレーンと通信するため、ディスプレイの無いホストでも動作します。

再実装するのではなく、アプリと実装を共有しています。本書のすべてのコマンドは、
デスクトップアプリもリンクしているクレート `kiwano-core` への呼び出しです。ここで
追加したプロバイダーは、あちらで追加したものと同じ行として、同じ方法で書き込まれます。

## インストール

Linux または macOS のサーバーでは、1 つのコマンドで完了します — バイナリ、チェックサム
検証、そしてあなたのユーザー用のサービスとしてのゲートウェイ登録:

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

root は不要です。これは便利だからではなく意図的です。`agents takeover` はエージェント
の設定をそのエージェント自身の `$HOME` 配下で書き換えるため、CLI とエージェントは同じ
アカウントでなければなりません。手順の詳細とマシン全体に適用する代替手段は
[`packaging/INSTALL.md`](../packaging/INSTALL.md) を参照してください。

## グローバル

| フラグ | 意味 |
| --- | --- |
| `--db <PATH>` | SQLite ファイル。既定は `~/.kiwano/kiwano.db`、または `KIWANO_DB_PATH` |
| `--admin-socket <TARGET>` | ゲートウェイの管理プレーンの場所。unix ではソケットパス、Windows ではパイプ名。既定はデータベースの隣の `admin.sock`、または `KIWANO_ADMIN_SOCKET` |
| `--json` | stdout に機械可読な出力 |
| `--quiet` | 情報メッセージを抑制 |
| `--data-port <PORT>` | エージェントを引き継ぐ際に、その設定が指す先のポート。既定は 8317、または `KIWANO_DATA_PORT` |
| `--home <PATH>` | エージェントの設定ファイルが置かれるルート。既定は `$HOME` |

グローバルは位置に依存しません。`kiwano --json providers list` と
`kiwano providers list --json` は同じコマンドです。

## 出力と終了コード

**stdout に出るのはペイロードだけです。** `--json` ではそれはちょうど 1 つの JSON
ドキュメントなので、`kiwano --json providers list | jq` が動きます。診断 — ルート
リロード後の通知、警告、エラー — は stderr に出ます。`--json` のときも同様です。

一覧は表として出力され、数値の列（リクエスト数、トークン数、コスト、レイテンシ）は
大きさで読めるよう右端に揃えられます:

```
+-------------------------+----------+--------+------------------+---------+---------------------------+
| ID                      | NAME     | PROTO  | ENDPOINT         | BILLING | AGENTS                    |
+-------------------------+----------+--------+------------------+---------+---------------------------+
| api-deepseek-com-271eb4 | DeepSeek | openai | api.deepseek.com | payg    | codex*,demo-route-00b40a* |
+-------------------------+----------+--------+------------------+---------+---------------------------+
```

表は出力先の端末に合わせてレイアウトされます。最も幅の広い列から順に余地を譲り、
最小 6 文字まで縮み、それでも収まらないセルは `…` で省略されます。出力が端末でない
場合 — パイプやファイル — は合わせる幅が無いため、表は全体が書き出されます。
`COLUMNS` を設定して自分で幅を指定できます（`COLUMNS=100 kiwano providers list`）。
スクリプトが意図的に狭いレイアウトを得るのもこの方法です。

エラーは stdout に JSON オブジェクトとして**出力されません**。そうしてしまうと、
部分的に成功したパイプラインが、どちらのドキュメントを読んでいたのか曖昧になります。
答えは終了コードがすでに示しています:

| コード | 意味 |
| --- | --- |
| `0` | 成功 |
| `1` | 答えが「いいえ」 — ゲートウェイが停止中の `status`。失敗ではありません。ストアは正常に読めており、報告すべきゲートウェイが無いだけです |
| `2` | 使い方または検証エラー（clap 自身のパース失敗もこれを使います） |
| `3` | 実行時エラー: ストア、IO、管理プレーン、ネットワーク |

## コマンドリファレンス

完全なフラグ一覧は `kiwano <command> --help` を実行してください。以下は全体の形を
示すもので、すべてのフラグではありません。

### ステータス

```
kiwano status              gateway, store, today's totals, and the blocked list
kiwano reload              ask a running gateway to rebuild its route table
kiwano gateway start       start one, adopting an already-running gateway
kiwano gateway stop        ask the running gateway to stop (graceful: it checkpoints its WAL)
kiwano gateway restart     stop and start
```

`status` には、ゲートウェイが現在**ルーティングを拒否している**プロバイダーとその
理由が含まれます — `capped-1  1.00 of 1.00 requests this period`。これはゲートウェイの
稼働中にのみ存在する派生状態なので、シェルから見られるのはここだけです。多くの場合、
「なぜ何もルーティングされないのか」の答えがこれです。

systemd のホストでは、`gateway` サブコマンドの代わりに `systemctl restart kiwanod` を
使ってください — INSTALL.md を参照。

### プロバイダー

```
kiwano providers list [--agent AGENT]
kiwano providers add --name N --endpoint URL [--key K] [--protocol P]
                     [--billing plan|payg|unl] [--limit N --unit U] [--reset R]
                     [--bind AGENT]...
kiwano providers edit <ID> [any of the above]
kiwano providers use <ID> --agent AGENT      switch an agent, forcing single strategy
kiwano providers enable <ID>                 make it the current route, keeping the strategy
kiwano providers remove <ID>
kiwano providers quota <ID> [--force]        plan quota windows
kiwano providers probe latency|endpoint|models
```

`add` と `edit` はどちらも、転送とクォータのオプションを取ります:

```
--timeout SECS                 upstream wait for response headers (1–3600)
--retries N                    same-provider attempts before failover (0–5)
--header 'Name: value'         repeatable; merged after credential injection,
                               so these can override the injected credentials
--endpoint-extra PROTO=URL     repeatable; the same vendor on another protocol
--plan-limit-5h PCT            plan: share of the five-hour window
--plan-limit-weekly PCT        plan: share of the weekly window
--plan-query JSON              {"template":"kimi","fields":{…}} — this is what
                               `providers quota` reads
```

`edit` はさらに `--no-headers` と `--clear-plan-query` を取ります。

`providers edit` がプロバイダーの **id** を保つのは、まさにそれが存在理由だからです。
バインディング、ローテーションするキー、使用量の行がすべて id を参照するため、削除して
再追加するのは別の操作になります。変更されるのは指定したフラグだけで、それ以外は
保存済みの行から引き継がれます。

この引き継ぎは `--timeout`、`--retries`、`--header`、`--endpoint-extra`、
`--plan-limit-*` では重要です。これらはそれぞれ、基盤の API が全体として再計算する
オブジェクトの*一部*なので、1 つだけ指定した編集でも他はそのまま送られます。
プロバイダーの名前を変えてもタイムアウトは消えません。

クリアは常に明示的です — `--no-headers`、`--clear-plan-query` — フラグが無いことは
「維持」を意味するためです。`--key` も同じで、省略すると保存済みのキーが維持され、
空にはなりません。

`--plan-limit-*` はプロバイダーがプランでない限り拒否されます（基盤の層が値をおとなしく
捨ててしまうためですし、何もしないフラグは拒否するフラグより悪いからです）。
`--plan-query` があって初めて `providers quota` は何かを問い合わせられます。

`--unit` は `--limit` が何を数えるかを示します。`requests`、`wan_tokens`、または Hub が
レートを公表している通貨です。最後のものは保存されるのではなく検査されます。知らない
通貨を渡すと、知っている通貨を挙げて終了コード 2 で終わります。上限は他の通貨で
価格付けされたコストに対して測られるため、レートの無い通貨は 1:1 で加算されてしまう
からです。一度も同期していないマシンには表がまったく無く、`USD` と `CNY` を受け付け
ます — Hub がレートを公表している 2 つです。

### キーのローテーション

```
kiwano keys list <PROVIDER_ID>            masked; the key is never printed in full
kiwano keys add <PROVIDER_ID> --key K [--label L]
kiwano keys remove <KEY_ID>
```

ローテーションするキーは、リクエストごとにプロバイダーのプライマリキーの後に試されます。

### エージェント

```
kiwano agents detect                    which agents are installed
kiwano agents versions                  version strings (slow: one subprocess each)
kiwano agents takeover <AGENT>          route it through the gateway, backing up its config
kiwano agents restore <AGENT>           put the original config back
kiwano agents list                      built-in and user-defined agents, with keys
kiwano agents add --name NAME [--note TEXT] [--protocol P]
                                        define your own agent: a named route with its own key
kiwano agents remove <ID>               delete it and its key (its usage history stays)
```

`takeover` はサーバーを使えるようにするコマンドです。エージェントが現在設定している
プロバイダーを取り込み、そのエージェントのプレースホルダーキーを発行し、エージェント
自身の設定を書き換えてゲートウェイを指させます。キーは重要です。データプレーンは自分が
発行したキーだけをルーティングし、それ以外は 401 で拒否するため、手でエージェントを
ゲートウェイに向けても動きません。

プレースホルダーキーを出力します。これは結果のうち、外から見えない唯一の部分だからです。

`takeover` は `--home` の下に書き込むので、設定を意図したユーザーとして実行してください。
サービスアカウントに対しては `--home` を明示的に渡してください。

`agents add` はルートを得るもう 1 つの方法です。Kiwano が検出できず、何も書き換えない
エージェントです。名前、id、キー、そしてプロバイダーを紐付けるストラテジーが手に入ります
— ディスク上の何も変わりません。出力されたキーを使ってクライアントをゲートウェイに
向けてください。`agents list` はまだキーの無いエージェントのキーとして `-` を出力し、
`--json` は行のすべてのフィールドを含みます。

`--protocol` はそのエージェントのクライアントが話すプロトコルを記録します:
`anthropic`、`openai`、`gemini`。省略すると、エージェントは何も言わないという意味に
なり、3 つのどれとも異なる答えです。`agents list` では `-` と表示されます。これは
**ラベル**です — これによってルーティング、変換、検証が行われることはありません。
リクエストのワイヤ形式は、それが届いたパスで決まります。

### ルート

```
kiwano routes list
kiwano routes strategy <AGENT> single|failover|roundrobin|timewindow|quota
                               [--limit N --unit requests|tokens]
kiwano routes reorder <AGENT> <PROVIDER_ID>...
kiwano routes apply --from SOURCE --to TARGET
kiwano routes binding add|remove <AGENT> <PROVIDER_ID>
kiwano routes binding set <AGENT> <PROVIDER_ID> [--weight N] [--window HH:MM-HH:MM] [--no-window]
```

`routes reorder` は候補の順序を引数に取ります。書いた順が優先度 0、1、2… になります。

クォータのペイロードはゲートウェイ自身のパーサーで検証されるため、CLI が書いた設定を
エンジンが黙って別解釈することはありません。無視するストラテジーに対する `--limit` は、
無操作として受け入れられるのではなく拒否されます。

### 使用量とログ

```
kiwano usage [--days N] [--agent AGENT] [--provider ID]
kiwano dashboard [--window today|7d|30d|all] [--provider ID] [--agent AGENT]
kiwano insights [--days N] [--agent AGENT]
kiwano alerts [--mark-notified]

kiwano logs list [--agent A] [--provider P] [--status ok|error]
                 [--from RFC3339] [--to RFC3339] [--page N] [--page-size N]
kiwano logs show <ID>
kiwano logs export --out PATH [same filters] [--include-bodies]
kiwano logs clear --yes
kiwano logs dir
```

`--from` は境界を含み、`--to` は含みません。ストアの半開区間に対応しています。

`insights` は、トークンが*どのように*使われているかについての 1 ページのレポートです。
エージェントごとのスコアカード（キャッシュヒット率、セッションコンテキストの増加、
推論の割合、リトライ）、`cache` / `bloat` / `retry` / `overhead` のタグが付いたルール
検出、そしてコンテキストが最も増えたセッションをまとめます。どの検出も、その背後にある
`request_logs` の id を伴います — `kiwano logs show <ID>` で 1 件を開き直せます — そして
すべてはログからローカルに計算されます。本文は固定のツール／システムペイロードを測る
ためにサンプリングされますが、このマシンから出ることはありません。金額もどこにも
現れません（コストはダッシュボードの役目です）。グローバルの `--json` フラグは同じ
レポートを機械可読な形で出力します。

`alerts` は**既定では読み取り専用**です。そうでなければ書き込まれる重複排除キーは、
デスクトップアプリが通知を出す前に参照するものと同じなので、スクリプトによるポーリング
は、あなたが待っていたアラートを黙って飲み込んでしまいます。CLI に配信を任せたい場合
だけ `--mark-notified` を渡してください。

`logs clear` は `--yes` を必要とします。アプリはダイアログで確認し、TTY の無いシェルでは
フラグがその相当物です。

### 設定・構成・カタログ

```
kiwano settings get
kiwano settings set --key KEY=VALUE [--key ...] [--patch JSON]

kiwano config export --out PATH [--include-keys]
kiwano config import --file PATH
kiwano import cc-switch

kiwano catalog list [--tag official|aggregate|third|free] [--search QUERY]
kiwano catalog sync
kiwano catalog currency
```

`settings set` への値は、パースできるときは JSON としてパースされます。したがって
`--key cost_alert=false` はブール値、`--key log_retention_days=30` は数値になります。

`config export --include-keys` は有効なクレデンシャルを書き出すため、ファイルは所有者
限定（`0600`）で作成されます。フラグ無しの `config export` はバージョン管理に
置いても安全です。両者は `config import` で往復でき、上書きではなく名前とエンドポイント
でマージされます。

`import cc-switch` は `~/.cc-switch/cc-switch.db`（v3.20 以降の SQLite。読み取り用に
開く）または古い `~/.cc-switch/config.json` を読み、Kiwano 自身のデータベースにだけ
書き込みます。`~/.cc-switch` 配下は変更も削除もされないので、判断している間も両者を
並べて置けます。

知っている 8 つのアプリ — claude、claude-desktop、codex、grokbuild、opencode、
openclaw、hermes、pi — を同名のエージェントに対応付け、cc-switch で*現在*使われていた
プロバイダーがそのエージェントのプライマリバインディングになります。つまり移行される
のは、候補の一覧から選び直すのではなく、実際に動かしていた構成です。

もう一度実行しても安全で、すでに使っている Kiwano の隣でも安全に実行できます。同じ
エンドポイントとプロトコルで既にあるプロバイダー — 手で追加したものか、以前の実行で
取り込んだもの — は、二度追加されるのではなく*再利用*され、名前、キー、課金はそのまま
です。取り込みは、もともと無かったものだけを埋めます。すでにルートを持つエージェントは
そのままにされ、まだルートの無いエージェントにだけ取り込みます。そのため再実行で
エージェントを、いったん切り替えた先から元へ戻してしまうことはありません。

2 つのケースは推測せず、報告されます。cc-switch の `gemini` の行はスキップします。
対象は Gemini CLI で、設定は Gemini の形をしており、このゲートウェイにはそれらを
取り込めるプロトコルがありません。base URL かキーが無いプロバイダーは、空で作成される
のではなくスキップとして一覧されます。どのスキップも理由を伴います。

`catalog list` は Hub のキャッシュだけを読み、それ以外は読みません — 同梱のコピーは
無いので、一度も同期していないマシンは古いカタログではなく何も一覧しません。先に
`catalog sync` を実行してください。何かをカタログから追加する前に新しいサーバーが
すべきはこれで、その後はキャッシュが他のものと同様にオフラインで機能します。

**`catalog add` はありません。** カタログのワンクリック「追加」は、エントリを埋めた
状態でプロバイダーフォームを開くだけで、独自の API を呼びません。したがって機能としては
`providers add` であり、値は `catalog list --json` から取ります:

```sh
kiwano catalog list --json --search deepseek
kiwano providers add --name DeepSeek --endpoint https://api.deepseek.com/anthropic \
    --protocol anthropic --billing payg --limit 50 --unit CNY --bind claude
```

事前入力の規則（`payg` の上限 50、および一部のカタログエントリが持つエージェントの
事前バインド）はアプリのフォームにあり、共有レイヤーにはありません。この対応付けが
後に `kiwano-core` に移れば、これは 1 コマンドのショートカットになります。それまでは、
CLI が意図的に持たない便宜であって、欠いている機能ではありません。

CLI が実際に記録するのは、あるプロバイダーがどのエントリに対応するかです。それに
よって、そのエントリ自身の公表レートでリクエストの価格が決まるからです。これは尋ねる
のではなくエンドポイントから推測されます。`providers add` はエンドポイントがちょうど
1 つのエントリを指すときにプロバイダーをリンクし、`catalog sync` はまだリンクの無い
プロバイダーに対して同じことをします。推測はしません — どのエントリにも一致しない、
または複数（同じホストを共有する 2 つのエントリ）に一致するエンドポイントは、リンク
されないまま一般レートで価格付けされます。

## アプリとの対応

デスクトップアプリのユーザー向け機能は、本質的にグラフィカルなもの（トレイ、ウィンドウ、
テーマ、通知、i18n、アプリ内セルフアップデート）を除いて、すべてここから到達できます。

| アプリの機能 | コマンド |
| --- | --- |
| ゲートウェイの状態 + 今日の合計 | `status` |
| プロバイダーの一覧 / 追加 / 編集 / 削除 | `providers list\|add\|edit\|remove` |
| プロバイダーをカレントにする | `providers enable`, `providers use` |
| レイテンシ / エンドポイント / モデルのプローブ | `providers probe …` |
| プランクォータのリング | `providers quota` |
| ローテーションする API キー | `keys list\|add\|remove` |
| エージェントのルート + ストラテジー | `routes list`, `routes strategy` |
| 候補の順序、重み、時間帯 | `routes reorder`, `routes binding …` |
| エージェント間のルートのコピー | `routes apply` |
| エージェントの検出とバージョン | `agents detect`, `agents versions` |
| 引き継ぎ / 復元 | `agents takeover`, `agents restore` |
| 自分のエージェント（名前付きルート） | `agents list\|add\|remove` |
| 使用量の合計 | `usage` |
| ダッシュボードの推移 | `dashboard` |
| 使用量のアラート | `alerts` |
| リクエストログ、詳細、CSV エクスポート、クリア | `logs …` |
| ログファイルの場所 | `logs dir` |
| 設定 | `settings get\|set` |
| 構成のエクスポート / インポート | `config export\|import` |
| cc-switch からの移行 | `import cc-switch` |
| カタログ + Hub 同期 + 通貨 | `catalog list\|sync\|currency` |
| デーモンのライフサイクル | `gateway start\|stop\|restart` |

どこにも無いもの: アップデートの確認、アップデートのダウンロード、進捗イベントの
ストリームです。これらはデスクトップアプリのインストーラーの性質であり、サーバー
バンドルは新しいものをインストールして更新します。

## `kiwano-cli` からの移行

`kiwano-cli` は無くなり、エイリアスもありません。それが行っていたことはすべて
`kiwano` の下にありますが、知っておくべき 2 つの違いがあります:

1. **課金の語彙はアプリのものです。** `--billing plan|payg|unl` が正で、
   `subscription|metered|unlimited` は今もエイリアスとしてパースされます。**プラン**の
   プロバイダーでは、`--limit`、`--unit`、`--reset` は拒否されるようになりました —
   プランのクォータはプランクエリから来るもので、これらのフラグはかつてアプリが
   読まない列に書き込んでいました。

2. **`--json` の出力は stdout 上の単一ドキュメントです。** 診断は stderr に移り、
   これが変更系コマンドを直しました。以前は `kiwano-cli --json providers add … | jq`
   がパースできませんでした。リロードの通知が JSON の後に stdout へ出力されていた
   ためです。

移動したサブコマンド:

| 旧 | 新 |
| --- | --- |
| `kiwano-cli status` | `kiwano status` |
| `kiwano-cli providers use <ID> --agent A` | `kiwano providers use <ID> --agent A`（変更なし） |
| `kiwano-cli keys list\|add\|remove` | `kiwano keys list\|add\|remove`（変更なし） |
| `kiwano-cli usage --days N` | `kiwano usage --days N`（変更なし） |

終了コードは新しくなりましたが、重要なものは趣旨としては変わっていません。
ゲートウェイが停止しているとき、`status` は今も `1` で終了します。
