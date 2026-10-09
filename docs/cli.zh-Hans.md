# `kiwano` 命令行

`kiwano` 让你不用桌面应用就能管理供应商、Agent 和路由。它读写的是应用和网关
共用的同一个 SQLite 存储,并通过网关的管理平面获取状态与热重载,所以在完全
没有显示器的机器上也能用。

它与应用共用同一份实现,而不是重新实现一遍:本文档里的每条命令都是对
`kiwano-core` 的一次调用,桌面应用链接的也是同一个 crate。在这里添加的供应商,
和在那里添加的是同一行,写法也相同。

## 安装

在 Linux 或 macOS 服务器上,一条命令就能做完所有事——二进制文件、校验和验证,
以及把网关注册为你的用户服务:

```sh
curl -fsSL https://hub.kiwano.cc/install.sh | sh
```

它不需要 root,这是刻意为之,而不是图方便:`agents takeover` 会改写 Agent 在它
自己 `$HOME` 下的配置,所以 CLI 和 Agent 必须是同一个账号。它一步一步做了什么,
以及全机器范围的替代做法,见
[`packaging/INSTALL.md`](../packaging/INSTALL.md)。

## 全局选项

| 标志 | 含义 |
| --- | --- |
| `--db <PATH>` | SQLite 文件。默认 `~/.kiwano/kiwano.db`,或 `KIWANO_DB_PATH` |
| `--admin-socket <TARGET>` | 网关管理平面的位置:unix 上是 socket 路径,Windows 上是 pipe 名称。默认:数据库旁边的 `admin.sock`,或 `KIWANO_ADMIN_SOCKET` |
| `--json` | stdout 上的机器可读输出 |
| `--quiet` | 抑制信息性提示 |
| `--data-port <PORT>` | 接管 Agent 时,其配置被指向的端口。默认 8317,或 `KIWANO_DATA_PORT` |
| `--home <PATH>` | Agent 配置文件所在的根目录。默认 `$HOME` |

全局选项与位置无关:`kiwano --json providers list` 和
`kiwano providers list --json` 是同一条命令。

## 输出与退出码

**stdout 只承载有效载荷,别无其他。** 在 `--json` 下,那正好是一份 JSON 文档,
所以 `kiwano --json providers list | jq` 能用。诊断信息——路由重载后的提示、
警告、错误——一律走 stderr,*包括*在 `--json` 之下。

列表以表格输出,数字列(请求数、token 数、成本、延迟)右对齐,方便按数量级
阅读:

```
+-------------------------+----------+--------+------------------+---------+---------------------------+
| ID                      | NAME     | PROTO  | ENDPOINT         | BILLING | AGENTS                    |
+-------------------------+----------+--------+------------------+---------+---------------------------+
| api-deepseek-com-271eb4 | DeepSeek | openai | api.deepseek.com | payg    | codex*,demo-route-00b40a* |
+-------------------------+----------+--------+------------------+---------+---------------------------+
```

表格会按照它输出到的终端来排版:最宽的列先让出空间,最多缩到六个字符的下限,
放不下的单元格用 `…` 省略。当输出不是终端——管道、文件——没有宽度可适配,
表格就完整写出;可以自己用 `COLUMNS` 指定宽度(`COLUMNS=100 kiwano providers
list`),脚本也能借此刻意取得窄版布局。

错误**不会**以 JSON 对象的形式输出到 stdout。放一个进去,会让部分成功的管道
无从判断它读的是哪份文档,而退出码已经给出了答案:

| 代码 | 含义 |
| --- | --- |
| `0` | 成功 |
| `1` | 答案是「否」——网关已停止时的 `status`。这不算失败:存储读取正常,只是没有网关可报告 |
| `2` | 用法或校验错误(clap 自己的解析失败也用这个) |
| `3` | 运行时错误:存储、IO、管理平面、网络 |

## 命令参考

完整的标志列表请运行 `kiwano <command> --help`。下面的地图是轮廓,不是每个
标志。

### 状态

