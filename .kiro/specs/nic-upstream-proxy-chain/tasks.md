# Implementation Plan: 每网卡上游代理链 / 多节点聚合

## Overview

本实现计划将 `design.md` 拆解为可增量交付、可独立验证的编码任务，严格遵循设计文档「实现顺序与依赖」的 7 个阶段（A–G），覆盖 9 条需求与 12 条 Correctness Property：

- **阶段 A 数据模型与透传**（前置基础，Req 1/2/8；B/C/D/E/F 均依赖 A 的类型契约与透传链）
- **阶段 B 上游握手纯函数**（Req 3/9；无 IO，可独立单测，服务 Property 1–6）
- **阶段 C 出口/回退决策纯函数**（Req 6/7/9；无 IO，服务 Property 7–10）
- **阶段 D Upstream_Client 组装**（Req 3/4；依赖 A/B）
- **阶段 E 路由集成**（Req 5/6/7；依赖 C/D，接入既有 `handle_socks`/`handle_http`）
- **阶段 F 前端 UI**（Req 8；相对独立于后端，服务 Property 11/12）
- **阶段 G 测试落地 / 最终检查点**（Req 9）

核心约束：**对既有直连聚合路径零破坏**（Req 5）。上游能力以「在既有『选好网卡 → 连目标』之间插入出口决策分派」的叠加式方式引入，不替换 `connect_via_nic`、`pick_nic`、`resolve_host_dual`、`decide_rule_action`、`dial_dual`、`handle_socks`/`handle_http` 的既有行为。每个任务在验证点写明相应既有路径（直连聚合 / bypass / IPv4 / IPv6 / DNS / 限速 / 调度 / 进程规则 / fake-ip）的零回归回归检查。

编码语言沿用现状：后端 Rust（tokio），属性测试用 `proptest`（每属性 ≥100 次，带 `// Feature: nic-upstream-proxy-chain, Property N` 注释）；前端 TypeScript，属性测试用 `vitest` + `fast-check`（每属性 ≥100 次）。每条 Correctness Property（共 12 条）对应一个独立的属性测试子任务，放在其实现阶段附近以尽早发现回归。

> 说明：不适合自动化测试的验收项（真实上游握手、真实网卡 `IP_UNICAST_IF`/`IPV6_UNICAST_IF` setsockopt/bind、真实 DNS/网卡解析、上游超时实机、UI 持久化与渲染等）不作为编码任务，统一列在文末「人工实机 / 集成冒烟验证清单」中，由人工验证。

## Tasks

