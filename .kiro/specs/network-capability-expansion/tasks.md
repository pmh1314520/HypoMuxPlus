# Implementation Plan: 网络能力扩展与工程补强

## Overview

本实现计划将 `design.md` 拆解为可增量交付、可独立验证的编码任务，严格遵循设计文档「实现顺序与依赖」的 9 个阶段：

- **阶段 A 地址族泛化**（前置基础，B / C / D / E 均依赖 A）
- **阶段 B SOCKS5 IPv6 + 双栈**（依赖 A）
- **阶段 C 可测性重构 + 测试基建**（前置，服务 Req 6/7，可与 A/B 并行；D 及所有属性测试依赖 C）
- **阶段 D 按进程分流**（依赖 A、C）
- **阶段 E TUN UDP/QUIC**（依赖 A、D）
- **阶段 F SOCKS5 UDP ASSOCIATE**（可选，依赖 A —— 本阶段全部子任务标注为可选 `*`）
- **阶段 G 本地日志**（相对独立）
- **阶段 H 诊断增强**（地址族无关，可较早并行）
- **阶段 I 测试落地**（既有纯函数回归 + 前端 Req7，依赖 C 测试基建）

核心约束：**对既有 IPv4 / TCP / DNS / 限速 / 调度 / fake-ip DNS 路径零破坏**。每个任务在验证点写明相应的既有路径回归检查。

编码语言沿用现状：后端 Rust（tokio），属性测试用 `proptest`（每属性 ≥100 次，带 `// Feature: network-capability-expansion, Property N` 注释）；前端 TypeScript，属性测试用 `vitest` + `fast-check`（每属性 ≥100 次）。每条 Correctness Property（共 24 条）对应一个独立的属性测试子任务，放在其实现阶段附近以尽早发现回归。

> 说明：不适合自动化测试的验收项（真实网卡 setsockopt/bind、`GetExtendedTcpTable`/`GetExtendedUdpTable` 系统调用、TUN 实机 UDP、canvas PNG 导出、打开日志目录等）不作为编码任务，统一列在文末「人工实机 / 集成冒烟验证清单」中，由人工验证。

## Tasks

- [x] 1. 阶段 A — 地址族泛化（前置基础，服务 Req 1/2/3/5）
  - [x] 1.1 泛化 `netadapter.rs` 网卡扫描为双栈并新增 `ipv6` 字段
    - 将 `GetAdaptersAddresses` 的 family 由 `AF_INET` 改为 `AF_UNSPEC`；遍历 `FirstUnicastAddress` 链表时，`AF_INET` 分支保留既有「取首个 IPv4」逻辑（行为不变），新增 `AF_INET6` 分支从 `SOCKADDR_IN6` 收集地址
    - `AdapterInfo` 结构新增 `pub ipv6: String`（serde camelCase），无 IPv6 时为 `""`；既有 `ipv4` / `is_virtual` 判定行为不变
    - 新增纯函数 `pub(crate) fn select_global_ipv6(addrs: &[Ipv6Addr]) -> Option<Ipv6Addr>`（全局单播优先于 `fe80::/10` 链路本地）与 `pub(crate) fn is_link_local_v6(ip: &Ipv6Addr) -> bool`
    - 验证点：既有 `ipv4` 字段取值与 `is_virtual` 结果对现有网卡保持不变
    - _Requirements: 2.1, 2.2, 2.3, 2.4_

  - [x] 1.2 泛化 `engine.rs` 网卡运行时与出站工厂
    - `NicRuntime` 增加 `pub ipv6: Option<Ipv6Addr>`；内部源地址字段由 `ip` 明确为 `ipv4: Ipv4Addr`（内部字段，非序列化）
    - 将 `connect_via_nic(nic, dst)` 的 `dst` 由 `SocketAddrV4` 泛化为 `SocketAddr`，按目标地址族分派：IPv4 走既有 `Domain::IPV4` + `setsockopt(IPPROTO_IP=0, IP_UNICAST_IF=31, if_index.to_be())` + `bind(nic.ipv4,0)`；IPv6 新增分支 `Domain::IPV6` + `setsockopt(IPPROTO_IPV6=41, IPV6_UNICAST_IF=31, if_index.to_be())` + `bind(nic.ipv6,0)`
    - 新增常量 `IPPROTO_IPV6 = 41`
    - 验证点：`ATYP=0x01/0x03` 的 IPv4 出站路径（`IP_UNICAST_IF` 绑定 + 非阻塞 connect）行为完全不变
    - _Requirements: 1.2, 1.7_

  - [x] 1.3 前端网卡类型契约与展示同步
    - `lib/api.ts`：`AdapterInfo` 增加 `ipv6: string`；`SelectedNic` 增加可选 `ipv6?: string`
    - `AdapterTable.tsx`：网卡条目在存在 IPv6 时展示 `ipv6`
    - `i18n.ts`：新增 IPv6 展示相关文案键并保证中英字典同步对齐
    - 验证点：无 IPv6 网卡的条目展示与既有一致，不出现空白/未定义
    - _Requirements: 2.5_

  - [x] 1.4 编写 `select_global_ipv6` 属性测试（proptest）
    - **Property 5: 代表性全局 IPv6 选择（select_global_ipv6）**
    - 存在至少一个全局单播时返回非链路本地地址；集合为空或全为链路本地时返回 `None`
    - `// Feature: network-capability-expansion, Property 5`，≥100 次
    - **Validates: Requirements 2.2, 2.3**

