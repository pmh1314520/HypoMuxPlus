import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

// 运行时读取应用版本（来源：tauri.conf.json），全局缓存避免重复请求
let cached = "1.0.0";

/** 返回应用版本号（如 "1.0.0"），自动跟随 tauri.conf.json，无需各处硬编码 */
export function useAppVersion(): string {
  const [v, setV] = useState(cached);
  useEffect(() => {
    if (cached !== "1.0.0") {
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
