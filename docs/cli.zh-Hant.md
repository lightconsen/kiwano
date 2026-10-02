# `kiwano` 命令列

`kiwano` 讓你在沒有桌面應用程式的情況下管理供應商、Agent 與路由。它存取的
是應用程式與閘道共用的同一個 SQLite 儲存區,並透過閘道的管理平面取得狀態
與熱重載,因此在完全沒有顯示器的機器上也能運作。

它與應用程式共用實作,而不是另外重寫一份:本文件裡的每個命令都是對
`kiwano-core` 的呼叫,桌面應用程式連結的也是同一個 crate。在這裡新增的
供應商,與在那裡新增的是同一列,寫法也相同。

## 安裝

在 Linux 或 macOS 伺服器上,一行命令搞定全部——二進位檔、總和檢查碼驗證,
以及把閘道註冊為你的使用者服務:

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

它不需要 root,這是刻意的設計而非圖方便:`agents takeover` 會改寫 Agent 在
它自己 `$HOME` 下的設定,所以 CLI 與 Agent 必須是同一個帳號。它逐步做了
什麼,以及全機器的替代做法,見
[`packaging/INSTALL.md`](../packaging/INSTALL.md)。

## 全域選項

| 旗標 | 意義 |
| --- | --- |
| `--db <PATH>` | SQLite 檔案。預設 `~/.kiwano/kiwano.db`,或 `KIWANO_DB_PATH` |
| `--admin-socket <TARGET>` | 閘道管理平面的位置:unix 上是 socket 路徑,Windows 上是 pipe 名稱。預設:資料庫旁的 `admin.sock`,或 `KIWANO_ADMIN_SOCKET` |
| `--json` | stdout 上的機器可讀輸出 |
| `--quiet` | 抑制資訊性提示 |
| `--no-reload` | 變更後不要求執行中的閘道重載 |
| `--data-port <PORT>` | 接管 Agent 時,其設定被指向的連接埠。預設 8317,或 `KIWANO_DATA_PORT` |
| `--home <PATH>` | Agent 設定檔所在的根目錄。預設 `$HOME` |

全域選項與位置無關:`kiwano --json providers list` 與
`kiwano providers list --json` 是同一個命令。

## 輸出與結束碼

**stdout 只帶酬載,別無其他。** 在 `--json` 下,那正好是一份 JSON 文件,
所以 `kiwano --json providers list | jq` 能運作。診斷訊息——路由重載後的
提示、警告、錯誤——一律走 stderr,*包括*在 `--json` 之下。

列表以表格輸出,數字欄(請求數、token 數、成本、延遲)靠右對齊,方便依量級
閱讀:

```
+-------------------------+----------+--------+------------------+---------+---------------------------+
| ID                      | NAME     | PROTO  | ENDPOINT         | BILLING | AGENTS                    |
+-------------------------+----------+--------+------------------+---------+---------------------------+
| api-deepseek-com-271eb4 | DeepSeek | openai | api.deepseek.com | payg    | codex*,demo-route-00b40a* |
+-------------------------+----------+--------+------------------+---------+---------------------------+
```

表格會依輸出所在的終端機調整寬度:最寬的欄先讓出空間,最多縮到六個字元的
下限,放不下的儲存格以 `…` 截略。當輸出不是終端機——管線、檔案——就沒有
寬度可配合,表格會完整寫出;可自行用 `COLUMNS` 指定寬度
(`COLUMNS=100 kiwano providers list`),腳本也能藉此刻意取得窄版版面。

錯誤**不會**以 JSON 物件的形式輸出到 stdout。放一個進去,會讓部分成功的
管線無從判斷它讀的是哪一份文件,而結束碼已經給出答案:

| 代碼 | 意義 |
| --- | --- |
| `0` | 成功 |
| `1` | 答案是「否」——閘道已停止時的 `status`。這不算失敗:儲存區讀取正常,只是沒有閘道可報告 |
| `2` | 用法或驗證錯誤(clap 自己的解析失敗也用這個) |
| `3` | 執行期錯誤:儲存區、IO、管理平面、網路 |

## 命令參考

完整旗標清單請跑 `kiwano <command> --help`。以下地圖是輪廓,不是每個旗標。

### 狀態

