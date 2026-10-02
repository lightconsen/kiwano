# エージェントの引き継ぎ

引き継ぎは、エージェントがローカルゲートウェイ経由でルーティングされるようにする
仕組みです。Kiwano は、エージェントの `$HOME` 配下にある**そのエージェント自身の**
設定ファイルを書き換えます。元のファイルは先にバックアップし、引き継ぎを切ると
バイト単位で復元します。プロキシのバイナリは注入せず、環境変数を代わりに export する
こともありません — エージェントは普段どおり自分の設定を読み、そこにゲートウェイが
あるのを見つけます。

本ページはエージェントごとのリファレンスです。各引き継ぎが書き込むファイル、その
モード、知っておくべき挙動をまとめます。エージェントの集合とこれらのパスは一貫性
テスト（docs の表 ↔ 組み込みレジストリ）で固定されているため、本ページがビルドの
挙動から黙ってずれることはありません。

## 3 つのモード

- **追加型** — エージェントの設定がプロバイダー一覧を持つ場合、引き継ぎは
  `kiwano-gateway` エントリを upsert して選択します。自分のプロバイダーはそのまま
  残ります。
- **排他型** — エージェントの設定がプロバイダーのスロット 1 つだけの場合、引き継ぎは
  それを上書きします。復元はあなたのバイトを書き戻します。
- **選択中スロットの置き換え**（Cline のみ） — 選択が指すのはプロバイダー id なので、
  引き継ぎは 2 つ目のエントリを足すのではなく、その id が指すスロットを書き換えます。

## エージェント一覧

| エージェント | 書き換える設定ファイル | モード | 知っておくべきこと |
| --- | --- | --- | --- |
| `claude` | `~/.claude/settings.json` | 排他型 | `CLAUDE_CONFIG_DIR` はプロファイル全体を移動させます。 |
| `codex` | `~/.codex/config.toml` + `auth.json` | 排他型 | `CODEX_HOME` を認識します。両ファイルをまとめて書き換えます。`config.toml` が無い場合は拒否します — 先に Codex を一度実行してください。 |
| `gemini` | `~/.gemini/.env` | 排他型 | ファイルが無ければ作成します。中のコメントは保持されます。 |
| `grokbuild` | `~/.grok/config.toml` | 排他型 | 公式 xAI ログインの設定は拒否します — 先にカスタムモデルを設定してください。 |
| `claude-desktop` | 2 つの `claude_desktop_config.json` + configLibrary プロファイル + `_meta.json` | 排他型 | macOS のみ。デプロイモードを 3p に切り替え、ゲートウェイのプロファイルを書き込みます。 |
| `opencode` | `~/.config/opencode/opencode.json` | 追加型 | XDG の規則に従います（`XDG_CONFIG_HOME`）。 |
| `openclaw` | `~/.openclaw/openclaw.json` + `agents/<id>/agent/models.json` | 追加型 | カタログファイルが、メイン設定のバリデータでは弾かれるセッション親和のフラグを宣言します。 |
| `hermes` | `~/.hermes/config.yaml` | 追加型 | `HERMES_HOME` を認識します。 |
| `pi` | `~/.pi/agent/models.json` + `settings.json` | 追加型 | 選択は settings ファイルが保持します。 |
| `omp` | `~/.omp/agent/config.yml` + `models.yml` | 追加型 | `.yml` と `.yaml` の両方の綴りを認識し、ファイルごとに探査します。プロバイダーエントリはロール選択が指すモデルを宣言し、`authHeader: true` を付けます — これが無いと omp はプレースホルダーキーを解決はするものの送信しません。旧来の `models.json` があって YAML が無い間は拒否します。先に YAML を書いてしまうと、omp がそれを二度と移行しなくなるためです。YAML は再シリアライズされます（無効化時にはコメントがバイト単位で復元されます）。 |
| `dsh` | `$DSH_HOME/profiles/*/cordis.patch.yml` + `$DSH_HOME/.env` | 追加型 | `DSH_HOME` を認識します（既定は `~/.dsh`）。`llm-deepseek` 行の `baseURL` と `apiKeyEnv` を書き換え（キー自体は `.env` に入ります）、他の行には触れないので、ユーザーが選んだモデルはそのまま残ります。マシンにまだ dsh < 0.1.5 の `config.yaml` しか無い場合は拒否します（更新後に dsh を一度実行してください）。また、dsh 自身の `settings.yaml` がその行のエンドポイントやキーを固定している場合も拒否します — dsh はパッチ一覧より settings を優先して解決するため、引き継ぎでは何もルーティングされません。引き継ぎ後に作成したプロファイルはパッチされません — 引き継ぎをやり直してください。 |
| `hanaagent` | `$HANA_HOME/provider-catalog.json` + `$HANA_HOME/agents/*/config.yaml` | 追加型 | `HANA_HOME` を認識します（既定は `~/.hanako`）。カタログのエントリがエンドポイントを持ち、各エージェントが選ぶモデル id を宣言します。各エージェントの `api.provider` はそこを指し、モデルはそのままです。すべてのエージェント（ペルソナ）を書き換えるので、どれも実際の上流には残りません。HanaAgent が一度動くまでは拒否します — カタログはそれ以降にしか存在せず、`added-models.yaml` からの自身の移行を追い越してはならないためです。カタログの `deletedProviders` に載っている id はそこから外されます。さもないとアプリがそのエントリを隠してしまいます。 |
| `commandcode` | `~/.commandcode/settings.json` + `providers.json` + `kiwano-gateway.key` | 追加型 | キーは Kiwano のキーファイルを読む `!` コマンド参照です — Command Code は貼り付けた生のシークレットを拒否し、キーの無いエントリは 401 で返されます。ゲートウェイ経由のモデルでも、Command Code は独自のサインイン（`cmd login`）を要求します。 |
| `workbuddy` | `~/.workbuddy/models.json` | 追加型 | `WORKBUDDY_CONFIG_DIR` を認識します。 |
| `codebuddy` | `~/.codebuddy/models.json` | 追加型 | `CODEBUDDY_CONFIG_DIR` を認識します。 |
| `kimi` | `~/.kimi-code/config.toml` | 追加型 | `KIMI_CODE_HOME` を認識します。後継版が無い場合は旧来の `~/.kimi` を読みます。 |
| `qwen` | `~/.qwen/settings.json` | 追加型 | `QWEN_HOME` を認識します。 |
| `cline` | `~/.cline/data/settings/providers.json` | 選択中スロットの置き換え | `CLINE_PROVIDER_SETTINGS_PATH` / `CLINE_DATA_DIR` / `CLINE_DIR` をこの順で認識します。 |
| `mimo` | `~/.config/mimocode/mimocode.jsonc` | 追加型 | XDG の規則に従います。 |
| `mcode` | `~/.minimax/config.yaml` | 追加型 | `MCODE_CONFIG_DIR` を認識します。 |
| `aider` | `~/.aider.conf.yml` | 排他型 | プロバイダー一覧ではなく 3 つのグローバルフィールドです。モデルには `openai/` を付けて LiteLLM の OpenAI 互換ルートを強制します。モデル id 自体はそのまま上流に届きます。`AIDER_CONFIG` は意図的に認識しません。 |
| `continue` | `~/.continue/config.yaml` | 追加型 | ゲートウェイのモデルは `roles: [chat]` を付けて `models` の先頭に挿入されます — Continue のドキュメント上の既定は「そのロールの最初のモデル」です。 |
| `crush` | `~/.config/crush/crush.json` | 追加型 | 自身の `model large` コマンドが永続化する `models.large` スロットを埋めます。`small` スロットには触れません。 |
| `droid` | `~/.factory/settings.json` | 追加型 | `customModels` の先頭にエントリを追加し、既定の `model` を設定します。組織ポリシーが `allowCustomModels: false` を設定している場合は拒否します。 |
| `goose` | `<goose config root>/config.yaml` + `custom_providers/kiwano-gateway.json` + `kiwano-gateway.key` | 追加型 | macOS のルートは `~/Library/Application Support/Block/goose` です（`~/.config/goose` ではありません）。Linux は XDG に従います。キーはプロバイダーの `auth.command`（キーファイルの `/bin/cat`）を通って goose に届きます。goose のキーチェーンベースのシークレットストアはファイルから読めないためです。初回引き継ぎの取り込み: なし — キーはキーチェーンにあります。 |
| `zcode` | `~/.zcode/v2/provider_config.json` | 追加型 | ZCode CLI と Desktop が共有するネイティブレジストリです — 引き継ぎは両方に影響します。`ZCODE_PERSONAL_PROVIDER_CONFIG_FILE` を認識し、`ZCODE_DATA_BASE_DIR` は認識しません。 |