- [x] 2. 阶段 B — SOCKS5 IPv6 与双栈策略（依赖 A，Req 1）
  - [x] 2.1 实现双栈地址族决策纯函数 `pick_family`
    - 新增 `pub(crate) enum Family { V4, V6 }` 与 `pub(crate) fn pick_family(pref: &str, has_v4: bool, has_v6: bool) -> Vec<Family>`
    - `pref ∈ {"auto","v4first","v6first","v4only"}`：`v4only` 仅返回 `[V4]`；单族目标只返回该族；双栈按 `pref` 决定首位并包含备选族；`auto` 等价 `v6first`
    - _Requirements: 1.5, 1.6_

  - [x] 2.2 `handle_socks` 新增 `ATYP=0x04`（IPv6）解析分支
    - 解析 16 字节 IPv6 地址 + 端口为 `SocketAddr::V6`，接入既有调度选择网卡并调用泛化后的 `connect_via_nic`
    - 验证点：`ATYP=0x01`（IPv4）与 `ATYP=0x03`（域名）解析分支行为不变
    - _Requirements: 1.1_

  - [x] 2.3 新增 AAAA 查询/解析与双栈域名解析
    - 新增 `fn build_dns_query_type(host: &str, qtype: u16) -> Vec<u8>`（A=1, AAAA=28）与 `fn parse_dns_aaaa(buf: &[u8]) -> Option<Ipv6Addr>`
    - 新增 `struct ResolvedAddrs { v4: Option<Ipv4Addr>, v6: Option<Ipv6Addr> }` 与 `async fn resolve_host_dual(&self, nic, host, port) -> ResolvedAddrs`（复用既有 A 记录解析，追加 AAAA）
    - 验证点：既有 `build_dns_query` / `parse_dns_a` 保持不变，仅新增平行 AAAA 路径
    - _Requirements: 1.4_

  - [x] 2.4 实现 Happy-Eyeballs 式双栈拨号与回退
    - 新增 `async fn dial_dual(nic, addrs: &ResolvedAddrs, port, pref, timeout) -> io::Result<TcpStream>`，按 `pick_family` 顺序尝试，首选族在超时 `T` 内失败且存在备选族时回退另一族
    - 在 `handle_socks` 域名/IPv6 目标路径接入 `dial_dual`；无全局 IPv6 源地址或 `IPV6_UNICAST_IF` 失败时记录可读日志并回退 IPv4
    - 验证点：纯 IPv4 目标不触发任何 IPv6 分支，行为与既有一致
    - _Requirements: 1.3, 1.6_

  - [x] 2.5 前端 IP 版本偏好设置与参数透传
    - `store.tsx`：`Settings` 增加 `ipVersion: "auto"|"v4first"|"v6first"|"v4only"`（默认 `"auto"`）并持久化
    - `SettingsPage.tsx`：通用区新增「IP 版本」Segmented 控件
    - `lib/api.ts`：`startBoost` 增加 `ipVersion` 参数；`engine::start` / `lib.rs` 的 `start_boost` 增加 `ip_version: String` 透传到 `Engine.ip_version`（默认 `"auto"`）
    - `i18n.ts`：新增 `ipVersion` / `ipVerAuto` / `ipVerV4First` / `ipVerV6First` / `ipVerV4Only` 键，中英对齐
    - 验证点：未改动设置时默认 `"auto"`，既有 `startBoost` 调用与引擎启动行为不变
    - _Requirements: 1.5_

  - [x] 2.6 编写 SOCKS5 IPv6 请求头解析属性测试（proptest）
    - **Property 1: SOCKS5 IPv6 请求头解析 round-trip**
    - 任意 IPv6 地址与端口编码为 `ATYP=0x04` 地址段后解析应还原等价地址与端口
    - `// Feature: network-capability-expansion, Property 1`，≥100 次
    - **Validates: Requirements 1.1**

  - [x] 2.7 编写 `pick_family` 属性测试（proptest）
    - **Property 2: 双栈地址族选择（pick_family）综合正确性**
    - `v4only` 结果绝不含 `V6`；单族目标只含该族；双栈同在时含两族且首位由 `pref` 决定
    - `// Feature: network-capability-expansion, Property 2`，≥100 次
    - **Validates: Requirements 1.3, 1.5, 1.6**

  - [x] 2.8 编写 AAAA 查询/应答 round-trip 属性测试（proptest）
    - **Property 3: AAAA 查询/应答 round-trip**
    - `build_dns_query_type(host,28)` 问题段可还原同一 host；任意 IPv6 地址的 AAAA 应答经 `parse_dns_aaaa` 还原该地址
    - `// Feature: network-capability-expansion, Property 3`，≥100 次
    - **Validates: Requirements 1.4**

