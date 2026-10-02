# Agent 接管

接管是讓 Agent 走本機閘道的方式:Kiwano 改寫 Agent **自己**在其 `$HOME`
下的設定檔,改寫前先備份,關閉接管時按位元組還原。不注入代理二進位檔,不
替你匯出環境變數——Agent 照常讀自己的設定,發現閘道就在那裡。

本頁是按 Agent 的參考:每個接管寫哪些檔案、用什麼模式、哪些行為值得知道。
Agent 集合與這些路徑由一致性測試釘住(docs 表格 ↔ 內建登錄表),本頁不會
與某個建置的行為悄悄漂移。

## 三種模式

- **Additive(增補)**——Agent 設定裡是供應商清單;接管 upsert 一條
  `kiwano-gateway` 條目並選中它。你自己的供應商原樣保留。
- **Exclusive(獨佔)**——設定就是一個供應商槽位;接管直接改寫。還原時
  把你的位元組放回去。
- **取代選中的槽位**(僅 Cline)——其選擇器指向一個供應商 id,接管改寫的
  是該 id 指向的槽位,而不是在旁邊加第二條。

## Agent 一覽

| Agent | 被改寫的設定檔 | 模式 | 值得知道 |
| --- | --- | --- | --- |
| `claude` | `~/.claude/settings.json` | 獨佔 | `CLAUDE_CONFIG_DIR` 會搬動整個 profile。 |
| `codex` | `~/.codex/config.toml` + `auth.json` | 獨佔 | 認 `CODEX_HOME`。兩個檔案一起改寫;`config.toml` 缺失時拒絕——先跑一次 Codex。 |
| `gemini` | `~/.gemini/.env` | 獨佔 | 檔案缺失時會建立;其中的註解保留。 |
| `grokbuild` | `~/.grok/config.toml` | 獨佔 | 拒絕官方 xAI 登入的設定——先設好自訂模型。 |
| `claude-desktop` | 兩份 `claude_desktop_config.json` + configLibrary profile + `_meta.json` | 獨佔 | 僅 macOS;把部署模式切到 3p 並寫入閘道 profile。 |
| `opencode` | `~/.config/opencode/opencode.json` | 增補 | 走 XDG 規則(`XDG_CONFIG_HOME`)。 |
| `openclaw` | `~/.openclaw/openclaw.json` + `agents/<id>/agent/models.json` | 增補 | 目錄檔承載主設定驗證器不接受的工作階段親和旗標。 |
| `hermes` | `~/.hermes/config.yaml` | 增補 | 認 `HERMES_HOME`。 |
| `pi` | `~/.pi/agent/models.json` + `settings.json` | 增補 | 選擇寫在 settings 裡。 |
| `omp` | `~/.omp/agent/config.yml` + `models.yml` | 增補 | `.yml` 與 `.yaml` 兩種拼法都認,逐檔偵測。供應商條目宣告角色選擇裡命名的模型,並帶 `authHeader: true`——沒有它 omp 會解析出佔位 Key 卻不送出。當存在舊版 `models.json` 且沒有 YAML 時拒絕:先寫 YAML 會讓 omp 永遠不再移轉它。YAML 會重新序列化(關閉接管時逐位元組還原)。 |
| `dsh` | `$DSH_HOME/profiles/*/cordis.patch.yml` + `$DSH_HOME/.env` | 增補 | 認 `DSH_HOME`(預設 `~/.dsh`)。改寫 `llm-deepseek` 那一列的 `baseURL` 與 `apiKeyEnv`——Key 本身寫在 `.env` 裡——其餘列一律不動,使用者選定的模型仍是他的。兩種情形拒絕:機器上只有 dsh < 0.1.5 的 `config.yaml`(更新後先跑一次 dsh);dsh 自己的 `settings.yaml` 為那一列釘了端點或金鑰——dsh 解析時以 settings 覆蓋補丁清單,那樣接管會什麼都路由不到。接管之後新建的 profile 不會被補,重跑一次接管即可。 |
| `hanaagent` | `$HANA_HOME/provider-catalog.json` + `$HANA_HOME/agents/*/config.yaml` | 增補 | 認 `HANA_HOME`(預設 `~/.hanako`)。目錄條目承載端點,並宣告各 agent 選用的模型 id;每個 agent 的 `api.provider` 指向它,模型選擇不動。所有 agent(人格)都會被改寫,不會有誰還留在真實上游。HanaAgent 跑過一次之前拒絕——目錄從那時才存在,而且不能搶在它自己從 `added-models.yaml` 移轉之前。目錄裡 `deletedProviders` 名單上的本 id 會被摘掉,否則應用程式會把這條藏起來。 |
| `commandcode` | `~/.commandcode/settings.json` + `providers.json` + `kiwano-gateway.key` | 增補 | Key 是 `!` 命令參考,讀取 Kiwano 自己的 Key 檔——Command Code 拒收直接貼上的明文密鑰,而無 Key 的條目會被閘道以 401 回覆。即使走閘道,Command Code 仍要求它自己的登入(`cmd login`)。 |
| `workbuddy` | `~/.workbuddy/models.json` | 增補 | 認 `WORKBUDDY_CONFIG_DIR`。 |
| `codebuddy` | `~/.codebuddy/models.json` | 增補 | 認 `CODEBUDDY_CONFIG_DIR`。 |
| `kimi` | `~/.kimi-code/config.toml` | 增補 | 認 `KIMI_CODE_HOME`;後繼版本缺席時讀舊版 `~/.kimi`。 |
| `qwen` | `~/.qwen/settings.json` | 增補 | 認 `QWEN_HOME`。 |
| `cline` | `~/.cline/data/settings/providers.json` | 取代選中的槽位 | 依序認 `CLINE_PROVIDER_SETTINGS_PATH` / `CLINE_DATA_DIR` / `CLINE_DIR`。 |
| `mimo` | `~/.config/mimocode/mimocode.jsonc` | 增補 | 走 XDG 規則。 |
| `mcode` | `~/.minimax/config.yaml` | 增補 | 認 `MCODE_CONFIG_DIR`。 |
| `aider` | `~/.aider.conf.yml` | 獨佔 | 三個全域欄位,不是供應商清單。模型加 `openai/` 前綴強制走 LiteLLM 的 OpenAI 相容路由;模型 id 本身原樣到達上游。`AIDER_CONFIG` 刻意不認。 |
| `continue` | `~/.continue/config.yaml` | 增補 | 閘道模型插到 `models` 開頭並限定 `roles: [chat]`——Continue 說明文件所載的預設是「該角色的第一個模型」。 |
| `crush` | `~/.config/crush/crush.json` | 增補 | 填它自己 `model large` 命令持久化的 `models.large` 槽;`small` 槽不動。 |
| `droid` | `~/.factory/settings.json` | 增補 | 在 `customModels` 開頭插入條目並設預設 `model`。組織政策 `allowCustomModels: false` 時拒絕接管。 |
| `goose` | `<goose 設定根>/config.yaml` + `custom_providers/kiwano-gateway.json` + `kiwano-gateway.key` | 增補 | macOS 根目錄是 `~/Library/Application Support/Block/goose`(不是 `~/.config/goose`);Linux 走 XDG。Key 透過供應商的 `auth.command`(`/bin/cat` Key 檔)送達,因為 goose 的鑰匙圈密鑰儲存不是檔案可及的。首次匯入:無——Key 就在鑰匙圈裡。 |
| `zcode` | `~/.zcode/v2/provider_config.json` | 增補 | ZCode CLI 與 Desktop 共享的原生登錄表——接管同時影響兩端。認 `ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`;不認 `ZCODE_DATA_BASE_DIR`。 |