## 環境変数

引き継ぎはユーザーの**ログインシェル**の環境を通してパスを解決します（GUI アプリは
rc ファイルが export した内容をそもそも見られません）。*相対*パスが設定されている
変数は、推測せず拒否します。

認識する: `CLAUDE_CONFIG_DIR`、`CODEX_HOME`、`XDG_CONFIG_HOME`（opencode、mimo、
crush、Linux の goose）、`HERMES_HOME`、`WORKBUDDY_CONFIG_DIR`、
`CODEBUDDY_CONFIG_DIR`、`QWEN_HOME`、`KIMI_CODE_HOME`（および旧来の
`KIMI_SHARE_DIR`）、`CLINE_PROVIDER_SETTINGS_PATH`、`CLINE_DATA_DIR`、
`CLINE_DIR`、`MCODE_CONFIG_DIR`、`ZCODE_PERSONAL_PROVIDER_CONFIG_FILE`。

意図的に認識しない: `AIDER_CONFIG`（aider v1.24 は読み込み時に読まない）、
`GOOSE_PATH_ROOT` と `ZCODE_DATA_BASE_DIR`（ツリー全体を移すと、引き継ぎが
まとめて書き込むファイルが分断される）、`OPENCODE_CONFIG` / `OPENCODE_CONFIG_DIR`
と OpenClaw の状態オーバーライド（引き継ぎが書き込むべきファイルを動かさない）。

## 初回引き継ぎの取り込み

エージェントにすでにカスタムエンドポイントが設定されている場合、引き継ぎを有効に
するとその取り込みを提案します。同じ base URL とキーが、そのエージェントに紐付いた
プロバイダーとして登録され、初日から同じ上流がルーティングされます。取り込みは
ループバックのエンドポイント（引き継ぎ後のゲートウェイ自身）を二度取り込むことは
なく、平文のキーが存在しない場所からは何も読み出しません — 公式プランのログイン
（Claude OAuth、Codex ChatGPT）と goose のキーチェーンからは何も取り出せないため、
オンボーディングガイドは手動入力を促します。

## 復元の意味

引き継ぎを切ると、バックアップした元のファイルがバイト単位で書き戻されます
（引き継ぎが作成したファイルは削除されます）。バックアップが失われている場合 —
またはバックアップが、設定がすでに Kiwano のものだった時点で取得されていた場合 — は、
ゲートウェイがそのエージェントに提供しているプロバイダーを基にルートを再構築します。
それもできない場合は、Kiwano のルートを稼働中の設定から取り除き、誰も listen して
いないループバックポートを指したままになるエージェントが生じないようにします。
取り除きは外科手術的で、Kiwano に属する行や値だけを削除します。
