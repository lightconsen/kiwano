// 落地页的语言清单。加一门语言 = 在下面加一条 + 加一个同名的字典文件
// (`./<locale>.ts`),别处不用动 —— main.ts 的应用、切换器的菜单、演示的语言
// 握手都从这张表读。
//
// 英文没有字典:模板 index.html 本身就是英文,每个元素的内容在运行时快照进
// data-en。所以一门语言的字典是一层覆盖,少一个键就是那一处保持英文。
import { ja } from "./ja";
import { zhHans } from "./zh-Hans";
import { zhHant } from "./zh-Hant";

export type Locale = "en" | "zh-Hans" | "zh-Hant" | "ja";

export interface LocaleMeta {
  /** `<html lang>` 的值。也是发给演示的应用侧 locale id —— 演示会拿它去认自己
      的语言,认不出就保持原样,所以这里的值必须和应用 `LANGUAGE_PREFS` 里的一致。 */
  tag: string;
  /** 浏览器标签与分享卡片上的标题。 */
  title: string;
  /** 语言名。用**该语言自己的写法** —— 找自己语言的人应当认得它,不论界面此刻
      是哪一门。这也是切换器菜单里唯一的文字,所以它同时是按钮的可读名字。 */
  label: string;
  /** 这门语言的文档根。没有翻译文档的语言指回 /docs/(英文),而不是一个 404。 */
  docs: string;
  /** 覆盖层;英文没有(见文件头)。 */
  dict?: Record<string, string>;
}

/** 顺序即菜单里的顺序,`DEFAULT_LOCALE` 是模板原文那一门。
 *
 * 没有国旗字段:语言不是国家(English 挂哪面旗?繁體中文 是台湾还是香港?),
 * 而四门语言之后,菜单里写各自的语言名比四张小旗子更清楚 —— 这也正是应用里
 * 语言选择器的做法。 */
export const LOCALES: Record<Locale, LocaleMeta> = {
  en: {
    tag: "en",
    title: "Kiwano — Local-first AI Provider Manager",
    label: "English",
    docs: "/docs/",
  },
  "zh-Hans": {
    tag: "zh-Hans",
    title: "Kiwano — 本地优先的 AI Provider 管理器",
    label: "简体中文",
    docs: "/docs/zh-Hans/",
    dict: zhHans,
  },
  "zh-Hant": {
    tag: "zh-Hant",
    title: "Kiwano — 本機優先的 AI Provider 管理器",
    label: "繁體中文",
    docs: "/docs/zh-Hant/",
    dict: zhHant,
  },
  ja: {
    tag: "ja",
    title: "Kiwano — ローカル優先の AI Provider マネージャー",
    label: "日本語",
    docs: "/docs/ja/",
    dict: ja,
  },
};

export const DEFAULT_LOCALE: Locale = "en";
export const LOCALE_IDS = Object.keys(LOCALES) as Locale[];
