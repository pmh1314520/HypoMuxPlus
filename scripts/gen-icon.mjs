// 将 appicon.svg 栅格化为 1024x1024 PNG，供 `tauri icon` 生成全套应用图标。
import sharp from "sharp";
import { readFileSync } from "node:fs";

const svg = readFileSync(new URL("../appicon.svg", import.meta.url));
await sharp(svg, { density: 384 })
  .resize(1024, 1024)
  .png()
  .toFile(new URL("../appicon.png", import.meta.url).pathname.replace(/^\//, ""));

console.log("appicon.png 生成完成 (1024x1024)");