```
kiwano status              gateway, store, today's totals, and the blocked list
kiwano gateway start       start one, adopting an already-running gateway
kiwano gateway stop        ask the running gateway to stop (graceful: it checkpoints its WAL)
kiwano gateway restart     stop and start
```

`status` 会列出网关当前**拒绝路由到**的供应商及其原因——
`capped-1  1.00 of 1.00 requests this period`。那是派生状态,只在网关运行时
存在,所以这是 shell 唯一能看到它的地方,通常也就是「为什么什么都没在路由」
的答案。

在 systemd 主机上,请用 `systemctl restart kiwanod` 代替 `gateway` 子命令——
见 INSTALL.md。

### 供应商

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

`add` 和 `edit` 也都接受转发与配额选项:

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

`edit` 另外接受 `--no-headers` 和 `--clear-plan-query`。

`providers edit` 会保留供应商的 **id**,这正是它存在的理由:绑定、轮换密钥和
用量行都引用它,所以「先删后加」是另一回事。只有你给出的标志会被修改;其余
一切都会从已存储的那一行沿用过来。

这套沿用对 `--timeout`、`--retries`、`--header`、`--endpoint-extra` 和
`--plan-limit-*` 至关重要:每一个都是底层 API 会整体重算的对象的*一部分*,所以
只指定其中一个的编辑,仍会把其余的一起送出。重命名供应商不会清掉它的
timeout。

清除一律要明说——`--no-headers`、`--clear-plan-query`——因为没给的标志意味着
保留。`--key` 同理:省略它会保留已存储的密钥,而不是把它清空。

除非供应商是套餐,否则 `--plan-limit-*` 会被拒绝(否则底层会默默丢弃这个值,
而一个什么都不做的标志比一个会拒绝的标志更糟)。`--plan-query` 才是
`providers quota` 能问到任何东西的前提。

`--unit` 说明 `--limit` 计数的是什么:`requests`、`wan_tokens`,或 Hub 公布了
汇率的某种货币。最后一种只做检查而不存储,遇到不认识的就以退出码 2 结束,并
列出它认识的货币:限额要用来与以其他货币计价的成本比较,没有汇率的限额会被
按 1:1 并入其中。从未同步过的机器完全没有汇率表,只接受 `USD` 和 `CNY`——
Hub 公布汇率所参照的两种货币。

### 轮换密钥

```
kiwano keys list <PROVIDER_ID>            masked; the key is never printed in full
kiwano keys add <PROVIDER_ID> --key K [--label L]
kiwano keys remove <KEY_ID>
```

轮换密钥会在供应商的主密钥之后尝试,按每个请求逐一进行。

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

`takeover` 是让服务器变得可用的命令。它会导入 Agent 当前配置的供应商、签发该
Agent 的占位密钥,并改写 Agent 自己的配置以指向网关。密钥很关键:数据平面只
路由它自己签发的密钥,其他的一律以 401 拒绝,所以手动把 Agent 指向网关是行不
通的。

它会打印占位密钥,因为那是结果中唯一从外部看不见的部分。

`takeover` 写入 `--home` 之下,所以请以你要改其配置的那个用户身份运行——或者
为服务账号显式传入 `--home`。

`agents add` 是取得路由的另一条路:一个 Kiwano 检测不到、也不会为它改写任何
东西的 Agent。你会得到一个名称、一个 id、一个密钥,以及一个可绑定供应商的
策略——磁盘上什么都不会变,所以请用打印出的密钥把客户端指向网关。
`agents list` 对还没有密钥的 Agent 会打印 `-`,而 `--json` 会带出该行上的
每一个字段。

`--protocol` 记录该 Agent 的客户端说的是什么:`anthropic`、`openai` 或
`gemini`。省略它意味着该 Agent 不作表态,这是与三者都不同的另一个答案,
`agents list` 会显示为 `-`。它是一个**标签**——没有任何路由、转换或校验依赖
它;请求的线上格式由它到达的路径决定。

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

`routes reorder` 以参数决定候选顺序:你写下的顺序就成为优先级 0、1、2……

