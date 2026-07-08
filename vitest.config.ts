import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// 前端测试基建：使用 jsdom 环境以支持依赖 DOM 的组件/工具测试，
// 仅收集 src 下的 *.test.ts / *.test.tsx 文件（属性测试与示例测试统一放置于源码旁）。
export default defineConfig({
  plugins: [react()],
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts?(x)"],
    globals: true,
  },
});
