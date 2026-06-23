<div align="center">

# HypoMux Plus

**现代化多网卡带宽聚合下载加速客户端 · Modern Multi-NIC Bandwidth Aggregation Accelerator**

[![Tauri](https://img.shields.io/badge/Tauri-2.x-FFC131?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.90%2B-000000?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=white)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?style=flat-square&logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![TailwindCSS](https://img.shields.io/badge/TailwindCSS-4-06B6D4?style=flat-square&logo=tailwindcss&logoColor=white)](https://tailwindcss.com)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-0078D4?style=flat-square&logo=windows)](#)
[![License](https://img.shields.io/badge/License-AGPL--3.0-D22128?style=flat-square)](./LICENSE)

[简体中文](#简体中文) · [English](#english)

</div>

---

## 简体中文

HypoMux Plus 是一款面向 Windows 平台的**多网卡带宽并发聚合下载加速工具**的现代化桌面客户端。它在 [Hypostasis-Cat 的开源项目 HypoMux](https://github.com/Hypostasis-Cat/HypoMux) 的核心思想之上，使用 **Tauri + Rust + React + TailwindCSS** 完整重构，提供更美观、更流畅、更专业的桌面体验，并将分流引擎用 Rust（tokio）原生重写，产物为零运行时依赖的独立可执行文件。

> 本项目是基于原 HypoMux 的衍生作品，遵循其 **AGPL-3.0** 协议开源。原作者：Hypostasis-Cat；衍生开发者：**青云制作_彭明航**。

### 核心特性

- **双协议无感接管**：后台同时运行 SOCKS5 与 HTTP/HTTPS 转发服务，启动后自动写入 Windows WinINet 系统代理，兼容 Steam、IDM、浏览器等遵循系统代理规范的客户端。
- **L3 物理层网卡绑定**：对每条出站连接执行 `setsockopt(IP_UNICAST_IF)` 接口索引强绑定 + 源地址 bind，把流量物理钉死在指定网卡上，根治同网段多网卡的 `WinError 10049` 错网卡问题。
- **Round-Robin 连接调度**：在用户勾选的网卡集合内轮询分发连接，将多线程下载的带宽叠加到多张物理网卡。
- **全生命周期代理保护**：手动停止、启动失败、窗口关闭、进程退出等所有路径都强制还原系统代理，降低代理残留导致断网的风险。
- **实时遥测大屏**：基于内核计数器（`GetIfEntry2`）的逐秒采样，展示合并下行总速度、实时波形、各网卡速度与活跃连接数。
- **现代化界面**：深色 / 浅色双主题、玻璃拟态、流畅动效、完整中英双语，矢量图标全程无 Emoji。
- **稳定性增强**：加速期间自动关闭死网关检测（Dead Gateway Detection），防止慢速链路被系统判定失效而中途罢工。
- **应用兼容性**：为 Steam / IDM 一键写入或还原 SOCKS5 代理配置。
- **系统托盘**：支持最小化到托盘 / 直接退出两种关闭行为。

### 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2（原生 WebView2，单文件、低占用） |
| 后端引擎 | Rust + tokio 异步运行时 + socket2 + windows-sys（IPHLPAPI / WinSock / WinINet） |
| 前端 | React 19 + TypeScript + Vite |
| 样式与动效 | TailwindCSS 4 + Framer Motion + Lucide 矢量图标 |

### 工作原理

```text
[多线程下载流量 (Steam / IDM / 浏览器)]
               │
               ▼  WinINet 系统全局代理自动接管
   http/https -> 127.0.0.1:10801 | socks -> 127.0.0.1:10800
               │
               ▼
   HypoMux Plus 分流引擎 (Rust + tokio)
               │
               ▼  Round-Robin 连接轮询
   L3 物理层套接字强绑定 (IP_UNICAST_IF + bind)
   ├── 网卡 1 (IfIndex) ──┐
   ├── 网卡 2 (IfIndex) ──┼─► 物理带宽叠加吞吐
   └── 网卡 N (IfIndex) ──┘
```

加速时程序向 `HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings` 写入全覆盖代理链条；分流引擎收到客户端 TCP 连接后，先 `setsockopt(IPPROTO_IP, IP_UNICAST_IF, htonl(if_index))` 锁死出口物理网卡，再 `bind(local_ip, 0)` 固定源地址，强制流量剥离默认网关，实现物理多通道并进。

### 使用方法

1. **环境就绪**：确保电脑同时接入多条独立网络线路（例如有线宽带 + Wi-Fi + 手机 USB 网络共享）。
2. **勾选网卡**：启动程序，等待网卡扫描完成，在「网卡分流矩阵」中勾选参与聚合的活动网卡。
3. **一键加速**：点击「一键加速」，状态切换为运行中，系统全局代理被接管。
4. **开始下载**：打开 Steam 触发更新，或在 IDM 新建多线程下载任务，控制台会显示连接调度日志，大屏实时展示各线路吞吐。
5. **干净停止**：点击「停止加速」或关闭软件，系统代理自动安全还原。

### 开发与构建

需要预先安装 [Node.js](https://nodejs.org)、[Rust](https://www.rust-lang.org/tools/install) 与 [Tauri 前置依赖](https://tauri.app/start/prerequisites/)（Windows 需 WebView2 与 MSVC 构建工具）。

```bash
# 安装前端依赖
npm install

# 开发模式（热重载）
npm run tauri dev

# 构建发行版（生成独立 exe 与安装包）
npm run tauri build
```

### 安全与边界说明

- 本工具工作在标准应用层代理与网络套接字绑定层，**不触碰游戏内存、不修改游戏封包、不注入任何 DLL**。
- 多网卡聚合本质是**多连接负载均衡**，对单线程 TCP 死速下载无法加速。
- 多网卡分流面向下载吞吐量；游玩对延迟敏感的网游前，请先「停止加速」，让网络回归单一默认网关。
- 部分稳定性增强功能（死网关检测）需要管理员权限，未提权时核心分流仍可正常使用。

### 致谢与衍生声明

本项目衍生自 [Hypostasis-Cat / HypoMux](https://github.com/Hypostasis-Cat/HypoMux)，核心的多网卡物理绑定思想与协议设计均源自原项目，特此致谢。HypoMux Plus 在其基础上重写了桌面客户端的全部界面与后端引擎实现。

### 开源协议

本项目基于 **AGPL-3.0** 协议开源，与原项目保持一致。详见 [LICENSE](./LICENSE)。

- 原作者 / Original Author：**Hypostasis-Cat**
- 衍生开发者 / Derivative Developer：**青云制作_彭明航**

---

## English

HypoMux Plus is a modernized desktop client of a **multi-network-adapter bandwidth aggregation download accelerator** for Windows. Built on top of the core ideas of [HypoMux by Hypostasis-Cat](https://github.com/Hypostasis-Cat/HypoMux), it is fully rebuilt with **Tauri + Rust + React + TailwindCSS**, delivering a more refined, fluid and professional desktop experience. The splitting engine is natively rewritten in Rust (tokio), producing a self-contained executable with zero runtime dependencies.

> This is a derivative work of the original HypoMux, released under its **AGPL-3.0** license. Original author: Hypostasis-Cat; derivative developer: **青云制作_彭明航 (Qingyun Studio / Peng Minghang)**.

### Key Features

- **Seamless Dual-Protocol Takeover**: Runs SOCKS5 and HTTP/HTTPS forwarders simultaneously, applying the Windows WinINet system proxy automatically. Compatible with Steam, IDM, browsers and any client honoring the system proxy.
- **L3 Socket Binding**: Each outbound connection is pinned to a chosen NIC via `setsockopt(IP_UNICAST_IF)` plus source `bind`, eliminating the same-subnet `WinError 10049` wrong-adapter problem.
- **Round-Robin Dispatch**: Connections are distributed across the selected adapters to physically stack bandwidth for multi-threaded downloads.
- **Fail-Safe Proxy Restore**: Manual stop, startup failure, window close and process exit all force-restore the system proxy.
- **Live Telemetry Dashboard**: Per-second sampling via kernel counters (`GetIfEntry2`) shows combined speed, a live waveform, and per-NIC speed and active connections.
- **Modern UI**: Dark / light themes, glassmorphism, fluid motion, full Chinese/English bilingual support, vector icons throughout (no emoji).
- **Stability Boost**: Dead Gateway Detection is disabled while boosting to keep slow links from being dropped by the OS.
- **App Compatibility**: One-click SOCKS5 config apply/restore for Steam and IDM.
- **System Tray**: Minimize-to-tray or exit-on-close behaviors.

### Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop | Tauri 2 (native WebView2, small footprint) |
| Backend | Rust + tokio + socket2 + windows-sys (IPHLPAPI / WinSock / WinINet) |
| Frontend | React 19 + TypeScript + Vite |
| Styling & Motion | TailwindCSS 4 + Framer Motion + Lucide icons |

### How It Works

When boosting, the app writes a full proxy chain into `HKCU\...\Internet Settings`. Upon receiving a client TCP connection, the engine first locks the outbound NIC with `setsockopt(IPPROTO_IP, IP_UNICAST_IF, htonl(if_index))`, then binds the local source IP, forcing traffic off the default gateway for true multi-channel throughput.

### Usage

1. Connect your PC to multiple independent networks (e.g. wired broadband + Wi-Fi + phone USB tethering).
2. Launch the app, wait for the adapter scan, and check the adapters to aggregate.
3. Click **Boost**; the system proxy is engaged automatically.
4. Start a Steam update or an IDM multi-threaded download and watch the dispatch console and live dashboard.
5. Click **Stop** or close the app; the system proxy is restored automatically.

### Development & Build

Prerequisites: [Node.js](https://nodejs.org), [Rust](https://www.rust-lang.org/tools/install), and [Tauri prerequisites](https://tauri.app/start/prerequisites/) (WebView2 and MSVC build tools on Windows).

```bash
npm install        # install frontend deps
npm run tauri dev  # development with hot reload
npm run tauri build  # production build (standalone exe + installer)
```

### Safety & Boundaries

- Operates purely at the application-layer proxy and socket-binding level. It does not touch game memory, modify packets, or inject DLLs.
- Multi-NIC aggregation is connection-level load balancing; it cannot accelerate single-threaded rate-capped downloads.
- For latency-sensitive online games, click **Stop** first to return to a single default gateway.
- Some stability features (dead gateway detection) require administrator rights; core splitting works without elevation.

### Acknowledgments & Derivative Notice

This project derives from [Hypostasis-Cat / HypoMux](https://github.com/Hypostasis-Cat/HypoMux). The core multi-NIC physical binding approach and protocol design originate from the original project. HypoMux Plus rewrites the entire desktop client UI and backend engine on top of it.

### License

Licensed under **AGPL-3.0**, consistent with the original project. See [LICENSE](./LICENSE).

- Original Author: **Hypostasis-Cat**
- Derivative Developer: **青云制作_彭明航**