```
kiwano status              gateway, store, today's totals, and the blocked list
kiwano reload              ask a running gateway to rebuild its route table
kiwano gateway start       start one, adopting an already-running gateway
kiwano gateway stop        ask the running gateway to stop (graceful: it checkpoints its WAL)
kiwano gateway restart     stop and start
```

`status` 會列出閘道目前**拒絕路由**的供應商及其理由——
`capped-1  1.00 of 1.00 requests this period`。那是衍生狀態,只在閘道執行
時存在,所以這是 shell 唯一能看到它的地方,通常也就是「為什麼什麼都沒
路由」的答案。

在 systemd 主機上,請用 `systemctl restart kiwanod` 取代 `gateway` 子命令
——見 INSTALL.md。

### 供應商

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

`add` 與 `edit` 也都接受轉送與配額選項:

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

`edit` 另外接受 `--no-headers` 與 `--clear-plan-query`。

`providers edit` 會保留供應商的 **id**,這正是它存在的理由:綁定、輪換金鑰
與用量列全都參照它,所以「移除後再新增」是另一回事。只有你給的旗標會被
變更;其餘一律從已儲存的列沿用過來。

這套沿用對 `--timeout`、`--retries`、`--header`、`--endpoint-extra` 與
`--plan-limit-*` 至關重要:每一個都是底層 API 會整體重算的物件的*一部分*,
所以只指名其中一個的編輯,仍會把其餘的一併送出。重新命名供應商不會清掉
它的 timeout。

清除一律要明講——`--no-headers`、`--clear-plan-query`——因為沒給的旗標代表
保留。`--key` 同理:省略它會保留已儲存的金鑰,而不是把它清空。

除非供應商是方案,否則 `--plan-limit-*` 會被拒絕(否則底層會默默丟掉這個
值,而一個什麼都不做的旗標比一個會拒絕的旗標更糟)。`--plan-query` 才是
`providers quota` 問得出任何東西的前提。

`--unit` 說明 `--limit` 計算的是什麼:`requests`、`wan_tokens`,或 Hub 有
公布匯率的貨幣。最後一種是檢查而非儲存,不認得時會以結束碼 2 結束,並附上
它認得的貨幣:限額是拿來與以其他貨幣計價的成本相比,沒有匯率的限額會被
以 1:1 併入它們。從未同步過的機器完全沒有匯率表,只接受 `USD` 與 `CNY`
——Hub 公布匯率所對的兩種貨幣。

### 輪換金鑰

```
kiwano keys list <PROVIDER_ID>            masked; the key is never printed in full
kiwano keys add <PROVIDER_ID> --key K [--label L]
kiwano keys remove <KEY_ID>
```

輪換金鑰會在供應商的主金鑰之後嘗試,依每個請求逐一進行。

### Agent

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

`takeover` 是讓伺服器變得可用的命令。它匯入 Agent 目前設定的供應商、簽發
該 Agent 的佔位金鑰,並改寫 Agent 自己的設定以指向閘道。金鑰要緊:資料
平面只路由它自己簽發的金鑰,其餘一律以 401 拒絕,所以手動把 Agent 指向
閘道是行不通的。

它會印出佔位金鑰,因為那是結果中唯一從外面看不見的部分。

`takeover` 寫在 `--home` 之下,所以請以你要改其設定的那位使用者身分執行
——或為服務帳號明確傳入 `--home`。

`agents add` 是取得路由的另一條路:一個 Kiwano 偵測不到、也不會為它改寫
任何東西的 Agent。你會得到名稱、id、金鑰,以及一個可綁定供應商的策略——
磁碟上什麼都不會變,所以請用印出的金鑰把用戶端指向閘道。`agents list` 對
還沒有金鑰的 Agent 會印出 `-`,而 `--json` 會帶出該列上的每一個欄位。

`--protocol` 記錄該 Agent 的用戶端說的是什麼:`anthropic`、`openai` 或
`gemini`。省略它代表該 Agent 不表態,這是與三者都不同的另一個答案,
`agents list` 會顯示為 `-`。它是一個**標籤**——沒有任何路由、轉換或驗證
依賴它;請求的傳輸格式由它送達的路徑決定。

### 路由