- [x] 3. 阶段 C — 可测性重构与测试基建（前置，服务 Req 6/7，可与 A/B 并行）
  - [x] 3.1 建立 Rust 属性测试基建
    - 在 `src-tauri/Cargo.toml` 的 `[dev-dependencies]` 加入 `proptest`
    - 约定统一的 `ProptestConfig { cases: 100, .. }`（辅助常量或每测试就地设置），确保 `cargo test` 独立于 GUI 与网络运行
    - _Requirements: 6.1, 6.7_

  - [x] 3.2 建立前端测试基建
    - 新增 `vitest.config.ts`（`environment: "jsdom"`，`include: ["src/**/*.test.ts?(x)"]`）
    - `package.json` 增加脚本 `"test": "vitest run"`、`"test:watch": "vitest"`，加入 `vitest`、`@types/*`、`jsdom`、`fast-check` 开发依赖
    - _Requirements: 7.1, 7.5_

  - [x] 3.3 析出前端诊断纯逻辑与 `niceCeil`
    - 新增 `src/lib/diag.ts`，从 `DiagnosticsPage.tsx` 析出 `appendTrendPoint`、`capTrend`、`buildReportLines` 为可导入的纯函数（不含 DOM/invoke）
    - 从 `AreaChart.tsx` 导出 `niceCeil` 供测试导入
    - 验证点：`DiagnosticsPage` 改为调用析出后的纯函数，页面行为不变
    - _Requirements: 7.3, 10.1, 10.2, 10.4_

  - [x] 3.4 析出 `RuleAction` 枚举与解析纯函数
    - 在 `engine.rs` 新增 `pub(crate) enum RuleAction { Direct, Aggregate, Nic(u32) }` 及解析/回写纯函数（`"direct"`/`"aggregate"`/`"nic:<ifindex>"` 双向），供进程规则与域名规则复用
    - 验证点：既有域名规则动作解析结果保持一致
    - _Requirements: 5.2_

