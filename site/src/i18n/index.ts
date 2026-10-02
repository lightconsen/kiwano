// 落地页的语言清单。加一门语言 = 在下面加一条 + 加一个同名的字典文件
// (`./<locale>.ts`),别处不用动 —— main.ts 的应用、切换器、演示的语言握手都从
// 这张表读。
//
// 英文没有字典:模板 index.html 本身就是英文,每个元素的内容在运行时快照进
// data-en。所以一门语言的字典是一层覆盖,少一个键就是那一处保持英文。
import { zhHans } from "./zh-Hans";

export type Locale = "en" | "zh-Hans";

export interface LocaleMeta {
  /** `<html lang>` 的值。也是发给演示的应用侧 locale id —— 演示会拿它去认自己
      的语言,认不出就保持原样,所以这里的值必须和应用 `LANGUAGE_PREFS` 里的一致。 */
  tag: string;
  /** 浏览器标签与分享卡片上的标题。 */
  title: string;
  /** 切换器里显示的名字。用**该语言自己的写法** —— 找自己语言的人应当认得它,
      不论界面此刻是哪一门。 */
  label: string;
  /** 切换器上的图标。 */
  flag: string;
  /** 这门语言的文档根。没有翻译文档的语言指回 /docs/(英文),而不是一个 404。 */
  docs: string;
  /** 覆盖层;英文没有(见文件头)。 */
  dict?: Record<string, string>;
}

/** 顺序即切换器里的顺序,`DEFAULT_LOCALE` 是模板原文那一门。 */
export const LOCALES: Record<Locale, LocaleMeta> = {
  en: {
    tag: "en",
    title: "Kiwano — Local-first AI Provider Manager",
    label: "English",
    flag: "#i-flag-gb",
    docs: "/docs/",
  },
  "zh-Hans": {
    tag: "zh-Hans",
    title: "Kiwano — 本地优先的 AI Provider 管理器",
    label: "简体中文",
    flag: "#i-flag-cn",
    docs: "/docs/zh-Hans/",
    dict: zhHans,
  },
};

export const DEFAULT_LOCALE: Locale = "en";
export const LOCALE_IDS = Object.keys(LOCALES) as Locale[];