```
kiwano routes list
kiwano routes strategy <AGENT> single|failover|roundrobin|timewindow|quota
                               [--limit N --unit requests|tokens]
kiwano routes reorder <AGENT> <PROVIDER_ID>...
kiwano routes apply --from SOURCE --to TARGET
kiwano routes binding add|remove <AGENT> <PROVIDER_ID>
kiwano routes binding set <AGENT> <PROVIDER_ID> [--weight N] [--window HH:MM-HH:MM] [--no-window]
```

`routes reorder` 以引數決定候選順序:你寫下的順序就成為優先順序 0、1、2……

配額酬載由閘道自己的解析器驗證,所以 CLI 寫出的設定,不會是引擎會默默
重新詮釋的那種。在不理會 `--limit` 的策略上加 `--limit`,會被拒絕,而不是
當成無作用的設定接受。

### 用量與日誌

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

`--from` 為包含、`--to` 為排除,與儲存區的半開區間一致。

`insights` 是一頁式的報告,說明 token *如何*被花掉:每個 Agent 的計分卡
(快取命中率、工作階段上下文成長、推理佔比、重試),標上 `cache` /
`bloat` / `retry` / `overhead` 的規則發現,以及上下文成長最多的那些工作
階段。每一項發現都帶著它背後的 `request_logs` id——用
`kiwano logs show <ID>` 重新打開其中一筆——而且全部都在本機從日誌計算:
內文會被取樣以量測固定的 tools/system 酬載,但它們從不離開這台機器,任何
地方都不會出現金額(成本是 dashboard 的事)。全域 `--json` 旗標會輸出同一
份報告供機器取用。

`alerts` **預設唯讀**。它原本會寫入的去重金鑰,正是桌面應用程式在發出
通知前查詢的那一個,所以腳本化的輪詢會默默吞掉你正在等的那則警示。若你
真的要 CLI 接手遞送,請傳入 `--mark-notified`。

`logs clear` 需要 `--yes`:應用程式用對話框確認,而對沒有 TTY 的 shell
來說,旗標就是對等做法。

### 設定、組態與目錄

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

`settings set` 的值若能以 JSON 解析就會如此解析,所以
`--key cost_alert=false` 是布林值,`--key log_retention_days=30` 是數字。

`config export --include-keys` 會寫出使用中的憑證,因此該檔案以僅限擁有者
(`0600`)建立。不帶該旗標的 `config export` 可安心放進版本控制;兩者都能
經由 `config import` 往返,匯入時依名稱與端點合併,而不是覆寫。

`import cc-switch` 讀取 `~/.cc-switch/cc-switch.db`(v3.20+ 的 SQLite,以
唯讀開啟)或較舊的 `~/.cc-switch/config.json`,且只寫入 Kiwano 自己的
資料庫。`~/.cc-switch` 底下的東西不會被修改或刪除,所以在你決定之前,
兩者可以並存。

它把它認得的所有八個應用程式——claude、claude-desktop、codex、grokbuild、
opencode、openclaw、hermes 與 pi——對應到同名的 Agent,而 cc-switch 裡
*當前*的供應商會成為該 Agent 的主綁定,所以移轉過來的是你實際在用的設定,
而不是一份要你重新挑選的候選清單。

重複執行是安全的,在已經在用的 Kiwano 旁邊執行也安全。若這裡已經有相同
端點與協定的供應商——你手動新增的,或先前某次匯入的——會被*重複使用*而不
是再加第二筆,而且它的名稱、金鑰與計費都不動:匯入只補上原本沒有的東西。
已經有路由的 Agent 會留在原路由上;只有還沒有路由的 Agent 會被匯入,所以
重跑永遠不會把 Agent 搬回你後來已經切走的東西上。

有兩種情形會被回報,而不是擅自猜測。cc-switch 的 `gemini` 列會被跳過:
它們的目標是 Gemini CLI,其設定是 Gemini 的形狀,而本閘道沒有可將它們
匯入的協定。沒有 base URL 或沒有金鑰的供應商會被列為跳過,而不是建立成
空的。每一次跳過都附上理由。

`catalog list` 只讀 Hub 快取,別無其他——沒有內附的副本,所以從未同步過的
機器列出的是空的,而不是過時的目錄。請先跑 `catalog sync`;全新的伺服器
在從貨架新增任何東西之前就該這麼做,之後快取便與其他快取一樣能離線供應。

**沒有 `catalog add`。** 貨架上的一鍵「新增」會開啟以該條目預填的供應商
表單——它不呼叫自己的 API,所以能力就是 `providers add`,而值來自
`catalog list --json`:

```sh
kiwano catalog list --json --search deepseek
kiwano providers add --name DeepSeek --endpoint https://api.deepseek.com/anthropic \
    --protocol anthropic --billing payg --limit 50 --unit CNY --bind claude
```

預填規則(`payg` 限額 50,以及某些目錄條目帶的 Agent 預綁定)存在於應用
程式的表單裡,不在共用的層級。若那套對應日後移進 `kiwano-core`,這就會
變成一行命令的捷徑——在那之前,它是 CLI 沒有的便利,而不是它欠缺的能力。

CLI *確實*會記錄的,是供應商對應到哪個條目,因為那決定了它的請求要按該
條目自己公布的價格計價。這是從端點推斷而非詢問得來:`providers add` 在
端點恰好對到一個條目時連結該供應商,`catalog sync` 也對還沒有連結的供應商
做同樣的事。不做任何猜測——對不到任何條目、或對到多個(兩個條目共用一個
主機)的端點會保持未連結,並按一般價格計價。

## 與應用程式的對應關係

桌面應用程式的每一項使用者可見能力在這裡都搆得到,除了本質上屬於圖形的
那些(系統匣、視窗、佈景主題、通知、i18n、應用程式內自我更新)。

| 應用程式能力 | 命令 |
| --- | --- |
| 閘道狀態 + 今日總計 | `status` |
| 供應商清單 / 新增 / 編輯 / 刪除 | `providers list\|add\|edit\|remove` |
| 把供應商設為當前 | `providers enable`, `providers use` |
| 延遲 / 端點 / 模型探測 | `providers probe …` |
| 方案配額環 | `providers quota` |
| 輪換 API 金鑰 | `keys list\|add\|remove` |
| Agent 路由 + 策略 | `routes list`, `routes strategy` |
| 候選順序、權重、時間窗 | `routes reorder`, `routes binding …` |
| 在 Agent 之間複製路由 | `routes apply` |
| Agent 偵測與版本 | `agents detect`, `agents versions` |
| 接管 / 還原 | `agents takeover`, `agents restore` |
| 你自己的 Agent(具名路由) | `agents list\|add\|remove` |
| 用量總計 | `usage` |
| Dashboard 趨勢 | `dashboard` |
| 用量警示 | `alerts` |
| 請求日誌、明細、CSV 匯出、清除 | `logs …` |
| 日誌檔位置 | `logs dir` |
| 設定 | `settings get\|set` |
| 組態匯出 / 匯入 | `config export\|import` |
| cc-switch 移轉 | `import cc-switch` |
| 目錄貨架 + Hub 同步 + 貨幣 | `catalog list\|sync\|currency` |
| 常駐程式生命週期 | `gateway start\|stop\|restart` |

完全沒有涵蓋的:更新檢查、更新下載,以及進度事件串流。那些是桌面應用程式
安裝程式的性質,而伺服器套裝是藉由安裝新的來更新。

## 從 `kiwano-cli` 移轉

`kiwano-cli` 已經移除,也沒有別名。它做過的一切都在 `kiwano` 之下,有兩個
值得知道的差異:

1. **計費詞彙以應用程式為準。** `--billing plan|payg|unl` 是正規寫法;
   `subscription|metered|unlimited` 仍可解析為別名。對**方案**供應商,
   `--limit`、`--unit` 與 `--reset` 現在會被拒絕——方案的配額來自它的
   plan query,而那些旗標過去會寫入應用程式從不讀取的欄位。

2. **`--json` 輸出是 stdout 上的單一文件。** 診斷訊息移到 stderr,這修好了
   會變更狀態的命令:`kiwano-cli --json providers add … | jq` 以前無法解析,
   因為重載提示被印在 JSON 之後的 stdout 上。

搬移的子命令:

| 原本 | 現在 |
| --- | --- |
| `kiwano-cli status` | `kiwano status` |
| `kiwano-cli providers use <ID> --agent A` | `kiwano providers use <ID> --agent A` (unchanged) |
| `kiwano-cli keys list\|add\|remove` | `kiwano keys list\|add\|remove` (unchanged) |
| `kiwano-cli usage --days N` | `kiwano usage --days N` (unchanged) |

結束碼是新的,但重要的那個精神上不變:閘道停止時,`status` 仍以 `1` 結束。