- [x] 4. 阶段 D — 按进程名分流（依赖 A、C，Req 5）
  - [x] 4.1 新增 `process.rs`（Process_Resolver）
    - 新增纯函数 `pub(crate) struct TcpRow { local_addr: IpAddr, local_port: u16, pid: u32 }` 与 `pub(crate) fn find_pid_by_endpoint(rows: &[TcpRow], local: SocketAddr) -> Option<u32>`
    - 新增纯函数 `pub(crate) fn exe_name_from_path(path: &str) -> String`（提取小写可执行文件名）
    - 新增 `ProcessResolver`：`(localAddr,localPort)->PID`（TTL≈1s）与 `PID->name`（TTL≈10s）缓存；`resolve(local: SocketAddr) -> Option<String>` 薄封装 `GetExtendedTcpTable(TCP_TABLE_OWNER_PID_ALL, AF_INET/AF_INET6)` + `QueryFullProcessImageNameW`；仅在新连接建立时调用一次
    - 在模块声明处注册 `mod process;`
    - _Requirements: 5.4, 5.5_

  - [x] 4.2 规则数据结构扩展与 `pick_nic` 进程优先匹配
    - `RouteRuleDef` 增加 `#[serde(default = "default_kind")] kind: String`（`"domain"` 默认 / `"process"`），旧配置零迁移；`engine::start` 按 `kind` 分派 `domain`→既有 `bypass`/`rules_nic`、`process`→`rules_proc: Vec<(String /*小写exe*/, RuleAction)>`
    - 新增纯函数 `pub(crate) fn match_proc_rule(rules, proc_name) -> Option<RuleAction>`（大小写不敏感精确匹配）
    - `pick_nic(host, port, proc_name: Option<&str>)`：先按进程名匹配（优先级高于域名），`proc_name=None` 或未命中时回退既有域名规则 + 调度策略
    - 验证点：`proc_name=None` 时 `pick_nic` 结果与既有域名规则/调度路径完全一致
    - _Requirements: 5.1, 5.3, 5.6_

  - [x] 4.3 前端进程规则编辑与类型同步
    - `lib/api.ts`：`RouteRuleDef` 契约增加 `kind: "domain" | "process"`
    - `SettingsPage.tsx` 的 `RouteRulesEditor` 增加「进程规则」类型（可执行文件名输入 + `direct`/`aggregate`/`nic:<ifindex>` 动作），持久化到既有 `rules` 数组
    - `i18n.ts`：新增 `ruleKindDomain` / `ruleKindProcess` / `procNamePlaceholder` 等键，中英对齐；新增交互元素补充 `aria-label`
    - 验证点：缺省 `kind` 的旧规则仍按域名规则渲染与生效
    - _Requirements: 5.7_

  - [x] 4.4 编写 `match_proc_rule` 属性测试（proptest）
    - **Property 10: 进程规则匹配大小写不敏感（match_proc_rule）**
    - `// Feature: network-capability-expansion, Property 10`，≥100 次
    - **Validates: Requirements 5.1**

  - [x] 4.5 编写 `RuleAction` 解析 round-trip 属性测试（proptest）
    - **Property 11: 规则动作解析 round-trip（RuleAction）**
    - 合法动作串解析再回写等价；`nic:<n>` 解析出的接口索引等于 `n`
    - `// Feature: network-capability-expansion, Property 11`，≥100 次
    - **Validates: Requirements 5.2**

  - [x] 4.6 编写 `pick_nic` 优先级与回退属性测试（proptest）
    - **Property 12: 进程规则优先级与无进程回退（pick_nic）**
    - 命中进程规则时结果等于该动作（无论是否命中域名规则）；`proc_name=None` 时与「无进程规则」路径一致
    - `// Feature: network-capability-expansion, Property 12`，≥100 次
    - **Validates: Requirements 5.3, 5.6**

  - [x] 4.7 编写 `find_pid_by_endpoint` 属性测试（proptest）
    - **Property 13: 连接表端点反查 PID（find_pid_by_endpoint）**
    - 存在匹配 `(本地地址,本地端口)` 行则返回其 PID，否则 `None`
    - `// Feature: network-capability-expansion, Property 13`，≥100 次
    - **Validates: Requirements 5.4, 5.5**

- [x] 5. 检查点 — 确保 A/B/C/D 全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