- [x] 1. 阶段 A — 数据模型与透传（前置基础，服务 Req 1/2/8）
  - [x] 1.1 新增上游数据结构（`engine.rs`）
    - 改动点：新增 `UpstreamProxy { id, kind, host, port, username?, password?, label }`（`#[serde(rename_all="camelCase")]`，`username`/`password`/`label` 带 `#[serde(default)]`）；新增 `UpstreamBinding { if_index, upstream_ids: Vec<String> }`（camelCase）；新增 `pub(crate) enum FallbackPolicy { Direct, Fail }`（`Copy`）
    - 改动点：`Engine` 结构新增字段 `upstream_chain: bool`（默认 `false`）、`upstreams: HashMap<String, UpstreamProxy>`、`upstream_bindings: HashMap<u32, Vec<String>>`、`upstream_fallback: FallbackPolicy`、`upstream_timeout: std::time::Duration`（缺省 ≤10s）
    - 验证点：仅新增字段与类型，既有 `Engine` 字段与既有直连聚合 / IPv4 / IPv6 / DNS / 限速 / 调度 / 进程规则 / fake-ip 分支不引用新字段，行为完全不变
    - _Requirements: 1.1, 1.2, 1.5, 2.1, 5.5_

  - [x] 1.2 扩展 `engine::start` 签名并构建映射 + 剔除悬空 + `lib.rs` 透传
    - 改动点：`engine::start(...)` 增参 `upstreams: Vec<UpstreamProxy>`、`upstream_bindings: Vec<UpstreamBinding>`、`upstream_chain: bool`、`upstream_fallback: String`；`start` 内将 `upstreams` Vec 构建为 `HashMap<id, UpstreamProxy>`，将 `upstream_bindings` 构建为 `HashMap<if_index, Vec<UpstreamId>>` 并**剔除引用了不存在条目的 id**（复用阶段 C 的 `sanitize_bindings`，Req 2.6），`upstream_fallback` 解析为 `FallbackPolicy`（未知值默认 `Direct`）
    - 改动点：`lib.rs` 的 `start_boost` 命令增参 `upstreams`/`upstream_bindings`/`upstream_chain`/`upstream_fallback` 并透传给 `engine::start`
    - 验证点：不传上游配置（默认空列表 + `upstream_chain=false`）时，`start` 构建结果为空映射且总开关关，既有启动路径与直连聚合行为与升级前一致；`RouteRuleDef`/`bypass`/`ip_version`/`udp_associate` 等既有入参解析不变
    - _Requirements: 1.1, 2.1, 2.6, 5.5_

  - [x] 1.3 前端类型契约、store 与透传（`api.ts` / `store.tsx` / `App.tsx`）
    - 改动点：`lib/api.ts` 新增 TS 类型 `UpstreamProxy { id; kind: "socks5"|"http"; host; port; username?; password?; label }` 与 `UpstreamBinding { ifIndex; upstreamIds: string[] }`；`startBoost` 增参 `upstreams`、`upstreamBindings`、`upstreamChain`、`upstreamFallback` 并在 `invoke("start_boost", {...})` 中透传
    - 改动点：`store.tsx` 的 `Settings` 增 `upstreamChain: boolean`（默认 `false`）、`upstreamFallback: "direct"|"fail"`（默认 `"direct"`）；上游节点列表（key `hmx-upstreams`）与映射（key `hmx-upstream-bindings`）作为独立持久化状态
    - 改动点：`App.tsx` 持久化上游列表 / 映射并在 `onBoost` 时并入 `startBoost` 调用
    - 验证点：未配置上游时 `startBoost` 以空列表 + `upstreamChain=false` 调用，既有启动与设置项（IP 版本 / 限速 / 规则 / udpAssociate）行为不变
    - _Requirements: 8.1, 8.3, 8.4_