## 環境變數

接管透過使用者的**登入 shell** 環境解析路徑(GUI 應用程式本來也看不見 rc
檔匯出的東西)。變數被設成*相對*路徑時直接拒絕,而不是猜。

認: `CLAUDE_CONFIG_DIR`、`CODEX_HOME`、`XDG_CONFIG_HOME`(opencode、
mimo、crush;Linux 上的 goose)、`HERMES_HOME`、`WORKBUDDY_CONFIG_DIR`、
`CODEBUDDY_CONFIG_DIR`、`QWEN_HOME`、`KIMI_CODE_HOME`(及舊版
`KIMI_SHARE_DIR`)、`CLINE_PROVIDER_SETTINGS_PATH`、`CLINE_DATA_DIR`、
`CLINE_DIR`、`MCODE_CONFIG_DIR`、`ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`。

刻意不認: `AIDER_CONFIG`(aider v1.24 載入時不讀它)、`GOOSE_PATH_ROOT`
和 `ZCODE_DATA_BASE_DIR`(搬整棵樹會把接管要一起寫的檔案拆散)、
`OPENCODE_CONFIG` / `OPENCODE_CONFIG_DIR` 和 OpenClaw 的狀態覆寫(它們
不會搬走接管要寫的那個檔案)。

## 首次接管的匯入

Agent 已設定過自訂端點時,開啟接管會提議匯入:同一 base URL 和 Key 變成
一條綁定到該 Agent 的供應商,第一天路由的還是同一個上游。匯入永不把
loopback 端點當成上游(那是接管後的閘道自己),也讀不出不存在的明文
Key——官方訂閱登入(Claude OAuth、Codex ChatGPT)和 goose 的鑰匙圈什麼
都取不出來,引導改為手動輸入。

## 還原語意

關閉接管:備份的原件按位元組寫回(接管建立的檔案被刪除)。備份遺失——
或備份是在設定本已指向我們時抓的——則用閘道為該 Agent 服務的供應商重建
路由;再不行,就把我們的路由從現行設定裡**剝離**,保證 Agent 不會被留在
指向無人監聽的 loopback 連接埠。剝離是外科手術式的:只刪屬於我們的
列/值。
