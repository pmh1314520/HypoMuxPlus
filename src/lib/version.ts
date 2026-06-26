import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";

// 运行时读取应用版本（来源：tauri.conf.json），全局缓存避免重复请求
// cached 为空表示尚未取到；取到后全局复用。显示回退值与 tauri.conf.json 保持一致。
let cached = "";

/** 返回应用版本号（自动跟随 tauri.conf.json，无需各处硬编码） */
export function useAppVersion(): string {
  const [v, setV] = useState(cached || "1.1.0");
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