- [x] 2. 阶段 B — 上游握手纯函数（依赖 A，服务 Req 3/9，无 IO）
  - [x] 2.1 实现 SOCKS5 版本协商与用户名/密码子协商纯函数
    - 改动点：新增 `build_socks5_greeting(with_auth: bool) -> Vec<u8>`（无认证 `[0x05,0x01,0x00]`；有认证声明含 `0x02`）与 `parse_socks5_method_reply(&[u8]) -> Option<u8>`
    - 改动点：新增 `build_socks5_userpass(username, password) -> Vec<u8>`（RFC 1929：`VER(0x01) ULEN USER PLEN PASS`）与 `parse_socks5_userpass_reply(&[u8]) -> Option<u8>`（返回 STATUS，`0x00` 成功）
    - 验证点：均为纯函数新增，不触碰既有 SOCKS5 入站 `handle_socks` 解析路径；截断/畸形输入返回 `None` 且绝不 panic
    - _Requirements: 3.3, 3.6, 9.1_

  - [x]* 2.2 编写 SOCKS5 用户名/密码子协商与认证声明属性测试（proptest）
    - **Property 3: SOCKS5 用户名/密码子协商 round-trip 与认证声明**
    - 随机用户名/密码（长度 1..=255）经 `build_socks5_userpass` 封装后可还原；`parse_socks5_userpass_reply` 对 `0x00` 判成功、非零判失败；`build_socks5_greeting(true)` 含 `0x02`、`false` 不含 `0x02`
    - `// Feature: nic-upstream-proxy-chain, Property 3`，≥100 次
    - **Validates: Requirements 3.3, 9.2**

  - [x] 2.3 实现 SOCKS5 CONNECT 请求/应答纯函数
    - 改动点：新增 `pub(crate) enum ConnectTarget { V4(Ipv4Addr,u16), V6(Ipv6Addr,u16), Domain(String,u16) }`
    - 改动点：新增 `build_socks5_connect_req(&ConnectTarget) -> Vec<u8>`（`VER(0x05) CMD(0x01) RSV(0x00) ATYP ADDR PORT`）与其互逆解析 `parse_socks5_connect_req(&[u8]) -> Option<ConnectTarget>`；新增 `parse_socks5_connect_reply(&[u8]) -> Option<(u8 rep, usize consumed)>`（当且仅当 `rep==0x00` 判成功）
    - 验证点：`ATYP=0x01/0x04/0x03` 语义与既有入站 SOCKS5 一致；任意/截断字节解析返回 `None`，不误判成功、不 panic
    - _Requirements: 3.1, 3.5, 3.6, 9.1, 9.2_

  - [ ]* 2.4 编写 SOCKS5 CONNECT 请求 round-trip 与健壮性属性测试（proptest）
    - **Property 1: SOCKS5 CONNECT 请求构造/解析 round-trip 与健壮性**
    - 任意 IPv4/IPv6/域名 + 端口经 `build_socks5_connect_req` 构造后由 `parse_socks5_connect_req` 还原等价目标（域名原样、端口一致、ATYP 正确）；任意字节序列解析绝不 panic（非法/截断返回 `None`）
    - `// Feature: nic-upstream-proxy-chain, Property 1`，≥100 次
    - **Validates: Requirements 3.1, 3.5, 3.6, 9.1, 9.2**

  - [ ]* 2.5 编写 SOCKS5 CONNECT 应答 REP 判定属性测试（proptest）
    - **Property 2: SOCKS5 CONNECT 应答 REP 判定**
    - 合法应答字节（`VER REP RSV ATYP BND.ADDR BND.PORT`）经 `parse_socks5_connect_reply` 返回的 `rep` 等于应答 REP，成功判定当且仅当 `rep==0x00`；截断/非法应答返回 `None`（不误判成功）
    - `// Feature: nic-upstream-proxy-chain, Property 2`，≥100 次
    - **Validates: Requirements 3.1, 3.6**

  - [x] 2.6 实现 HTTP CONNECT 请求行、状态行解析与 Basic 认证纯函数
    - 改动点：新增 `build_http_connect_req(host, port, auth: Option<(&str,&str)>) -> Vec<u8>`（`CONNECT <host>:<port> HTTP/1.1\r\nHost: <host>:<port>\r\n` + 可选 `Proxy-Authorization: Basic <b64>\r\n` + `\r\n`）
    - 改动点：新增 `parse_http_status_line(line: &str) -> Option<u16>`（当且仅当 2xx 判成功，非法格式 `None`）与 `basic_auth_b64(username, password) -> String`（`Base64("<user>:<pass>")`，采用成熟 Base64 实现，不自研）
    - 验证点：纯函数新增，不改动既有 HTTP 入站 `handle_http` 解析路径；畸形状态行返回 `None`，不误判成功
    - _Requirements: 3.2, 3.4, 3.5, 3.6, 9.1_

  - [ ]* 2.7 编写 HTTP CONNECT 请求行 round-trip 属性测试（proptest）
    - **Property 4: HTTP CONNECT 请求行构造 round-trip**
    - 任意 host + 端口（及可选认证）经 `build_http_connect_req` 生成首行形如 `CONNECT <host>:<port> HTTP/1.1`，解析出的 host/port 等于输入；提供认证时恰含一行 `Proxy-Authorization: Basic <b64>`，未提供时不含该头
    - `// Feature: nic-upstream-proxy-chain, Property 4`，≥100 次
    - **Validates: Requirements 3.2, 3.5, 9.1, 9.2**

  - [ ]* 2.8 编写 HTTP 状态行解析与 2xx 判定属性测试（proptest）
    - **Property 5: HTTP 状态行解析与 2xx 判定**
    - 任意合法状态行 `HTTP/1.x <code> <reason>` 经 `parse_http_status_line` 返回状态码等于 `<code>`，成功判定当且仅当状态码 ∈ [200,299]；非法/畸形状态行返回 `None`（不误判成功）
    - `// Feature: nic-upstream-proxy-chain, Property 5`，≥100 次
    - **Validates: Requirements 3.2, 3.6**

  - [ ]* 2.9 编写 HTTP Basic 认证 Base64 round-trip 属性测试（proptest）
    - **Property 6: HTTP Basic 认证 Base64 round-trip**
    - 任意用户名/密码，`basic_auth_b64(user, pass)` 输出经标准 Base64 解码后等于字节串 `"<user>:<pass>"`
    - `// Feature: nic-upstream-proxy-chain, Property 6`，≥100 次
    - **Validates: Requirements 3.4**