- [x] 6. 阶段 E — TUN 模式 UDP / QUIC 中继（依赖 A、D，Req 3）
  - [x] 6.1 泛化 `parse_dns_question` 返回 qtype 并支持 AAAA fake-ip
    - `parse_dns_question(buf) -> Option<(u16 id, String host, u16 qtype)>`（末位由 bool 泛化为 qtype）
    - AAAA 查询复用同一 fake-ip 分配或按策略回空；`build_dns_response` / `allocate` / `lookup` 既有 A 记录行为不变
    - 验证点：53 端口 A 记录 fake-ip 应答行为完全不变（Req 3.5）
    - _Requirements: 3.5_

  - [x] 6.2 实现 UDP 会话表与空闲回收纯函数
    - 新增 `struct UdpSession { upstream: Arc<UdpSocket>, last_active: Instant, nic_name: String }`、`type UdpKey = (SocketAddr, SocketAddr)`、`struct UdpSessionTable { inner: Mutex<HashMap<UdpKey, UdpSession>> }`
    - 新增纯函数 `pub(crate) fn expired_udp_keys(entries: &[(UdpKey, Instant)], now: Instant, idle: Duration) -> Vec<UdpKey>`（`now - last_active > idle` 的 key 全含且不含未超时者）
    - _Requirements: 3.3_

  - [x] 6.3 实现非 53 端口 UDP 中继与出口绑定
    - 新增 `async fn udp_socket_via_nic(nic, family: Family) -> io::Result<UdpSocket>`（IPv4/IPv6 各自 `*_UNICAST_IF` + bind 源地址）
    - `handle_udp`：目标端口非 53 时不再丢弃，建立/复用经所选网卡的 UDP 中继；目标为 Fake_IP 时经 `FakeDns::lookup` 反查域名并用所选网卡 `resolve_host_dual` 解析真实地址再中继；网卡选择复用 `engine.pick_nic`（含 bypass / 按网卡规则 / 调度 / 进程规则）
    - 启动后台 tokio 定时任务（约 5s）调用 `expired_udp_keys` 回收 `idle>60s` 会话并 drop socket；上游 socket 创建失败时记录日志并结束该流，不影响其他会话
    - 验证点：53 端口 DNS 与既有 TCP 流处理路径行为不变（Req 3.5/3.6）
    - _Requirements: 3.1, 3.2, 3.4, 3.6_

  - [x] 6.4 编写 Fake-IP 分配 round-trip 与幂等属性测试（proptest）
    - **Property 6: Fake-IP 分配 round-trip 与幂等**
    - `// Feature: network-capability-expansion, Property 6`，≥100 次
    - **Validates: Requirements 3.2, 6.5**

  - [x] 6.5 编写 `expired_udp_keys` 属性测试（proptest）
    - **Property 7: UDP 会话空闲回收（expired_udp_keys）**
    - `// Feature: network-capability-expansion, Property 7`，≥100 次
    - **Validates: Requirements 3.3**

  - [x] 6.6 编写 DNS 问题/应答 round-trip 属性测试（proptest）
    - **Property 8: DNS 问题/应答 round-trip**
    - 构造 A 查询经 `parse_dns_question` 还原同一 id 与域名；`build_dns_response` 回填后解析 id 与问题段域名保持一致
    - `// Feature: network-capability-expansion, Property 8`，≥100 次
    - **Validates: Requirements 3.5, 6.5, 6.6**

- [x] 7. 阶段 F — SOCKS5 UDP ASSOCIATE（可选能力，依赖 A，Req 4）
  - [x] 7.1 实现 SOCKS5 UDP 请求头解析/封装纯函数
    - 新增 `pub(crate) fn parse_socks_udp_header(buf) -> Option<(SocksUdpTarget, usize)>`（RSV(2) FRAG(1) ATYP ADDR PORT，返回目标与载荷偏移）与 `pub(crate) fn build_socks_udp_header(target: &SocksUdpTarget) -> Vec<u8>`
    - _Requirements: 4.2_

  - [x] 7.2 实现 `CMD=0x03` UDP ASSOCIATE 与未启用拒绝
    - 新增 `async fn udp_associate(engine, client)`：在 `127.0.0.1` 分配 UDP 中继端口，应答返回 BND.ADDR/BND.PORT；按请求头目标经所选网卡 Egress_Binding 转发数据报
    - `Engine` 增加 `udp_associate: bool`（默认 `false`）；`handle_socks` 中 `CMD=0x03` 在未启用时走既有非 CONNECT 拒绝分支（`REP=0x07`）
    - 验证点：`CMD=0x01`（CONNECT）路径完全不变（Req 4.4）
    - _Requirements: 4.1, 4.3, 4.4_

  - [x] 7.3 前端 UDP ASSOCIATE 开关
    - `store.tsx`：`Settings` 增加 `udpAssociate: boolean`（默认 `false`）并持久化；`lib/api.ts` 的 `startBoost` 增加 `udpAssociate` 参数，`start_boost`/`engine::start` 增加 `udp_associate: bool` 透传
    - `SettingsPage.tsx` 流量控制区新增开关；`i18n.ts` 新增键，中英对齐 + `aria-label`
    - _Requirements: 4.1_

  - [x] 7.4 编写 SOCKS5 UDP 请求头 round-trip 属性测试（proptest）
    - **Property 9: SOCKS5 UDP 请求头解析 round-trip**
    - 任意目标（IPv4/IPv6/域名）与端口经 `build_socks_udp_header` 封装后由 `parse_socks_udp_header` 还原等价目标与正确载荷偏移
    - `// Feature: network-capability-expansion, Property 9`，≥100 次
    - **Validates: Requirements 4.2, 6.6**

