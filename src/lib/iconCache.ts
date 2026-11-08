/// 应用图标按需缓存（apps 段图标传输改造，2026-11-09 P2）：后端搜索条目只带
/// iconKey 不带 base64，前端按键向 app_icons 命令批量拉取。会话级 Map 命中后
/// 击键路径零图标流量；缺图标键不缓存负值——后台 backfill 补齐后下次搜索/
/// 拉取自然可见，in-flight 去重防击键连发堆叠请求
import { invoke } from "@tauri-apps/api/core";

const cache = new Map<string, string>();
const pending = new Set<string>();

/// 读缓存（未命中返回 undefined，渲染退回文字角标）
export const cachedIcon = (key: string): string | undefined => cache.get(key);

/// 确保键集已请求（内部对已缓存与在途去重）；拉取完成回调 onResolved 触发
/// 重渲。失败静默：不写缓存，键下次结果集出现时仍会重试
export function ensureIcons(keys: string[], onResolved: () => void): void {
  const missing = [...new Set(keys)].filter(
    (k) => !cache.has(k) && !pending.has(k)
  );
  if (missing.length === 0) return;
  for (const k of missing) pending.add(k);
  void invoke<Record<string, string>>("app_icons", { keys: missing })
    .then((map) => {
      for (const [k, v] of Object.entries(map)) cache.set(k, v);
    })
    .catch(() => {})
    .finally(() => {
      for (const k of missing) pending.delete(k);
      onResolved();
    });
}