- [x] 3. 阶段 C — 出口 / 回退决策纯函数（依赖 A，服务 Req 6/7/9，无 IO）
  - [x] 3.1 实现出口决策与一网卡多上游选择及悬空引用剔除纯函数
    - 改动点：新增 `pub(crate) enum Egress { Direct, ViaUpstream(String) }`
    - 改动点：新增 `pick_upstream_for_nic(bindings, if_index, sched_idx) -> Option<String>`（空/不存在 => `None`；`len==1` => 唯一 id；`len>1` => `sched_idx % len` 轮转）
    - 改动点：新增 `decide_egress(upstream_chain, if_index, bindings, is_bypass, sched_idx) -> Egress`（总开关关 => `Direct`；`is_bypass` => `Direct`；无绑定/空绑定 => `Direct`；否则 `ViaUpstream(pick_upstream_for_nic)`）
    - 改动点：新增 `sanitize_bindings(upstreams, bindings)`（剔除引用不存在 id 的绑定，供 `engine::start` 与 `decide_egress` 复用，Req 2.6）
    - 验证点：均为纯函数，不触碰既有 `pick_nic` / bypass / 调度 / 进程规则路径；`upstream_chain=false` 时 `decide_egress` 恒 `Direct`（零回归）
    - _Requirements: 2.2, 2.3, 2.4, 2.6, 5.1, 5.2, 5.3, 7.1, 7.2, 7.3, 7.4, 9.3_

  - [x] 3.2 实现回退决策状态机纯函数
    - 改动点：新增 `pub(crate) enum FallbackStep { TryUpstream(String), Direct, Fail }`
    - 改动点：新增 `next_fallback(tried: &[String], nic_upstreams: &[String], policy: FallbackPolicy) -> FallbackStep`（存在未试上游 => `TryUpstream(下一个未试 id)`；试尽 + `Direct` => `Direct`；试尽 + `Fail` => `Fail`）
    - 验证点：纯函数新增，与 IO 拨号解耦，不影响既有直连聚合 / 调度路径
    - _Requirements: 6.2, 6.3, 6.4, 9.4_

  - [ ]* 3.3 编写一网卡上游选择综合属性测试（proptest）
    - **Property 7: 一网卡上游选择综合正确性（pick_upstream_for_nic）**
    - 无绑定/空 => `None`；非空 => 返回值 ∈ 列表；`len==1` 恒返回唯一 id；`len>1` 连续 `sched_idx` 轮转全覆盖；共享映射（不同 `if_index` 指向同一 id）各自正确选出
    - `// Feature: nic-upstream-proxy-chain, Property 7`，≥100 次
    - **Validates: Requirements 2.2, 2.3, 2.4, 9.3**

  - [ ]* 3.4 编写悬空上游引用剔除属性测试（proptest）
    - **Property 8: 悬空上游引用剔除（sanitize_bindings）**
    - 构建后的 `upstream_bindings` 不含任何不属于条目全集的 id；引用存在条目的绑定全保留；剔除后 `upstream_ids` 为空的网卡等价于未绑定
    - `// Feature: nic-upstream-proxy-chain, Property 8`，≥100 次
    - **Validates: Requirements 2.6**

  - [ ]* 3.5 编写出口决策综合属性测试（proptest）
    - **Property 9: 出口决策综合正确性（decide_egress）**
    - 总开关 `false` 恒 `Direct`（零回归）；`is_bypass==true` 恒 `Direct`（最高优先）；总开关 `true`+非 bypass+非空绑定 => `ViaUpstream(id)` 且 id ∈ 该网卡绑定集合；总开关 `true`+非 bypass+无绑定/空 => `Direct`
    - `// Feature: nic-upstream-proxy-chain, Property 9`，≥100 次
    - **Validates: Requirements 5.1, 5.2, 5.3, 7.1, 7.2, 7.3, 7.4**

  - [ ]* 3.6 编写回退决策综合属性测试（proptest）
    - **Property 10: 回退决策综合正确性（next_fallback）**
    - 存在未试上游 => `TryUpstream(下一个未试 id)`；试尽 + `Direct` => `Direct`；试尽 + `Fail` => `Fail`
    - `// Feature: nic-upstream-proxy-chain, Property 10`，≥100 次
    - **Validates: Requirements 6.2, 6.3, 6.4, 9.4**