- [x] 8. 阶段 G — 本地日志文件落地（Req 8）
  - [x] 8.1 实现 `logger.rs` 纯函数
    - 新增 `pub(crate) fn format_log_line(ts, level, msg) -> String`（含时间戳 + 级别标签 + 消息）
    - 新增 `pub(crate) fn redact(msg: &str) -> String`（IPv4 后两段掩码、IPv6 前缀外掩码、`C:\Users\<name>\` 的 `<name>` 替换为 `<USER>`）
    - 新增 `pub(crate) fn files_to_prune(existing: &[String], max_files: usize) -> Vec<String>`（保留最新 `max_files` 个，返回应删除的较旧文件）
    - _Requirements: 8.1, 8.2, 8.3_

  - [x] 8.2 实现 Logger sink 与统一日志入口
    - 新增 `Logger { dir, file, max_bytes(≈2MB), max_files(≈5), current_bytes }`：`write(level, msg)` 先脱敏再写文件，`rotate_if_needed()` 达上限滚动并按 `files_to_prune` 裁剪；所有文件操作用 `Result` 静默降级，失败不 panic、不阻断主流程
    - `lib.rs` 的 `AppState` 增加 `log: OnceCell<Arc<Logger>>`；新增 `hmx_log(app, level, msg)`/`Engine.log` 辅助，在既有 `emit("hmx-log", msg)` 之后再调 `Logger::write`
    - 验证点：既有 `emit("hmx-log")` 前端日志面板行为不变（Req 8.6）；写盘失败时仅前端输出（Req 8.5）
    - _Requirements: 8.1, 8.2, 8.5, 8.6_

  - [x] 8.3 实现 `open_log_dir` 命令与前端入口
    - `lib.rs` 新增 `#[tauri::command] fn open_log_dir(app)`（opener 打开 `app_log_dir`）并注册进 `invoke_handler`
    - `lib/api.ts` 新增 `openLogDir()`；设置页/关于页新增「打开日志文件夹」入口（`aria-label` + 中英文案）；`i18n.ts` 新增 `openLogDir` 键，中英对齐
    - 验证点：invoke 失败经既有 `toast("error", ...)` 反馈，不崩溃
    - _Requirements: 8.4_

  - [x] 8.4 编写 `format_log_line` 属性测试（proptest）
    - **Property 19: 日志行格式包含时间戳与级别（format_log_line）**
    - `// Feature: network-capability-expansion, Property 19`，≥100 次
    - **Validates: Requirements 8.1**

  - [x] 8.5 编写 `files_to_prune` 属性测试（proptest）
    - **Property 20: 日志滚动裁剪保留上限（files_to_prune）**
    - `// Feature: network-capability-expansion, Property 20`，≥100 次
    - **Validates: Requirements 8.2**

  - [x] 8.6 编写 `redact` 脱敏属性测试（proptest）
    - **Property 21: 敏感信息脱敏（redact）**
    - 输出不含完整本机 IPv4/IPv6（尾段掩码）且不含原始用户名段（替换 `<USER>`）
    - `// Feature: network-capability-expansion, Property 21`，≥100 次
    - **Validates: Requirements 8.3**

- [x] 9. 检查点 — 确保 E/F/G 全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