配额载荷由网关自己的解析器校验,所以 CLI 写出的配置不会是引擎会默默重新解释
的那种。在不理会 `--limit` 的策略上加 `--limit`,会被拒绝,而不是当作无操作
接受。

### 用量与日志

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

`--from` 为包含、`--to` 为排除,与存储的半开区间一致。

`insights` 是一页式的报告,说明 token *如何*被花掉:每个 Agent 的记分卡(缓存
命中率、会话上下文增长、推理占比、重试)、标上 `cache` / `bloat` / `retry` /
`overhead` 的规则发现,以及上下文增长最多的那些会话。每一项发现都带着它背后
的 `request_logs` id——用 `kiwano logs show <ID>` 重新打开其中一条——而且全部
都在本地从日志计算:请求体会被抽样以测量固定的 tools/system 载荷,但它们从不
离开这台机器,任何地方都不会出现金额(成本是仪表盘的事)。全局 `--json` 标志
会输出同一份报告供机器使用。

`alerts` 默认**只读**。它原本会写入的去重键,正是桌面应用在发出通知前查询的
那一个,所以脚本化的轮询会默默吞掉你正在等的那条告警。如果你确实想让 CLI
接手投递,请传入 `--mark-notified`。

`logs clear` 需要 `--yes`:应用用对话框确认,而对没有 TTY 的 shell 来说,标志
就是对等的做法。

### 设置、配置与目录

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

`settings set` 的值只要能解析成 JSON 就会按 JSON 解析,所以
`--key cost_alert=false` 是布尔值,`--key log_retention_days=30` 是数字。

`config export --include-keys` 会写出正在使用的凭证,因此该文件以仅限属主
(`0600`)的方式创建。不带该标志的 `config export` 可以放心放进版本控制;两者
都能经由 `config import` 往返,导入时按名称和端点合并,而不是覆盖。

`import cc-switch` 读取 `~/.cc-switch/cc-switch.db`(v3.20+ 的 SQLite,以只读
方式打开)或较旧的 `~/.cc-switch/config.json`,并且只写入 Kiwano 自己的数据
库。`~/.cc-switch` 下的任何东西都不会被修改或删除,所以在做决定期间,两者可以
并存。

它把它认识的全部八个应用——claude、claude-desktop、codex、grokbuild、
opencode、openclaw、hermes 和 pi——映射到同名的 Agent,而 cc-switch 里*当前*
的那个供应商会成为该 Agent 的主绑定,所以迁移过来的是你实际在用的配置,而不是
一份要你重新挑选的候选列表。

重复运行是安全的,在已经用着的 Kiwano 旁边运行也安全。如果这里已经有相同端点
和协议的供应商——你手动添加的,或之前某次导入的——会被*复用*而不是再添加一条,
而且它的名称、密钥和计费都不动:导入只补上原本没有的东西。已经有路由的 Agent
会留在原路由上;只有还没有路由的 Agent 会被导入,所以重跑绝不会把 Agent 搬回
你后来已经切走的东西上。

有两种情况会被报告出来,而不是擅自猜测。cc-switch 的 `gemini` 行会被跳过:它们
的目标是 Gemini CLI,其设置是 Gemini 的形状,而本网关没有可将它们导入的协议。
没有 base URL 或没有密钥的供应商会被列为跳过,而不是创建成空的。每一次跳过都
附上原因。

`catalog list` 只读 Hub 缓存,别的什么都不读——没有内置副本,所以从未同步过的
机器列出来的是空的,而不是过期的目录。请先运行 `catalog sync`;全新的服务器在
从货架添加任何东西之前就该这么做,之后缓存便能像其他缓存一样离线供应。

**没有 `catalog add`。** 货架上的一键「添加」会打开用该条目预填的供应商表单——
它不调用自己的 API,所以这项能力就是 `providers add`,而值来自
`catalog list --json`:

```sh
kiwano catalog list --json --search deepseek
kiwano providers add --name DeepSeek --endpoint https://api.deepseek.com/anthropic \
    --protocol anthropic --billing payg --limit 50 --unit CNY --bind claude
```