- [x] 4. 检查点 — 确保阶段 A/B/C 全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

- [x] 5. 阶段 D — Upstream_Client 组装（依赖 A/B，服务 Req 3/4）
  - [x] 5.1 实现 `connect_via_upstream`（经网卡出口连上游 + 握手 + 超时包裹）
    - 改动点：新增 `async fn connect_via_upstream(engine, nic, upstream, host, port) -> io::Result<TcpStream>`：① 经 `nic` 调用既有 `resolve_host_dual` 解析上游域名（Req 4.2）；② 经既有 `dial_dual`/`connect_via_nic` 以该网卡物理出口连上游地址（复用 Egress_Binding，不改其底层，Req 4.1/4.5）；③ 按 `upstream.kind` 调用阶段 B 纯函数执行 SOCKS5 / HTTP CONNECT 握手（含可选认证），成功判定依 Property 2/5；④ 整体以 `tokio::time::timeout(engine.upstream_timeout)` 包裹（Req 6.1），任一步失败/超时返回 `Err` 交回退处理
    - 改动点：上游失败 / 无对应地址族源地址（Req 4.4）时记录含 `upstream.label` 与原因的可读日志（复用既有日志入口）
    - 验证点：仅新增函数且仅在总开关启用且走上游时被调用；`connect_via_nic`/`resolve_host_dual`/`dial_dual` 底层实现不变，既有直连聚合 / IPv4 / IPv6 / DNS 路径零回归
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 4.1, 4.2, 4.4, 4.5, 6.1_

- [x] 6. 阶段 E — 路由集成（依赖 C/D，服务 Req 5/6/7）
  - [x] 6.1 实现出口分派 + 回退循环 `establish_target`
    - 改动点：新增 `async fn establish_target(engine, nic, host, port, literal_ip, dual_addrs, is_bypass) -> io::Result<TcpStream>`：调用 `decide_egress` 判定 —— `Direct` 走既有 `dial_dual`/`connect_via_nic`（字节流与现状完全一致）；`ViaUpstream` 以 `next_fallback` 驱动，在该网卡上游集合内逐个 `connect_via_upstream`，维护 `tried` 集合，全试尽后按 `upstream_fallback`：`Direct` 回退既有直连、`Fail` 返回 `Err` 供调用方回标准错误应答（Req 6.2/6.3/6.4）
    - 改动点：`sched_idx` 复用既有连接计数（如 `nic.active` 或全局 `conn_id`）取模，保证一网卡多上游轮转；每次上游失败/回退记录含上游标签与原因的日志（Req 6.5）
    - 验证点：`decide_egress` 返回 `Direct` 时 `establish_target` 内部完全走既有直连路径，限速 / 调度 / fake-ip / DNS 行为不变
    - _Requirements: 5.1, 5.2, 5.3, 6.2, 6.3, 6.4, 6.5, 7.2, 7.3, 7.4_

  - [x] 6.2 在 `handle_socks` 三处连接点接入 `establish_target`
    - 改动点：`handle_socks` 的 `ATYP=0x01`（字面 IPv4）、`ATYP=0x04`（IPv6）、域名三处连接点统一改为经 `establish_target`；传入 `is_bypass = engine.is_bypass(host)` 结果（bypass 恒直连，Req 7.1）；上游全失败且策略 `Fail` 时回标准 SOCKS5 错误应答（`REP=0x05` 等）
    - 验证点：总开关关 / 命中 bypass / 网卡无绑定时，三处连接点字节流与既有 SOCKS5 直连聚合完全一致（IPv4/IPv6/域名/DNS 零回归）
    - _Requirements: 5.1, 5.3, 5.4, 7.1, 7.2, 7.3, 7.4_

  - [x] 6.3 在 `handle_http` 连接点接入 `establish_target`
    - 改动点：`handle_http` 既有 `connect_via_nic(&nic, dst)` 连接点改为经 `establish_target`（传入 `is_bypass`）；上游全失败且策略 `Fail` 时回 HTTP 失败应答（如 `502`），复用既有错误应答分支
    - 验证点：总开关关 / 命中 bypass / 网卡无绑定时，HTTP 隧道字节流与既有直连聚合一致；既有 CONNECT 转发、限速、调度行为不变
    - _Requirements: 5.1, 5.3, 5.4, 7.1, 7.4_