- [x] 10. 阶段 H — 诊断抖动/丢包与历史曲线（Req 9/10）
  - [x] 10.1 后端多采样统计与 `LatencyResult` 扩展
    - 新增纯函数 `pub(crate) fn compute_latency_stats(samples: &[Option<u64>]) -> LatencyStats`（`min<=avg`；相等样本 `jitter=0`；`loss_pct=失败/总数`；全失败 `loss_pct=1.0`、`jitter=-1`、`avg/min/latency_ms=-1`；有成功样本 `latency_ms=avg`）
    - `LatencyResult` 增加 `min_ms`、`avg_ms`、`jitter_ms`、`loss_pct`（serde camelCase）；`test_latency` 对每张网卡多次 TCP 握手采样（默认 N=10）并调用 `compute_latency_stats`
    - 验证点：既有 `latency_ms`（成功时=avg）与 `ok` 字段语义对既有前端保持兼容（Req 9.5）
    - _Requirements: 9.1, 9.2, 9.3, 9.5_

  - [x] 10.2 前端诊断展示、历史曲线与报告/PNG
    - `lib/api.ts`：`LatencyResult` 增加 `minMs`/`avgMs`/`jitterMs`/`lossPct`
    - `DiagnosticsPage.tsx`：卡片展示 jitter/loss；新增 `hmx-diag-trend`（`DiagTrend`，带时间戳、上限≈50 裁剪，复用 `appendTrendPoint`/`capTrend`）并以 `AreaChart` 展示趋势曲线；文本报告（`buildReportLines`）与 PNG 导出增列 jitter/loss
    - `i18n.ts`：新增 `diagJitter`/`diagLoss`/`diagTrend` 等键，中英对齐
    - 验证点：既有 `hmx-diag-history`（上次评级）与「应用健康网卡」行为不变（Req 10.6）
    - _Requirements: 9.4, 10.1, 10.2, 10.3, 10.4, 10.5, 10.6_

  - [x] 10.3 编写 `compute_latency_stats` 属性测试（proptest）
    - **Property 22: 延迟统计综合正确性（compute_latency_stats）**
    - `// Feature: network-capability-expansion, Property 22`，≥100 次
    - **Validates: Requirements 9.1, 9.2, 9.3, 9.5**

  - [x] 10.4 编写诊断历史追加/裁剪属性测试（vitest + fast-check）
    - **Property 23: 诊断历史追加与裁剪（appendTrendPoint / capTrend）**
    - 位于 `src/lib/diag.test.ts`；`// Feature: network-capability-expansion, Property 23`，≥100 次
    - **Validates: Requirements 10.1, 10.2**

  - [x] 10.5 编写文本报告指标覆盖属性测试（vitest + fast-check）
    - **Property 24: 文本报告包含全部指标（buildReportLines）**
    - 每张网卡对应行含 RTT / 抖动 / 丢包 / 吞吐四项标签；位于独立测试文件；≥100 次
    - **Validates: Requirements 10.4**

- [x] 11. 阶段 I — 测试落地（既有纯函数回归 + 前端 Req7，依赖 C 测试基建）
  - [x] 11.1 编写既有域名规则与 DNS/头部纯函数测试（proptest + 示例）
    - **Property 4: 既有域名/端口规则匹配（pattern_match）不变**（≥100 次）
    - 同时为 `build_dns_query`、`parse_dns_a`、`dns_skip_name`、`split_host_port`、`build_origin_header`、`find_header`、`Strategy::parse` 补充示例/属性测试
    - `// Feature: network-capability-expansion, Property 4`
    - **Validates: Requirements 6.2, 1.7**

  - [x] 11.2 编写 `RateLimiter` 令牌桶属性测试（proptest）
    - **Property 14: 令牌桶取用不变量（RateLimiter）**；时间以 `Instant`/`Duration` 参数注入，无需真实时钟
    - `// Feature: network-capability-expansion, Property 14`，≥100 次
    - **Validates: Requirements 6.3**

  - [x] 11.3 编写 SWRR / 最少连接调度属性测试（proptest）
    - **Property 15: SWRR 加权轮询长期比例正确**（含最少连接选择最小 `活跃/权重`）
    - `// Feature: network-capability-expansion, Property 15`，≥100 次
    - **Validates: Requirements 6.3**

  - [x] 11.4 编写版本比较属性测试（proptest + vitest/fast-check）
    - **Property 16: 版本比较与逐段数值序一致（version_gt / 前端 version）**
    - 后端 `version_gt` 用 proptest；前端 `src/lib/version.test.ts` 用 fast-check，同序、反自反
    - `// Feature: network-capability-expansion, Property 16`，≥100 次
    - **Validates: Requirements 6.4, 7.4**

  - [x] 11.5 编写 i18n 键对齐属性测试（vitest + fast-check）
    - **Property 17: i18n 中英字典键集合完全一致**
    - 位于 `src/i18n.test.ts`；断言 `zh` 与 `en` 键集合对称差为空；≥100 次
    - **Validates: Requirements 7.2**

  - [x] 11.6 编写 `niceCeil` 属性测试（vitest + fast-check）
    - **Property 18: niceCeil 上界单调性**
    - 结果不小于输入且对不减序列单调不减，落在预期刻度集合上；≥100 次
    - **Validates: Requirements 7.3**

  - [x] 11.7 编写 clipboard 回退与 useModal 行为示例测试（vitest）
    - jsdom 下 mock `navigator.clipboard` 不可用，断言回退到 `document.execCommand`/文本兜底；`useModal` 的 open/close、ESC、焦点行为示例
    - **Validates: Requirements 7.3**

