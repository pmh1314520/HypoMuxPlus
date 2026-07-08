import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

/**
 * 比较点分数字版本号：`a` 是否严格大于 `b`（缺位段按 0 处理）。
 * 与后端 `version_gt` 采用同一逐段数值序，供前端更新判断复用。
 */
export function versionGt(a: string, b: string): boolean {
  const pa = a.split(".").map((x) => parseInt(x.trim(), 10) || 0);
  const pb = b.split(".").map((x) => parseInt(x.trim(), 10) || 0);
  const n = Math.max(pa.length, pb.length);
  for (let i = 0; i < n; i++) {
    const x = pa[i] ?? 0;
    const y = pb[i] ?? 0;
    if (x !== y) return x > y;
  }
  return false;
}

// 运行时读取应用版本（来源：tauri.conf.json），全局缓存避免重复请求
// cached 为空表示尚未取到；取到后全局复用。显示回退值与 tauri.conf.json 保持一致。
let cached = "";

/** 返回应用版本号（自动跟随 tauri.conf.json，无需各处硬编码） */
export function useAppVersion(): string {
  const [v, setV] = useState(cached || "1.2.0");
  useEffect(() => {
    if (cached) {
      setV(cached);
      return;
    }
    getVersion()
      .then((ver) => {
        cached = ver;
        setV(ver);
      })
      .catch(() => {});
  }, []);
  return v;
}
