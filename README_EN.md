<div align="center">

<img src="./appicon.svg" width="120" height="120" alt="HypoMuxPlus" />

# HypoMuxPlus

**Multi-NIC Bandwidth Aggregation Tool**

[![Tauri](https://img.shields.io/badge/Tauri-2.x-FFC131?style=flat-square&logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.90%2B-000000?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-19-61DAFB?style=flat-square&logo=react&logoColor=white)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?style=flat-square&logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![TailwindCSS](https://img.shields.io/badge/TailwindCSS-4-06B6D4?style=flat-square&logo=tailwindcss&logoColor=white)](https://tailwindcss.com)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-0078D4?style=flat-square&logo=windows)](#)
[![License](https://img.shields.io/badge/License-AGPL--3.0-D22128?style=flat-square)](./LICENSE)

[![Download](https://img.shields.io/badge/⬇%20下载%20Download-v1.2.6-3b82f6?style=for-the-badge)](https://github.com/pmh1314520/HypoMuxPlus/releases/download/v1.2.6/HypoMuxPlus.exe)
[![GitHub](https://img.shields.io/badge/GitHub-pmh1314520%2FHypoMuxPlus-181717?style=for-the-badge&logo=github)](https://github.com/pmh1314520/HypoMuxPlus)
[![Gitee](https://img.shields.io/badge/Gitee-peng--minghang%2Fhypo--mux--plus-C71D23?style=for-the-badge&logo=gitee)](https://gitee.com/peng-minghang/hypo-mux-plus)

**🌐 Language / 语言：[简体中文](./README.md) · English**

</div>

---

HypoMuxPlus is a **multi-network-adapter bandwidth aggregation tool** for Windows. Built on top of the core ideas of [HypoMux by Hypostasis-Cat](https://github.com/Hypostasis-Cat/HypoMux), it is fully rebuilt with **Tauri + Rust + React + TailwindCSS**, delivering a more refined, fluid and professional desktop experience. The splitting engine is natively rewritten in Rust (tokio), producing a self-contained executable with zero runtime dependencies.

> This is a derivative work of the original HypoMux, released under its **AGPL-3.0** license. Original author: Hypostasis-Cat; derivative developer: **青云制作_彭明航 (Qingyun Studio / Peng Minghang)**.

## Download

- **Windows 10 / 11 only.** Just download and run (running as administrator is recommended to enable all stability features).
- Direct download: **[HypoMuxPlus.exe (v1.2.6)](https://github.com/pmh1314520/HypoMuxPlus/releases/download/v1.2.6/HypoMuxPlus.exe)** (via GitHub Releases; users in mainland China can use the [Gitee mirror](https://gitee.com/peng-minghang/hypo-mux-plus/releases/download/v1.2.6/HypoMuxPlus.exe))
- Repositories: [GitHub](https://github.com/pmh1314520/HypoMuxPlus) · [Gitee](https://gitee.com/peng-minghang/hypo-mux-plus)
- Website: **[hmp.pmhs.top](https://hmp.pmhs.top)**

## Preview

<div align="center">

**🚀 Acceleration Console**

<img src="./docs/console-dark.png" width="780" alt="Acceleration Console" />

**📊 Live Statistics · 🩺 Link Diagnostics**

<img src="./docs/stats-dark.png" width="780" alt="Live Statistics" />

<img src="./docs/diagnostics-dark.png" width="780" alt="Link Diagnostics" />

**⚙️ Settings · 📖 Tutorial · ℹ️ About**

<img src="./docs/settings-dark.png" width="780" alt="Settings" />

<img src="./docs/tutorial-dark.png" width="780" alt="Tutorial" />

<img src="./docs/about-dark.png" width="780" alt="About" />

**🌗 Dark / Light Themes (Console as example)**

| Dark Theme | Light Theme |
| :---: | :---: |
| <img src="./docs/console-dark.png" width="390" alt="Dark Theme" /> | <img src="./docs/console-light.png" width="390" alt="Light Theme" /> |

<sub>Built-in dark / light themes and full Chinese/English bilingual support. Visit the project website for more screenshots.</sub>

</div>

## Key Features

### Splitting & Scheduling
- **Seamless Dual-Protocol Takeover**: Runs SOCKS5 and HTTP/HTTPS forwarders simultaneously, applying the Windows WinINet system proxy automatically. Compatible with Steam, IDM, browsers and any client honoring the system proxy.
- **Per-Connection NIC Egress Binding**: Each outbound TCP connection is directed out a chosen NIC via `setsockopt(IP_UNICAST_IF)` (egress interface index) plus a source `bind`, eliminating the same-subnet `WinError 10049` wrong-adapter problem. This is **connection-level egress selection**, not packet-level link bonding / MPTCP.
- **Smart Scheduler**: Three connection strategies — classic round-robin, least-connections, and dynamic weighting by real-time download speed (smooth weighted round-robin) — so faster adapters carry more connections and weak links no longer hold aggregation back.
- **Per-NIC Weight & Limit · Global Download Cap**: Set a scheduling weight and per-adapter download cap for each NIC, plus a global download speed cap, for fine-grained control over per-link load and total bandwidth usage.
- **Routing Rules**: Custom rules by domain / port / process name — direct (bypass proxy), aggregate, or pin to a specific NIC; process-name matching (e.g. `steam.exe`) takes priority over domain rules; plus a direct-connect allowlist and rule-list subscription from a URL.
- **SOCKS5 UDP Relay**: Supports SOCKS5 UDP ASSOCIATE, relaying UDP datagrams out the chosen NIC egress for proxied apps, compatible with clients that rely on UDP.
- **Upstream Proxy Chain (Multi-Node Aggregation)**: Bind an upstream proxy node (SOCKS5 / HTTP, with username/password auth) to each participating NIC, so "NIC A → Node 1" and "NIC B → Node 2" stack in parallel, breaking through both the local uplink bandwidth and single-node bottlenecks at once; a single NIC can rotate across multiple upstreams, with a fallback strategy when all upstreams are unavailable (fall back to direct / return failure). Off by default — behavior is identical to before when disabled.
- **Upstream Health Probing & Circuit Breaking**: Periodically probes the availability and latency of referenced upstream nodes; a node is tripped (temporarily excluded from candidates) once consecutive failures hit the threshold, then half-open probed after a cooldown and auto-recovered on success; when a NIC binds multiple upstreams, faster and more stable nodes are preferred by health and latency weighting. Probe interval / timeout / trip threshold / cooldown are all configurable.
- **Subscription Import**: Paste a Clash subscription / base64 subscription body / node share links to parse out supported (socks5 / http) upstream candidates, then one-click benchmark-sort and confirm them into the upstream node list (respecting the 128-entry cap).
- **IPv6 Dual-Stack Splitting**: Supports IPv6 literal targets and dual-stack (A/AAAA) domain resolution, with Happy-Eyeballs-style dialing and fallback by IP-version preference.
- **Per-NIC DNS / DoH**: Assign a plaintext DNS server or DoH endpoint per participating NIC, so resolution requests egress through that NIC and never leak out the wrong interface; unconfigured NICs fall back to the existing global resolution path.
- **Global Takeover (TUN) Mode · No Per-App Proxy Config**: A built-in global-takeover mode using a wintun virtual adapter + user-space TCP/IP stack routes all system traffic into the multi-NIC engine in one click — no need to configure a proxy per application. DNS uses fake-ip to bypass hijacking. A **service mode** is supported: after a one-time install (single UAC prompt), TUN can be enabled with standard privileges thereafter, no admin needed each time.

### Diagnostics & Monitoring
- **Link Test & Benchmark**: One-click per-adapter latency (RTT) probing — also measuring jitter and packet loss and plotting a historical trend curve — plus per-adapter download benchmarking to pick the healthiest, fastest links; results can be exported as an image or text report.
- **One-Click Aggregate Benchmark**: Concurrently benchmarks all selected adapters and shows "single-NIC speed → combined speed" and the improvement.
- **Live Connection Monitor**: A live connection list shows each connection's target and the adapter it was assigned to — fully transparent dispatch, with filtering by protocol / NIC / target and export.
- **Live Telemetry Dashboard**: Per-second sampling via kernel counters (`GetIfEntry2`) shows combined speed, a live waveform, and per-NIC speed and active connections.
- **Floating HUD**: An optional always-on-top mini overlay showing combined speed, up/down, connection count and per-NIC sparklines; supports position lock, click-through, opacity and accent following the main UI.
- **NIC Dropout Guard**: While boosting, participating adapters are monitored live; a lost link is removed from rotation (with a prompt) and re-added automatically once recovered.
- **Lifetime & Daily Stats**: Persisted cumulative traffic, peak and duration across sessions, plus a recent daily-traffic trend; each boost ends with a "session report" you can export as PNG.

### Stability & Automation
- **Fail-Safe Proxy Restore**: Manual stop, startup failure, window close and process exit all force-restore the system proxy; on launch it only cleans up leftovers from this app, never touching third-party proxies like Clash.
- **Stability Boost**: Dead Gateway Detection is disabled while boosting to keep slow links from being dropped by the OS.
- **Proxy Leak Guardian**: Continuously watches the system proxy settings and auto-corrects them the moment an external program tampers with or clears them; the original settings are snapshotted before takeover and compensated on next launch after an abnormal exit, reducing the risk of traffic leaking past the splitting engine or losing connectivity from leftover proxy config.
- **Stability Hardening (Per-Connection Isolation + Cap Protection)**: A single connection's error is caught and isolated — only that connection is released, without affecting the engine or other connections; configurable caps on concurrent connections and background-task concurrency prevent resource exhaustion under extreme load; unhandled exceptions are written to a structured crash log (with sensitive data redacted).
- **Process-Aware Auto-Boost**: Automatically starts boosting when download apps such as Steam / IDM / Thunder / qBittorrent are detected running, and stops once they all exit (only sessions it auto-started).
- **App Compatibility**: One-click SOCKS5 config apply/restore for Steam and IDM.
- **Global Hotkeys & Notifications**: Bind separate "boost" and "stop" global hotkeys (default `Ctrl+Alt+H`, customizable) to toggle from anywhere; system notifications announce start/stop.
- **CLI Control · Single Instance**: Supports `--start` / `--stop` / `--toggle` / `--show` / `--quit` command-line control; runs as a single instance and forwards commands to the existing instance.
- **In-App Auto-Update**: Silently checks for new versions on launch; one click downloads and, after exit, silently replaces and restarts (with visible progress).
- **Automation**: Launch at startup, start minimized to tray, and auto-boost with the last selected adapters on launch.

### UI & Experience
- **Modern UI**: Dark / light themes (can follow the system), customizable accent color, high-contrast mode, glassmorphism, fluid motion, full Chinese/English bilingual support, vector icons throughout (no emoji), with keyboard and screen-reader accessibility.
- **Route Decision Simulator**: Enter a target domain / process name (and optional port) to simulate, by pure computation, the splitting path it would hit (bypass / rule / carrying NIC / direct or via upstream / the selected upstream node) — display only, making no real connection and changing no boost state, handy for troubleshooting rule configuration.
- **System Tray**: Minimize-to-tray or exit-on-close behaviors; the tray icon renders the live aggregate speed as a number, with speed and connection count on hover.

## Tech Stack

| Layer | Technology |
| --- | --- |
| Desktop | Tauri 2 (native WebView2, small footprint) |
| Backend | Rust + tokio + socket2 + windows-sys (IPHLPAPI / WinSock / WinINet) |
| Frontend | React 19 + TypeScript + Vite |
| Styling & Motion | TailwindCSS 4 + Framer Motion + Lucide icons |

## How It Works

```text
[Multi-threaded download traffic (Steam / IDM / Browser)]
               │
               ▼  WinINet system-wide proxy auto-takeover
   http/https -> 127.0.0.1:10801 | socks -> 127.0.0.1:10800
               │
               ▼
   HypoMuxPlus splitting engine (Rust + tokio)
               │
               ▼  Connection dispatch by strategy
   Per-connection socket egress binding (IP_UNICAST_IF + bind)
   ├── NIC 1 (IfIndex) ──┐
   ├── NIC 2 (IfIndex) ──┼─► Multi-connection aggregated throughput
   └── NIC N (IfIndex) ──┘
```

When boosting, the app writes a full proxy chain into `HKCU\...\Internet Settings`. Upon receiving a client TCP connection, the engine first resolves the target's real IP through the chosen physical NIC (DoH/UDP, bypassing fake-ip hijacking), then selects that connection's egress interface via `setsockopt(IPPROTO_IP, IP_UNICAST_IF, htonl(if_index))` and binds the local source IP, so the connection leaves through the target NIC. The many parallel connections of a multi-threaded download are distributed across NICs, stacking link bandwidth in a **connection-level load-balancing** sense. When the upstream proxy chain is enabled, a connection first leaves through the chosen NIC's physical egress to reach its bound upstream node, which then forwards it to the target.

> **Scope & limitations**: HypoMuxPlus is a user-space **per-connection multi-NIC proxy dispatcher (L4)** — not an L3/L4 link-bonding, MPTCP or SD-WAN system. Speed gains come from "multiple parallel TCP connections + per-connection NIC distribution + server-side multi-connection support (CDN / HTTP Range)"; it **cannot accelerate a single TCP stream**. `IP_UNICAST_IF` is an egress-interface hint, not hard isolation. If **Clash/Mihomo TUN mode** is running, it takes over all IP-layer traffic via a virtual adapter at L3, overriding this app's L4 NIC binding and collapsing multi-NIC to a single uplink — in that case disable TUN (use Clash's system-proxy/rule mode) for real multi-NIC splitting.

## Use Cases

Any network links that are **mutually independent — each with its own uplink bandwidth** — can join the pool and stack up:

- **Dorm · Pool your roommate's broadband**: Your PC is on a wired line that tops out around 10 MB/s; your roommate has a separate broadband line that also reaches 10 MB/s. Connect your PC to their network over Wi-Fi as well, and both links enter the pool — HypoMuxPlus stacks them so downloads can approach 20 MB/s.
- **Add one more · Bring phone data into the pool**: Two links still not enough? Tether your phone over USB, and its 4G/5G data becomes a third independent link joining the aggregation, pushing bandwidth even higher. The more independent links, the bigger the gain.
- **Home / Studio · Many lines at once**: With dual home broadband (e.g. two ISPs) or several independent leased lines in a studio, select them all to aggregate and let Steam updates and large downloads max out every line.

> ⚠️ **Key requirement**: each link must have its own independent uplink bandwidth. If your "wired" and "wireless" actually go through the same router / same broadband line (sharing one ISP uplink), aggregation does **nothing** — they already compete for the same bandwidth, so combining them adds no total capacity.

## Usage

1. Connect your PC to multiple independent networks (e.g. wired broadband + Wi-Fi + phone USB tethering).
2. Launch the app, wait for the adapter scan, and check the adapters to aggregate.
3. Click **Boost**; the system proxy is engaged automatically.
4. **Point your download tool at the proxy (important)** — traffic only aggregates if it goes through this app's proxy:
   - Clients that honor the system proxy (most browsers, Steam) usually work automatically.
   - **Tools that ignore the system proxy (IDM, Thunder, qBittorrent…) must be set manually**: set **SOCKS5 proxy `127.0.0.1:10800`** (or HTTP `127.0.0.1:10801`; see Settings for ports). For Steam / IDM you can one-click apply in **Settings → App Compatibility**.
5. Start a multi-threaded download and watch the dispatch console and live dashboard.
6. Click **Stop** or close the app; the system proxy is restored automatically.

> **Only one adapter carries traffic / no speed change?** 90% of the time step 4 was missed — the download tool isn't pointing at this app's proxy, so traffic never enters the splitting engine. Set SOCKS5 `127.0.0.1:10800` in the tool. Also confirm each participating adapter has its **own independent internet uplink** (the app runs a "NIC self-test" on boost and prints the result in the dispatch log) and that the download is multi-threaded.

> **Don't want to configure a proxy per app?** Enable **Global Takeover (TUN) mode** in Settings to route all system traffic through the multi-NIC engine in one click — no per-tool proxy setup needed. It's recommended to install the "TUN service mode" first, so it can be enabled with standard privileges afterward (otherwise run the app as administrator). Note: TUN mode conflicts with Clash/Mihomo's TUN — only one can be enabled at a time.

## Development & Build

Prerequisites: [Node.js](https://nodejs.org), [Rust](https://www.rust-lang.org/tools/install), and [Tauri prerequisites](https://tauri.app/start/prerequisites/) (WebView2 and MSVC build tools on Windows).

```bash
npm install          # install frontend deps
npm run tauri dev    # development with hot reload
npm run tauri build  # production build (standalone exe + installer)
```

## Safety & Boundaries

- Operates purely at the application-layer proxy and socket-binding level. It does not touch game memory, modify packets, or inject DLLs.
- Multi-NIC aggregation is connection-level load balancing; it cannot accelerate single-threaded rate-capped downloads.
- For latency-sensitive online games, click **Stop** first to return to a single default gateway.
- Some stability features (dead gateway detection) require administrator rights; core splitting works without elevation.

## Support

HypoMuxPlus is completely free and open source! If it helped you, consider buying the author a coffee — please note "HypoMuxPlus" when donating. Donations are entirely voluntary; all features stay free forever regardless.

<div align="center">

| WeChat Pay | Alipay |
| :---: | :---: |
| <img src="./docs/sponsor-wechat.png" width="220" alt="WeChat Pay" /> | <img src="./docs/sponsor-alipay.jpg" width="220" alt="Alipay" /> |

**Developer contact: WeChat `QyPmh20061026` · QQ `2124691573`**

</div>

## Acknowledgments & Derivative Notice

This project derives from [Hypostasis-Cat / HypoMux](https://github.com/Hypostasis-Cat/HypoMux). The core multi-NIC physical binding approach and protocol design originate from the original project. HypoMuxPlus rewrites the entire desktop client UI and backend engine on top of it.

## License

Licensed under **AGPL-3.0**, consistent with the original project. See [LICENSE](./LICENSE).

- Original Author: **Hypostasis-Cat**
- Derivative Developer: **青云制作_彭明航**
