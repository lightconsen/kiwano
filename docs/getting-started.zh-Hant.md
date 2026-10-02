# 快速上手

Kiwano 位於你的編碼 Agent 與 AI 供應商之間:供應商註冊一次,本機閘道跑在
`127.0.0.1`,所有 Agent 都指向這一個連接埠。本頁從安裝帶到第一條被計量的
請求。Key 永遠不離開你的機器。

## 你需要準備

- 一個供應商 API Key(OpenAI 相容或 Anthropic 相容端點)。
- 一個或多個已安裝的編碼 Agent——Claude Code、Codex、Gemini CLI……
  (全部 26 個內建 Agent 見 [Agent 接管](agent-takeover.zh-Hant.md))。

## 1. 安裝並啟動

從 [Releases](https://github.com/lightconsen/kiwano/releases/latest) 下載
對應平台的簽章安裝檔(macOS / Windows / Linux);若你用 Homebrew,也可以
`brew install`。首次啟動會拉起 **常駐程式**——本機閘道,它的生命週期獨立於
GUI 視窗:關掉 App,閘道仍在服務。系統匣圖示顯示它正在運作。

## 2. 註冊供應商

在 **Models** 貨架(Hub 目錄)裡搜尋供應商——DeepSeek、Kimi、智譜 GLM、
OpenRouter……——或在 **Apps** 畫面手動新增:

![Models 貨架:Hub 目錄,按類別歸檔,含協定、計費與價格](screenshots/shelf.webp)

*Models 貨架——Hub 目錄:官方、聚合、第三方、免費四類供應商,計費與價格透明。*

1. **Add provider** → 名稱、端點、API Key,以及端點使用的協定(`openai` 或
   `anthropic`)。
2. 可選:預設模型、計費類型(按量 / 方案 / 不限量)和花費限額。
3. 儲存。一旦有請求打到它,供應商那一列就會顯示健康狀態。

Key 存放於僅限目前使用者讀取的本機資料庫裡。除了你填寫的端點,資料不送往
任何地方。

## 3. 接管一個 Agent

**Apps** 畫面上每個受支援的 Agent 都是一張卡片:

![Apps 畫面:供應商清單,帶綁定的 Agent、用量與配額、狀態](screenshots/apps.webp)

*Apps 畫面——供應商與綁定的 Agent 一目了然;用量、配額與健康狀態就在列上。*

- 如果該 Agent 已設定過供應商,**Enable Kiwano** 會提議匯入它——上游在
  第一天就保持不變,只是改為經過閘道(從而受到計量)。
- 接管開關會改寫 Agent 自己的設定檔,把端點指向閘道。改寫前先備份,
  關閉接管時按位元組還原。Agent 的其它設定一概不動。

每個 Agent 具體改寫哪些檔案、哪些行為特殊,見
[Agent 接管](agent-takeover.zh-Hant.md)的表。

## 4. 驗證

透過 Agent 送出一條 prompt(`claude "hi"`,或你日常會下的任何 prompt),然後
打開 Kiwano 的 **Request logs**:應能看到這條請求,帶歸屬(哪個供應商服務
的)、token、延遲與估算成本。Dashboard 會把同樣的列按供應商、按 Agent、
按天聚合。

![Dashboard:統計磚、用量趨勢、按供應商與 Agent 歸因、請求日誌](screenshots/costs.webp)

*Dashboard——7 天趨勢按供應商與 Agent 歸因;下方日誌列可展開單次請求。*

## 5. 一個 Agent 路由多個供應商

一個 Agent 綁一個供應商是 `single` 策略。給 Agent 的路由多綁幾個候選,
再選一個策略——`failover`、`roundrobin`、`timewindow`、`quota` 或
`least-busy`——並設定按 Agent 的花費限額。見
[策略與限額](strategies.zh-Hant.md)。

## 資料都在哪

| 內容 | 位置 |
| --- | --- |
| 供應商、路由、用量、備份 | 本機 SQLite(僅目前使用者可讀),由常駐程式讀寫 |
| Agent 設定 | 各 Agent 自己在 `$HOME` 下的檔案——見接管表 |
| 請求日誌與內文 | 本機,保留期在設定裡可調 |

## 解除安裝 / 關閉

按 Agent 關閉接管(設定從備份還原),或刪除供應商(自動提升下一個候選)。
常駐程式可從系統匣停止;不會有 Agent 被留在指向死掉的連接埠的狀態——
被移除的接管會還原,備份遺失的接管會降級回 Agent 自己的預設設定,而不是
一個沒人監聽的 loopback 位址。