- [x] 7. 检查点 — 确保阶段 D/E 集成与既有路径零回归
  - Ensure all tests pass, ask the user if questions arise.

- [x] 8. 阶段 F — 前端上游代理链 UI（相对独立，服务 Req 8）
  - [x] 8.1 析出前端上游纯逻辑 `lib/upstream.ts`
    - 改动点：新增 `src/lib/upstream.ts`，导出 `validateUpstream(input)`（校验：`host` 非空且 ≤253、`port ∈ [1,65535]`、`kind ∈ {socks5,http}`、配置认证时用户名/密码长度 ∈ [1,255]；返回通过与否 + 失败字段标记）与删除条目时清理映射引用的纯函数（从所有 `UpstreamBinding.upstreamIds` 移除该 id，删除后无悬空引用）
    - 验证点：纯逻辑不含 DOM / `invoke`，可被 vitest 直接导入
    - _Requirements: 1.6, 2.6, 9.6_

  - [x] 8.2 `SettingsPage.tsx` 新增「上游代理链」分区
    - 改动点：新增总开关（`Switch` + `aria-label`）；上游节点编辑器（新增/编辑/删除，字段 类型 socks5/http、host、port、可选用户名/密码、label，接入 `validateUpstream` 字段级校验与保留输入 + 高亮失败字段，达 128 上限时禁用新增并提示，Req 1.6/1.7）；网卡↔上游映射编辑器（每张聚合网卡多选绑定上游，允许共享映射）；回退策略 `Segmented`（回退直连 / 失败）；回环上游提示（host 为 `127.0.0.1`/`localhost` 时给出「无法叠加」提示）
    - 改动点：删除条目时调用 8.1 清理逻辑同步移除映射引用；上游节点 id 用 `crypto.randomUUID()` 生成且删除后不复用（Req 1.8）
    - 验证点：既有设置项（IP 版本 / 限速 / 规则 / udpAssociate / bypass）分区与交互不受影响
    - _Requirements: 1.3, 1.4, 1.6, 1.7, 1.8, 2.1, 2.5, 8.1, 8.2, 8.3, 8.4, 8.6_

  - [x] 8.3 `i18n.ts` 新增上游代理链文案键（中英对齐）
    - 改动点：新增 `upstreamTitle`/`upstreamHint`/`upstreamEnable`/`upstreamAddNode`/`upstreamKind`/`upstreamHost`/`upstreamPort`/`upstreamUser`/`upstreamPass`/`upstreamLabel`/`upstreamBinding`/`upstreamFallback`/`upstreamFallbackDirect`/`upstreamFallbackFail`/`upstreamLimitReached`/`upstreamInvalidHost`/`upstreamInvalidPort`/`upstreamInvalidKind`/`upstreamLoopbackWarn` 等键，`zh` 与 `en` 双字典严格对齐
    - 验证点：切换英文时上游分区无中文残留；既有键不改动
    - _Requirements: 8.5_

  - [ ]* 8.4 编写上游条目校验属性测试（vitest + fast-check）
    - **Property 11: 上游条目校验综合正确性（validateUpstream）**
    - 位于 `src/lib/upstream.test.ts`；`fast-check` 生成随机 host 长度 / 端口 / 类型 / 认证组合，断言通过当且仅当 host 非空且 ≤253、`port ∈ [1,65535]`、`kind ∈ {socks5,http}`、（配置认证时）用户名/密码长度 ∈ [1,255]，违反时标记对应失败字段
    - `// Feature: nic-upstream-proxy-chain, Property 11`，≥100 次（`{ numRuns: 100 }`）
    - **Validates: Requirements 1.2, 1.6, 9.6**

  - [ ]* 8.5 编写 i18n 键对齐属性测试（vitest + fast-check）
    - **Property 12: i18n 中英字典键集合完全一致**
    - 位于 `src/i18n.test.ts`（复用/追加）；导入 `zh`/`en` 字典，断言键集合对称差为空，覆盖上游代理链全部新增键
    - `// Feature: nic-upstream-proxy-chain, Property 12`，≥100 次
    - **Validates: Requirements 8.5, 9.6**