- [x] 12. 最终检查点 — 确保全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- 标注 `*` 的子任务为可选（测试类 + 阶段 F 可选能力），可为更快的 MVP 跳过；`spec 任务执行子代理` 不会自动实现 `*` 子任务，仅实现未标 `*` 的核心任务。
- 阶段 F（SOCKS5 UDP ASSOCIATE）整阶段子任务均为可选（`*`）；若不实现，Req 4 退化为「未启用时以 `REP=0x07` 拒绝 `CMD=0x03`」，该拒绝行为已由既有非 CONNECT 分支覆盖。
- 每个任务引用具体需求子条目并对应设计中的组件/函数签名，可由子代理独立实现；所有 IPv6/UDP/进程新增能力以「泛化 + 分支」引入，既有 IPv4/TCP/DNS/限速/调度/fake-ip 路径的回归检查已写入各任务验证点。
- 属性测试对应 24 条 Correctness Property，每条一个独立子任务：Rust 用 `proptest`（≥100 次，带 `// Feature: network-capability-expansion, Property N` 注释），前端用 `vitest` + `fast-check`（≥100 次）。
- 依赖关系：A 是 B/C/D/E 的前置；C（含测试基建）是 D 与全部属性测试的前置；D 是 E 的前置；F/G/H 相对独立。文档阶段顺序与执行波次（见依赖图）为两个维度：测试基建（3.1/3.2）在依赖图中被安排到早期波次，以供后续各阶段的属性测试依赖。

### 人工实机 / 集成冒烟验证清单（不写自动化测试，由人工验证）

以下验收项涉及真实网卡、Win32 系统调用、TUN 实机、canvas 或运行时集成，不适合 PBT / 单元测试，由人工实机或 1–3 个代表性冒烟场景覆盖：

- IPv6 出站经指定网卡：真实 `setsockopt(IPV6_UNICAST_IF)` + `bind` IPv6 源地址（Req 1.2）
- 网卡 IPv6 地址扫描与展示：`GetAdaptersAddresses(AF_UNSPEC)` 真实枚举（Req 2.1、2.4、2.5）
- TUN 模式 UDP/QUIC 实机中继（HTTP3 站点/应用）（Req 3.1）
- SOCKS5 UDP ASSOCIATE 端口分配与真实转发（Req 4.1，若实现阶段 F）
- 进程反查系统调用：`GetExtendedTcpTable`/`GetExtendedUdpTable`（owning PID）→ 可执行文件名（Req 5.4、5.5）
- 打开日志文件夹：系统文件管理器打开日志目录（Req 8.4）
- 诊断 PNG 导出含 jitter/loss 新列的 canvas 绘制（Req 10.5）
- 既有路径回归实机确认（Req 1.7、3.6、4.4、8.6、9.5、10.6）
- 依赖 Tauri `invoke` 的运行时集成路径（Req 7 边界）

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "1.2", "3.1", "3.2", "3.3", "8.1"] },
    { "id": 1, "tasks": ["1.3", "2.1", "8.2"] },
    { "id": 2, "tasks": ["1.4", "2.3", "4.1", "6.1"] },
    { "id": 3, "tasks": ["2.2", "4.7", "6.2", "8.4"] },
    { "id": 4, "tasks": ["2.4", "6.3", "8.3", "8.5"] },
    { "id": 5, "tasks": ["2.5", "6.4", "8.6"] },
    { "id": 6, "tasks": ["3.4", "4.3", "6.5"] },
    { "id": 7, "tasks": ["4.2", "6.6", "7.3"] },
    { "id": 8, "tasks": ["7.1", "10.4", "11.7"] },
    { "id": 9, "tasks": ["7.2", "10.5", "11.6"] },
    { "id": 10, "tasks": ["10.1"] },
    { "id": 11, "tasks": ["10.2", "2.6"] },
    { "id": 12, "tasks": ["2.7", "11.5"] },
    { "id": 13, "tasks": ["2.8"] },
    { "id": 14, "tasks": ["4.4"] },
    { "id": 15, "tasks": ["4.5"] },
    { "id": 16, "tasks": ["4.6"] },
    { "id": 17, "tasks": ["7.4"] },
    { "id": 18, "tasks": ["10.3"] },
    { "id": 19, "tasks": ["11.1"] },
    { "id": 20, "tasks": ["11.2"] },
    { "id": 21, "tasks": ["11.3"] },
    { "id": 22, "tasks": ["11.4"] }
  ]
}
```