预填规则(`payg` 限额 50,以及某些目录条目自带的 Agent 预绑定)存在于应用的
表单里,不在共用的层里。如果那套映射日后移进 `kiwano-core`,这就会变成一条命令
的捷径——在那之前,它是 CLI 没有的一项便利,而不是它欠缺的能力。

CLI *确实*会记录的是供应商对应哪个条目,因为那决定了它的请求要按该条目自己
公布的价格计价。这是从端点推断出来的,而不是询问得来的:`providers add` 在端点
恰好对应一个条目时链接该供应商,`catalog sync` 也对还没有链接的供应商做同样的
事。不做任何猜测——匹配不到任何条目、或匹配到多个条目(两个条目共用一个主机)
的端点会保持未链接,并按一般价格计价。

## 与应用的对应关系

桌面应用里每一项面向用户的能力在这里都能到达,除了本质上是图形的那几项(托盘、
窗口、主题、通知、i18n、应用内自我更新)。

| 应用能力 | 命令 |
| --- | --- |
| 网关状态 + 今日总计 | `status` |
| 供应商列表 / 添加 / 编辑 / 删除 | `providers list\|add\|edit\|remove` |
| 把供应商设为当前 | `providers enable`, `providers use` |
| 延迟 / 端点 / 模型探测 | `providers probe …` |
| 套餐配额环 | `providers quota` |
| 轮换 API 密钥 | `keys list\|add\|remove` |
| Agent 路由 + 策略 | `routes list`, `routes strategy` |
| 候选顺序、权重、时间窗 | `routes reorder`, `routes binding …` |
| 在 Agent 之间复制路由 | `routes apply` |
| Agent 检测与版本 | `agents detect`, `agents versions` |
| 接管 / 还原 | `agents takeover`, `agents restore` |
| 你自己的 Agent(具名路由) | `agents list\|add\|remove` |
| 用量总计 | `usage` |
| 仪表盘趋势 | `dashboard` |
| 用量告警 | `alerts` |
| 请求日志、明细、CSV 导出、清除 | `logs …` |
| 日志文件位置 | `logs dir` |
| 设置 | `settings get\|set` |
| 配置导出 / 导入 | `config export\|import` |
| cc-switch 迁移 | `import cc-switch` |
| 目录货架 + Hub 同步 + 货币 | `catalog list\|sync\|currency` |
| 守护进程生命周期 | `gateway start\|stop\|restart` |

完全没有涵盖的:更新检查、更新下载,以及进度事件流。那些是桌面应用安装程序的
属性,而服务器套件是通过安装一个新的来更新。

## 从 `kiwano-cli` 迁移

`kiwano-cli` 已经移除,也没有别名。它做过的一切都在 `kiwano` 之下,有两个值得
知道的差异:

1. **计费词汇以应用为准。** `--billing plan|payg|unl` 是规范写法;
   `subscription|metered|unlimited` 仍能解析为别名。对**套餐**供应商,
   `--limit`、`--unit` 和 `--reset` 现在会被拒绝——套餐的配额来自它的
   plan query,而那些标志过去会写入应用从不读取的列。

2. **`--json` 输出是 stdout 上的单份文档。** 诊断信息移到了 stderr,这修好了
   会改变状态的命令:`kiwano-cli --json providers add … | jq` 以前无法解析,
   因为重载提示被打印在 JSON 之后的 stdout 上。

搬迁的子命令:

| 原本 | 现在 |
| --- | --- |
| `kiwano-cli status` | `kiwano status` |
| `kiwano-cli providers use <ID> --agent A` | `kiwano providers use <ID> --agent A` (unchanged) |
| `kiwano-cli keys list\|add\|remove` | `kiwano keys list\|add\|remove` (unchanged) |
| `kiwano-cli usage --days N` | `kiwano usage --days N` (unchanged) |

退出码是新的,但重要的那个在精神上没变:网关停止时,`status` 仍以 `1` 退出。
