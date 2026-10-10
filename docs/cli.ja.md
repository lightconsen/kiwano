# `kiwano` コマンドライン

`kiwano` はデスクトップアプリなしでプロバイダー、エージェント、ルートを管理します。
app と同じく**ゲートウェイのクライアント**です — 読み書きはすべて管理面の API を
通り、共有データベースに書き込むのはゲートウェイだけです。ディスプレイの無いホスト
でも動作し、[2 つの環境変数](remote-gateway.md)を設定すれば別のマシンの
ゲートウェイも管理できます。

このツール自身が触るのは、このマシンについての事実だけです — エージェントの設定が
どこにあり、ゲートウェイのキーを持っているかどうか、そして共有データベースの隣にある
小さなクライアント側データベース(宣言したディレクトリと、テイクオーバーを取り消す
ためのバックアップ)。

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
kiwano providers price <ID> --model M --input X --output Y
                            [--cache-read Z] [--cache-creation W] [--currency CODE]
kiwano providers price <ID> --model M --clear
kiwano providers probe latency|endpoint|models
```

`providers price` は、あるプロバイダーがあるモデルにいくら課金するかを百万トークン単位で宣言します —— あなたにしか出せない数字です。Hub が値付けするのは**自分**のカタログ項目のモデルで、項目を持たないプロバイダーには公表価格がありません。プロバイダーの宣言は**1 つの通貨とモデルの一覧全体**なので、このコマンドは保存済みの内容を読み、そのモデルを畳み込み、一覧ごと送り返します —— 2 つ目のモデルを宣言しても 1 つ目は消えません。`--clear` は 1 つを外します。

Hub が値付けできないモデル名の逃げ道でもあります —— たとえばプラン自身の名前で、それはサブスクリプションが現在与えているモデルを指します。`kiwano history import` は値付けできなかった名前を報告するので、その 1 つに価格を宣言すれば、その行が金額に数えられます。

`--openai-wire` は、OpenAI プロバイダーが**どちらの**リクエスト形状に対応するかを述べます。両者が食い違うときだけ意味を持ちます：`chat` は `/v1/chat/completions` のみ、つまり agent が送る `/v1/responses`（Codex）は送信時に Chat Completions へ翻訳され、応答は逆に翻訳されます。`responses` は `/v1/responses` のみで、この場合 `/v1/chat/completions` は推測せず**理由を添えて拒否**されます。`both`（既定）はそのまま通し、何に対応しているかはベンダーが答えます。このフラグ以前に追加されたプロバイダーはすべて `both` なので、指定しなければ何も変わりません。

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

### ローカル履歴

```
kiwano history import [--agent claude|codex]… [--dry-run]
```

Agent 自身の履歴です。エージェントのセッションファイルから読み取って取り込むので、
今朝 Kiwano を入れたばかりのマシンでも dashboard に昨日の出費が並びます。`import` は
Claude Code と Codex の記録ファイルを読み、書かれていることを送ります。アプリは初回
起動時に同じスキャンを一度だけ実行します。`--dry-run` は読んで報告するだけで何も書き
ません —— 「なぜ dashboard がまだ空なのか」への唯一正直な答えで、ファイルがこのマシンの
想定した場所に無いだけかもしれません（その行は stderr に出ます。スキャンこそ `| jq` する
コマンドだからです）。

数字を読む前に 2 点：

- **ゲートウェイより前の分だけ**を取り込みます。テイクオーバー以降のリクエストはすでに
  台帳にあり、エージェントはどちらにせよ自分のファイルを書きます。境界は各 Agent 自身の
  最初の計量行なので、同じリクエストが二重に数えられることはありません。`import` は
  その境界が何行を弾いたかを報告します。
- **コストは推定値です。** ファイルにあるのはトークンで金額ではなく、取り込んだリクエスト
  には宣言価格を当てられるプロバイダーがありません —— 各行はそのモデルの一般価格で
  見積もられます。Hub を一度も同期していないマシンでは何も価格が付きません（トークンだけ
  で着地します）。あとで `history import` をもう一度走らせると埋まります。取り込みは
  冪等だからです。
- **各行はどのマシンから来たかを覚えます。** 2 台のノート PC が同じゲートウェイへ履歴を
  取り込むことがあり、そのとき統合された台帳には、どの行が誰のものかを区別する手立てが
  要ります。取り込みはマシンの**ラベル** —— このマシンが自分について調べられたもの
  （多くの場合ホスト名、ベストエフォート）—— を記録し、報告にもそれを書きます。これは
  ラベルであって身元ではありません。2 台が同じ名前を持つことも、名前が変わることもあり、
  自分の名前を調べられなかったマシンは何も記録しません（`unnamed` と表示されます）。
  ゲートウェイがルーティングしたトラフィックには、そもそも取り込み元のマシンがありません。

取り込まれたものは `kiwano sessions`（下記）が一覧します。ゲートウェイ自身がルーティング
したトラフィックと並べて表示されます。

### セッション

```
kiwano sessions [--project P] [--agent A] [--days N]
```

1 セッション 1 行で、**2 つの情報源**から来ます。どちらなのかは `SOURCE` 列が示します：

- `gateway` —— そのセッションのリクエストはゲートウェイがルーティングしました。リクエスト
  数・トークン・コストは分かりますが、プロジェクト・ターン数・ツールは分かりません。
- `imported` —— エージェント自身のファイルにしか無いものです（`kiwano history import` が
  読み込みます）。プロジェクト・ターン数・ツールは分かりますが、金額はありません。ファイルに
  あるのはトークンでコストではないからです。
- `both` —— 両方の台帳に同じ id があります。テイクオーバー以降に走ったセッションの通常の
  状態です。

**2 つの情報源は決して足し合わせません。** 両者は重なるけれど等しくない仕事を表すからです
—— 1 回のリトライはファイルでは 1 ターン、ゲートウェイでは 2 リクエストです。そこで両方に
同じ項目があるときは、片方だけを採り、もう片方は捨てます（合算しません）：リクエスト数・
トークン・コストは**トラフィック**が、プロジェクト・期間・ターン数・ツールは**ファイル**が
優先です。`both` 行のトークン列はゲートウェイが数えた値で、ファイルの値と足したものでは
ありません。

列は次のとおりです：

```
PROJECT  MACHINE  AGENT  SOURCE  SPAN  REQ  TURNS  TOKENS (in/out/read/write)  COST  TOOLS
```

`imported` 行の `REQ` は 0、`gateway` 行の `TURNS` は 0 です。どちらの側も、もう一方の
数をでっち上げません。`COST` はトラフィックが使った金額で、通貨ごとに分けて表示し、換算は
しません。`-` はファイル側にトラフィックが無いことを意味し（「無料」ではありません）、
一部のリクエストに価格が付かなかったセッションはその旨を書きます
（`2.50 USD (+3 unpriced)`）。`SPAN` はファイルがあればセッションの全期間、無ければ
ゲートウェイが見たリクエストの範囲です。

`MACHINE` は取り込んだ行がどのマシンから来たかです（上の**ローカル履歴**を参照）。
取り込み側のクライアントが自分のために記録したラベルで、自分の名前を調べられなければ
`unnamed`、ゲートウェイだけが見た行も同じく `unnamed` です —— それはゲートウェイを
通って来たのであって、ファイルから来たのではありません。読者のためのラベルであり、
2 台のマシンを確実に見分ける身元ではありません。

プロジェクトは記録に残った作業ディレクトリから導いた**ラベル**で、リポジトリのディレクトリ名
です（パスではありません。ゲートウェイはこのマシンの配置を知らされないからです）。また、
ゲートウェイだけが見た行にはプロジェクトが無いので、`--project` はファイル側が場所を
示せたセッションだけを表示します。

末尾に、ファイルを最後にスキャンした時刻が出ます（一度も無ければ `kiwano history import`
を実行するよう書かれます）。

### クライアントキー

```
kiwano clients list [--agent A]                 ハンドル、Agent、上限と許可リスト
kiwano clients show <ID>                        ハンドル（ck-…）で 1 つ見る
kiwano clients add --agent A [--label L]        1 つ鋳造する。キーはこの一度だけ表示
                   [--model M]… [--provider P]… [--window PERIOD:LIMIT[:UNIT]]…
