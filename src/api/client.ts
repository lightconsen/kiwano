// UI 数据访问唯一入口。M0 阶段由 mock.ts 提供实现；
// 集成阶段（任务 #8）替换为 Tauri invoke 版本，组件零改动。
import type { KiwanoApi } from "./types";
import { mockApi } from "./mock";

export const api: KiwanoApi = mockApi;