- [x] 9. 阶段 G — 最终检查点，确保全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- 标注 `*` 的子任务为可选（全部为属性测试类），可为更快的 MVP 跳过；`spec 任务执行子代理` 不会自动实现 `*` 子任务，仅实现未标 `*` 的核心任务。
- 每个任务引用具体需求子条目并对应设计中的组件/函数签名，可由子代理独立实现；上游能力以「叠加式分派」引入，既有直连聚合 / bypass / IPv4 / IPv6 / DNS / 限速 / 调度 / 进程规则 / fake-ip 路径的零回归检查已写入各任务验证点（Req 5）。
- 属性测试对应 12 条 Correctness Property，每条一个独立子任务：Rust 用 `proptest`（≥100 次，带 `// Feature: nic-upstream-proxy-chain, Property N` 注释），前端用 `vitest` + `fast-check`（≥100 次）。`proptest` 已作为既有 dev-dependency 存在，无需重复引入。
- 依赖关系：A 是 B/C/D/E/F 的前置（类型契约与透传链）；B/C 为无 IO 纯函数，D 组装依赖 B、E 集成依赖 C/D；F 前端相对独立可与后端并行。纯函数化确保 `cargo test` 与前端 `vitest --run` 独立于 GUI 与网络（Req 9.7）。

### 人工实机 / 集成冒烟验证清单（不写自动化测试，由人工验证）

以下验收项涉及真实 socket、Win32 系统调用、真实上游握手或运行时集成，不适合 PBT / 单元测试，由人工实机或 1–3 个代表性冒烟场景覆盖：

- 真实上游握手：真实 `socks5`（带/不带认证）与 `http`（带/不带 Basic 认证）上游，验证 CONNECT 隧道建立成功并拉取真实目标数据（Req 3.1/3.2/3.3/3.4）
- 网卡绑定与物理出口：多网卡各绑定不同上游，实机抓包/上游侧观察确认「网卡A→上游1」「网卡B→上游2」经各自物理出口而非回环，并行叠加带宽（Req 4.1/4.3/4.5）
- 上游域名解析经网卡：上游 host 填域名，确认经所选网卡出口解析并建连（Req 4.2）
- 系统调用/地址族回退：为绑定上游的网卡制造无对应地址族源地址场景，确认记录日志并回退（Req 4.4）
- 回退实机验证：故意配置不可用上游，验证 ≤10s 超时后按「回退直连」直连成功、按「失败」返回错误应答，日志含上游标签（Req 6.1/6.3/6.4/6.5）
- 零回归实机确认：总开关关闭时既有直连聚合 / bypass / 按网卡·进程规则 / 调度策略 / 限速 / fake-ip / IPv4·IPv6·TCP·DNS 路径与升级前一致（Req 5.1/5.4/5.5/7.5）
- 持久化与 UI：上游条目增删改、映射编辑、总开关切换的持久化与列表刷新（Req 1.3/1.4/2.1/2.5/8.1/8.2/8.3/8.4）；`aria-label` 存在性（Req 8.6）；128 上限提示（Req 1.7）；id 唯一稳定不复用（Req 1.8）
- 依赖 Tauri `invoke` 的运行时集成路径（Req 9 边界）

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1", "8.1", "8.3"] },
    { "id": 1, "tasks": ["1.2", "1.3", "8.2"] },
    { "id": 2, "tasks": ["2.1", "8.4", "8.5"] },
    { "id": 3, "tasks": ["2.3", "3.1"] },
    { "id": 4, "tasks": ["2.6", "3.2"] },
    { "id": 5, "tasks": ["2.2", "2.4", "2.5"] },
    { "id": 6, "tasks": ["2.7", "2.8", "2.9"] },
    { "id": 7, "tasks": ["3.3", "3.4", "3.5", "3.6"] },
    { "id": 8, "tasks": ["5.1"] },
    { "id": 9, "tasks": ["6.1"] },
    { "id": 10, "tasks": ["6.2"] },
    { "id": 11, "tasks": ["6.3"] }
  ]
}
```