kiwano clients remove <ID>
kiwano clients rotate <ID>                      新しいキーに差し替え、ハンドルと予算はそのまま
kiwano clients limits <ID> [--window PERIOD:LIMIT[:UNIT]]…   --window を省くと消去
kiwano clients policy <ID> [--label L] [--model M]… [--provider P]…
```

データプレーンの資格情報です。Agent はこれを提示し、同時にこれで自分のできることが
制限されます。1 つのキーは 1 つの Agent に対応し、同じ Agent の 2 つ目のキーは 2 つ目の
予算です —— 2 つ要るのは、同じ Agent をノート PC とデスクトップで走らせている場合です。
窓は `day:200:requests`、`monthly:20:USD`、`weekly:5:wan_tokens` のように書き、単位は
省略できます（省略時は `requests`）。窓を超えるとゲートウェイは 429 と `Retry-After` を
返し、許可リストの外のモデルやプロバイダーには 403 を返します。保存済みのキーは
**読み戻せません** —— `add` と `rotate` が一度だけ表示します。

`list` の最後の列は**最終使用** —— そのキーが最後にリクエストを運んだ時刻で、一度も
使っていなければ `never` です。鍵を取り消すかどうかはこの列で決めるので、2 つの状態は
書き分けます：`never` は「鋳造したが未使用」、日付は「使われ、その後は使われていないかも
しれない」。計量行を読むので、**このゲートウェイ**を通ったトラフィックしか知りません。

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
                 [--from RFC3339] [--to RFC3339] [--session ID]
                 [--page N] [--page-size N]
kiwano logs show <ID>
kiwano logs export --out PATH [same filters] [--include-bodies]
kiwano logs clear --yes
kiwano logs dir
```

`--from` は境界を含み、`--to` は含みません。ストアの半開区間に対応しています。
`--session` はあるリクエストが持っていたセッション id —— `kiwano sessions` が一覧する
のと同じ文字列 —— を受け取り、そこの 1 行をここで開けるようにします。

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
