# Implementation Plan: 专业化差异化与稳定性加固

## Overview

本实现计划将 `design.md` 拆解为可增量交付、可独立验证的编码任务，严格遵循设计文档的实现顺序与 10 条 Correctness Property，覆盖 13 条需求：

- **阶段 A 编译配置 + 数据结构 + 端到端透传**（前置基础，Req 1/5/7/8/13）
- **阶段 B 健康探测状态机 + 加权优选纯函数与接线**（Req 1/2）
- **阶段 C 订阅解析纯函数（后端 + 前端镜像）**（Req 4）
- **阶段 D 分流决策纯函数 + 模拟器**（Req 3）
- **阶段 E 每网卡 DNS/DoH**（Req 7）
- **阶段 F 系统代理防泄漏看门狗**（Req 5）
- **阶段 G 崩溃日志 + 稳定性加固（panic 隔离 + 上限保护）**（Req 8/9）
- **阶段 H 高效转发形式化**（Req 6）
- **阶段 I 前端 UI + i18n 中英对齐**（Req 12）
- **阶段 J 端到端本地 mock 测试基建**（Req 10，非可选核心）
- **阶段 K 补齐既有可选属性测试**（Req 11）
- **阶段 L 最终检查点**

核心约束：**对既有全部能力零回归**（Req 13）。本次每项能力默认关闭 / 默认旁路，未启用时 `engine.rs` 既有路径字节流不变。可测核心逻辑一律析出为不依赖 IO 的纯函数（Req 11）。

编码约束：仅 Windows 10/11；所有监听端点仅 `127.0.0.1`；后端属性测试用 `proptest`（每属性 ≥100 次，带 `// Feature: pro-differentiation-and-hardening, Property N` 注释）；前端用 `vitest` + `fast-check`（每属性 ≥100 次）；端到端集成测试用本地 mock（tokio 监听 `127.0.0.1`），不依赖真实公网 / 真实网卡；release profile 由 `panic = "abort"` 改为 `panic = "unwind"`（连接级 panic 隔离的前提）。

> 说明：不适合自动化测试的验收项（真实公网上游握手、真实多物理网卡叠加、真实网卡 setsockopt/bind、真实强杀后的启动补偿、GUI 渲染与持久化、CPU 下降对比等）不作为编码任务，统一列在文末「人工实机 / 集成冒烟验证清单」中。

- 标注 `*` 的子任务为可选（属性测试类），`spec 任务执行子代理` 不会自动实现；阶段 J 端到端 mock 集成测试为**非可选**核心任务。

## Tasks

- [x] 1. 阶段 A — 编译配置 + 数据结构 + 端到端透传（前置基础）
  - [x] 1.1 编译配置与后端数据结构（`Cargo.toml` / `engine.rs`）
    - 改动点：`Cargo.toml` 的 `[profile.release]` 将 `panic = "abort"` 改为 `panic = "unwind"`（连接级 panic 隔离前提，Req 8）
    - 改动点：`engine.rs` 新增 `enum HealthState { Healthy, CircuitOpen }`、`struct UpstreamHealth { state, consecutive_failures, last_latency_ms, opened_at_ms }`（含 `Default`）、`struct HealthConfig { enabled, interval_ms, timeout_ms, fail_threshold, cooldown_ms }`（`Copy`）、`enum ProbeEvent { Success(u64), Failure }`、`struct PerNicDns { kind, endpoint }`、`enum DnsKind { Plain, Doh }`
    - 改动点：`Engine` 新增字段 `health_cfg: HealthConfig`、`upstream_health: Arc<Mutex<HashMap<String, UpstreamHealth>>>`、`per_nic_dns: HashMap<u32, PerNicDns>`、`conn_cap: usize`、`task_cap: usize`、`active_conns: Arc<AtomicI64>`
    - 验证点：仅新增字段/类型/改 profile；既有 Engine 字段与既有直连聚合/上游链/IPv4/IPv6/DNS/限速/调度/进程规则/fake-ip 分支不引用新字段，行为不变；`cargo check` 通过
    - _Requirements: 1.1, 1.7, 7.1, 8.1, 8.6, 13.1_

  - [x] 1.2 扩展 `engine::start` 签名并透传 + `lib.rs` 命令透传
    - 改动点：`engine::start(...)` 末尾增参 `health_cfg: HealthConfig`、`per_nic_dns: Vec<(u32, PerNicDns)>`、`conn_cap: usize`、`task_cap: usize`、`proxy_guardian: bool`；`start` 内构建 `per_nic_dns` 为 `HashMap<u32, PerNicDns>`、初始化 `upstream_health` 空表、`active_conns=0`；未提供时全部取默认（health 关闭、空 DNS 映射、合理默认上限）
    - 改动点：`lib.rs` 的 `start_boost` 命令增参并透传；`AppState` 视需要持有 `proxy_guardian` 标志与守护目录
    - 验证点：不传新配置（health 关闭、空 DNS、默认上限）时构建结果与升级前等价，既有启动路径行为不变；`cargo check` 通过
    - _Requirements: 1.1, 7.1, 8.2, 8.6, 13.1, 13.2_

  - [x] 1.3 前端类型契约、store 与透传（`api.ts` / `store.tsx` / `App.tsx`）
    - 改动点：`lib/api.ts` 新增类型 `HealthCfg`、`PerNicDnsCfg`；`startBoost` 增参 `healthCfg`、`perNicDns`、`connCap`、`taskCap`、`proxyGuardian` 并在 `invoke("start_boost", {...})` 透传（camelCase）
    - 改动点：`store.tsx` 的 `Settings` 增 `healthCfg`（默认 enabled=false + 缺省 30000/5000/3/60000）、`connCap`（默认 4096）、`taskCap`（默认 64）、`proxyGuardian`（默认 true，但正常路径等价既有）；每网卡 DNS 映射（key `hmx-per-nic-dns`）作为独立持久化状态
    - 改动点：`App.tsx` 的 `onBoost` 并入新参数
    - 验证点：未配置新项时以默认值调用，既有启动与设置项行为不变；`tsc --noEmit` 通过
    - _Requirements: 12.1, 12.3, 13.1, 13.2_

- [x] 2. 阶段 B — 健康探测状态机 + 加权优选（`engine.rs`，Req 1/2）
  - [x] 2.1 实现健康状态机纯函数
    - 改动点：新增 `health_transition(cur, event, cfg, now_ms) -> UpstreamHealth`（Success=>Healthy+清零+更新延迟；Failure 达阈值=>CircuitOpen+记 opened_at；未达=>Healthy+计数+1；CircuitOpen 的 Success=>Healthy）、`should_half_open(&h, cfg, now_ms) -> bool`、`is_selectable(&h, cfg, now_ms) -> bool`
    - 验证点：均纯函数无 IO，对任意输入不 panic；不触碰既有路径
    - _Requirements: 1.2, 1.3, 1.4, 1.5, 2.6_

  - [x] 2.2 编写健康状态机迁移属性测试（proptest）
    - **Property 1: 健康状态机迁移正确性（health_transition）**
    - `// Feature: pro-differentiation-and-hardening, Property 1`，≥100 次
    - **Validates: Requirements 1.2, 1.3, 1.5**

  - [x] 2.3 编写冷却期与半开/候选判定属性测试（proptest）
    - **Property 2: 冷却期与半开判定（should_half_open / is_selectable）**
    - `// Feature: pro-differentiation-and-hardening, Property 2`，≥100 次
    - **Validates: Requirements 1.4, 2.6**

  - [x] 2.4 实现上游加权优选纯函数
    - 改动点：新增 `select_weighted_upstream(candidates: &[String], latencies: &[Option<u64>], sched_idx: usize) -> Option<String>`（候选空=>None；返回值恒 ∈ candidates；权重 ∝ 1/(latency+base)，用 sched_idx 在加权分布上确定性取样）
    - 验证点：纯函数无 IO，不引入候选集合外元素；不触碰既有 `pick_upstream_for_nic`
    - _Requirements: 2.1, 2.6_

  - [x] 2.5 编写加权优选属性测试（proptest）
    - **Property 3: 加权优选恒在候选集内且排除熔断（select_weighted_upstream）**
    - `// Feature: pro-differentiation-and-hardening, Property 3`，≥100 次
    - **Validates: Requirements 2.1, 2.6**

  - [x] 2.6 后台健康探测任务 + `establish_target` 接入优选
    - 改动点：新增后台探测任务（`start` 内当 `health_cfg.enabled` 时 spawn）：按 `interval_ms` 对被引用上游经其所属网卡 Egress_Binding 拨号探测（复用 `connect_via_upstream` 或轻量握手），以 `tokio::time::timeout(timeout_ms)` 包裹，结果喂 `health_transition` 更新 `upstream_health`；状态变化记可读日志（Req 1.8）；受 `task_cap` 限制并发
    - 改动点：`establish_target` 的上游首选来源：`health_cfg.enabled` 时用「`is_selectable` 过滤候选 + `select_weighted_upstream`」，否则维持既有 `pick_upstream_for_nic`；其余 `next_fallback` 回退循环、`tried`、回退直连/失败完全不变
    - 验证点：`health_cfg.enabled=false` 时 `establish_target` 与上游代理链现状逐字节一致（零回归）；未引用的上游不被探测
    - _Requirements: 1.1, 1.6, 1.8, 2.1, 2.2, 2.3, 2.4, 2.5_

- [ ] 3. 阶段 C — 订阅解析（Req 4）
  - [x] 3.1 新增 `subscription.rs` 订阅解析纯函数（后端）
    - 改动点：新增模块 `src-tauri/src/subscription.rs`（在 `lib.rs` `mod subscription;`）：`struct ImportResult { candidates: Vec<UpstreamProxy>, ignored_unsupported: usize }`、`parse_subscription(&str) -> ImportResult`、`try_base64_decode(&str) -> Option<String>`、`parse_clash_proxies(&str) -> ImportResult`（仅取 type∈{socks5,http} 的 name/server/port/username/password）、`parse_share_link(&str) -> Option<UpstreamProxy>`（socks5://、http(s) 代理 => 候选；ss/vmess/trojan/hysteria => None 计入 ignored）
    - 验证点：对任意字节输入不 panic；无受支持节点返回空候选；非 socks5/http 计入 ignored；`cargo check` 通过
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 4.8_

  - [ ] 3.2 编写订阅解析属性测试（proptest）
    - **Property 4: 订阅解析健壮性与 round-trip（parse_subscription / parse_share_link）**
    - `// Feature: pro-differentiation-and-hardening, Property 4`，≥100 次
    - **Validates: Requirements 4.1, 4.2, 4.3, 4.4, 4.8**

  - [x] 3.3 前端订阅解析镜像 `lib/subscription.ts`
    - 改动点：新增 `src/lib/subscription.ts`，导出 `parseSubscription(input): { candidates: UpstreamProxy[]; ignoredUnsupported: number }` 及子函数，语义与后端一致，供 UI 即时预览；纯逻辑不含 DOM/invoke
    - 验证点：`tsc --noEmit` 通过；可被 vitest 直接导入
    - _Requirements: 4.1, 4.4, 4.5_

  - [ ] 3.4 编写前端订阅解析属性测试（vitest + fast-check）
    - 对任意输入不抛异常；受支持节点 round-trip；非支持计入 ignored
    - `{ numRuns: 100 }`
    - **Validates: Requirements 4.1, 4.4, 4.8**

- [ ] 4. 阶段 D — 分流决策 + 模拟器（Req 3）
  - [x] 4.1 实现分流决策纯函数 `compute_route_decision`（`engine.rs`）
    - 改动点：新增 `struct RouteDecision { bypass_hit, matched_rule, nic_if_index, via_upstream }`、`enum MatchedRule { Process(String), Domain(String), None }`、`compute_route_decision(upstream_chain, bypass, rules_proc, rules_nic, bindings, host, port, proc_name, chosen_if_index, sched_idx) -> RouteDecision`，与 `decide_rule_action`/`decide_egress` 优先级严格一致（bypass 最高 > 进程 > 域名 > 调度；走上游 vs 直连与 `decide_egress` 一致）
    - 验证点：纯函数无 IO、不发起真实连接、不改引擎状态
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.6_

  - [ ] 4.2 编写分流决策语义一致属性测试（proptest）
    - **Property 5: 分流决策与 Route_Resolver 语义一致（compute_route_decision）**
    - `// Feature: pro-differentiation-and-hardening, Property 5`，≥100 次
    - **Validates: Requirements 3.2, 3.3, 3.4, 3.6**

  - [x] 4.3 前端模拟器纯逻辑 `lib/routesim.ts`
    - 改动点：新增 `src/lib/routesim.ts`：`validateSimInput(host, port)`（host 非空且 port∈[1,65535]，返回失败标记，Req 3.5）+ `formatRouteDecision(...)` 展示映射；可携带当前配置以纯 TS 复算展示，语义与后端一致
    - 验证点：纯逻辑；`tsc --noEmit` 通过
    - _Requirements: 3.1, 3.5_

  - [ ] 4.4 编写模拟器输入校验属性测试（vitest + fast-check）
    - **Property 9: 模拟器输入校验（validateSimInput）**
    - `{ numRuns: 100 }`
    - **Validates: Requirements 3.5**

- [ ] 5. 阶段 E — 每网卡 DNS/DoH（Req 7）
  - [x] 5.1 实现 DNS 端点校验纯函数 + 解析接入回退（`engine.rs`）
    - 改动点：新增 `validate_dns_endpoint(kind: DnsKind, endpoint: &str) -> bool`（plain=合法 IPv4/IPv6；doh=`https://` 且主机段非空）
    - 改动点：`resolve_host` / `resolve_host_dual` 入口前插入：若 `per_nic_dns.get(nic.if_index)` 存在则经该网卡出口用指定 DNS/DoH 解析（复用既有 DoH/UDP 拨号骨架 + IP_UNICAST_IF），失败/超时记日志并回退既有全局解析；未配置则直接走既有路径
    - 验证点：`per_nic_dns` 为空时既有全局 DNS/DoH/fake-ip/AAAA/A 路径逐字节不变（Req 7.3/7.6）
    - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5, 7.6_

  - [ ] 5.2 编写 DNS 端点校验属性测试（proptest）
    - **Property 6: DNS 端点校验正确性（validate_dns_endpoint）**
    - `// Feature: pro-differentiation-and-hardening, Property 6`，≥100 次
    - **Validates: Requirements 7.5**

  - [x] 5.3 前端每网卡 DNS 校验 `lib/dnsvalidate.ts`
    - 改动点：新增 `src/lib/dnsvalidate.ts`：`validateDnsEndpoint(kind, endpoint)` 与后端语义一致
    - 验证点：纯逻辑；`tsc --noEmit` 通过
    - _Requirements: 7.5_

- [ ] 6. 阶段 F — 系统代理防泄漏看门狗（Req 5）
  - [x] 6.1 新增 `proxyguardian.rs` + `sysproxy.rs` 接线 + 生命周期
    - 改动点：新增模块 `src-tauri/src/proxyguardian.rs`：`struct ProxySnapshot { enable: u32, server: String }`（serde）、`capture_and_persist(dir) -> io::Result<()>`（接管前读注册表原始快照并写守护 json 文件，Req 5.1）、`restore_and_clear(dir)`（据快照还原 + 删文件，Req 5.2）、`recover_on_startup(dir)`（残留快照则补偿还原，损坏则安全跳过，Req 5.3）、`is_dead_gateway(proxy_enabled, port_listening) -> bool`（纯函数）、还原失败按最大次数重试并记日志（Req 5.5）
    - 改动点：`sysproxy.rs` 的 `enable_system_proxy` 调 `capture_and_persist`、`disable_system_proxy` 调 `restore_and_clear`；既有内存快照/`looks_like_ours`/`clear_residual_proxy` 保留为兜底；仅 `127.0.0.1`（Req 5.7）
    - 改动点：`lib.rs` `setup()` 调 `recover_on_startup`；`cleanup()`/`stop_boost` 调 `restore_and_clear`
    - 验证点：正常设置/清除路径行为与既有等价（Req 5.6）；`cargo check` 通过
    - _Requirements: 5.1, 5.2, 5.3, 5.5, 5.6, 5.7_

  - [ ] 6.2 编写死网关判定属性测试（proptest）
    - **Property 7: 死网关判定（is_dead_gateway）**
    - `// Feature: pro-differentiation-and-hardening, Property 7`，≥100 次
    - **Validates: Requirements 5.4**

  - [x] 6.3 运行期死网关自检任务
    - 改动点：`proxy_guardian` 启用且非 TUN 模式时，`start_boost` 成功接管后 spawn 定时任务：周期探测本地代理端口是否仍 listen，`is_dead_gateway` 为真则 `restore_and_clear` 并记日志（Req 5.4）；引擎停止时取消该任务
    - 验证点：TUN 模式或未启用时不影响既有停止/清理路径
    - _Requirements: 5.4, 5.5_

- [ ] 7. 阶段 G — 崩溃日志 + 稳定性加固（Req 8/9）
  - [x] 7.1 结构化崩溃日志 `Crash_Logger`（`logger.rs` / `lib.rs`）
    - 改动点：`logger.rs` 新增 `format_structured(ts, level, subsystem, msg) -> String`（纯函数，输出含时间戳/级别/子系统/消息，复用既有 `redact`）；`Logger` 增写结构化错误/崩溃记录的入口（复用滚动/脱敏）
    - 改动点：`lib.rs` `run()` 用 `std::panic::set_hook` 安装 panic hook：捕获未处理 panic 的位置/消息/回溯摘要，经 `redact` 脱敏写崩溃日志文件；写盘失败降级 `emit("hmx-log")`（Req 9.5/9.6）；hook 内绝不再 panic
    - 验证点：既有 `format_log_line`/`redact`/滚动/`emit("hmx-log")` 行为不变；`cargo check` 通过
    - _Requirements: 9.1, 9.2, 9.3, 9.4, 9.5, 9.6_

  - [ ] 7.2 编写结构化日志与脱敏属性测试（proptest）
    - **Property 8: 结构化日志与脱敏（format_structured + redact）**
    - `// Feature: pro-differentiation-and-hardening, Property 8`，≥100 次
    - **Validates: Requirements 9.1, 9.2, 9.3**

  - [x] 7.3 稳定性加固：panic 隔离 + 连接/任务上限（`engine.rs`）
    - 改动点：`accept_loop` 派发前检查 `active_conns < conn_cap`，超限拒绝该连接（不 spawn，不影响既有活跃连接，Req 8.2）；spawn 后以 RAII 守卫增减 `active_conns`
    - 改动点：连接处理 future 以 `catch_unwind`（`futures`/`std::panic::AssertUnwindSafe` + tokio）兜底：单连接 panic 被捕获、记结构化日志（含连接标识与位置，Req 8.4）、仅释放本连接、引擎与其他连接存活（Req 8.1，依赖 release panic=unwind）
    - 改动点：后台任务（探测/测速等）经 `task_cap` 信号量（`tokio::sync::Semaphore`）限制并发派发（Req 8.3）
    - 验证点：连接数/任务数低于上限且无 panic 时既有连接处理路径行为无影响（Req 8.5）
    - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

- [x] 8. 阶段 H — 高效转发形式化（Req 6）
  - [x] 8.1 `relay` 复用缓冲常量化与语义固化（`engine.rs`）
    - 改动点：将 `relay` 的每方向缓冲尺寸常量化为 `const RELAY_BUF_BYTES: usize = 65536`，明确「每方向一次性分配复用缓冲、读到 0 半关闭、双向结束释放资源」的顺序；严格保持逐字节等价、既有 `RateLimiter` 下行限速语义与遥测统计口径不变
    - 验证点：转发内容逐字节等价（不改写/截断/重排）；限速与遥测口径不变；`cargo check` 通过
    - _Requirements: 6.1, 6.2, 6.3, 6.4, 6.5_

- [ ] 9. 阶段 I — 前端 UI + i18n（Req 12）
  - [x] 9.1 `i18n.ts` 新增全部文案键（中英对齐）
    - 改动点：为健康探测/优选、订阅导入、每网卡 DNS/DoH、稳定性上限、防泄漏看门狗、分流决策模拟器新增全部文案键，`zh`/`en` 双字典严格对齐；既有键不改
    - 验证点：切换英文无中文残留；`tsc --noEmit` 通过
    - _Requirements: 12.4_

  - [ ] 9.2 `SettingsPage.tsx` 新增配置分区
    - 改动点：新增分区——健康探测与优选（总开关 + 间隔/超时/阈值/冷却参数，接入校验）、订阅导入（粘贴 Import_Source + 调 `parseSubscription` 预览 + 一键测速排序 + 确认并入上游列表，遵守 128 上限）、每网卡 DNS/DoH（每张聚合网卡配置 plain/doh + endpoint，接入 `validateDnsEndpoint`）、稳定性上限（connCap/taskCap）、防泄漏看门狗开关；全部走 i18n、`aria-label`，沿用既有 Switch/Segmented/NumberField/Row 风格
    - 验证点：既有设置项分区与交互不受影响；`tsc --noEmit` 通过
    - _Requirements: 4.6, 4.7, 7.5, 12.1, 12.3, 12.5_

  - [ ] 9.3 `SettingsPage.tsx` 新增分流决策模拟器分区
    - 改动点：输入目标（域名/进程名 + 可选端口）+ 触发模拟 + 展示 `RouteDecision`（命中 bypass/规则/承载网卡/直连或上游/选中上游标签），接入 `validateSimInput`；不发起真实连接
    - 验证点：纯展示不改引擎状态；`tsc --noEmit` 通过
    - _Requirements: 3.1, 3.5, 12.2, 12.5_

  - [ ] 9.4 编写 i18n 键对齐属性测试（vitest + fast-check）
    - **Property 10: i18n 中英字典键集合完全一致**
    - 位于 `src/i18n.test.ts`（复用/追加），`{ numRuns: 100 }`
    - **Validates: Requirements 12.4**

- [ ] 10. 阶段 J — 端到端本地 mock 测试基建（Req 10，非可选核心）
  - [x] 10.1 实现 Mock_Upstream 与 Echo_Target 测试基建
    - 改动点：在 `src-tauri` 测试作用域（`#[cfg(test)]` 或 `tests/`）实现：`Mock_Upstream`（tokio 监听 `127.0.0.1:0`，socks5/http 两模式，可配置要求/不要求认证；要求认证时正确凭据成功、错误失败、**无凭据视为有效认证尝试并成功**）；`Echo_Target`（tokio 监听 `127.0.0.1:0`，隧道内字节原样回写）
    - 验证点：仅绑定 `127.0.0.1`，不触达真实公网
    - _Requirements: 10.1, 10.2, 10.4_

  - [ ] 10.2 `connect_via_upstream` 端到端集成测试（握手+转发+认证三态）
    - 改动点：`#[tokio::test]` 用例：经 Mock_Upstream(socks5/http、含/不含认证) 到 Echo_Target 写随机字节，断言逐字节回读相等（Req 10.3）；认证正确=>成功、错误=>失败、无凭据=>成功（Req 10.4）
    - 验证点：独立于真实公网/网卡/GUI；任何真实网络依赖视为失败（Req 10.7/10.8）
    - _Requirements: 10.3, 10.4, 10.7, 10.8_

  - [ ] 10.3 `establish_target` + `next_fallback` 回退端到端集成测试
    - 改动点：`#[tokio::test]` 用例：首个 Mock_Upstream 拒连、次选可用 => 断言回退到次选成功（Req 10.5）；全部上游不可用 + 回退策略 Direct => 断言直连 Echo_Target 成功、策略 Fail => 断言返回错误（Req 10.6）
    - 验证点：独立于真实公网/网卡；`cargo test` 可跑
    - _Requirements: 10.5, 10.6, 10.7_

- [ ] 11. 阶段 K — 补齐既有可选属性测试（Req 11）
  - [ ] 11.1 补齐 `nic-upstream-proxy-chain` 后端可选属性测试
    - 在 `engine.rs` 测试模块补齐该 spec Property 1/2/4/5/6/7/8/9/10：SOCKS5 CONNECT 请求 round-trip、CONNECT 应答 REP 判定、HTTP CONNECT 请求行 round-trip、HTTP 状态行 2xx 判定、Base64 round-trip、`pick_upstream_for_nic`、`sanitize_bindings`、`decide_egress`、`next_fallback`；各 ≥100 次带 Feature 注释
    - **Validates: Requirements 11.1, 11.2, 11.3, 11.4（既有可选补齐）**

  - [x] 11.2 补齐前端可选属性测试
    - 补齐 `validateUpstream`（`src/lib/upstream.test.ts`）与 i18n 键对齐既有可选测试；`{ numRuns: 100 }`
    - **Validates: Requirements 11.5**

- [ ] 12. 阶段 L — 最终检查点，确保全部测试通过
  - Ensure all tests pass, ask the user if questions arise.

## Notes

- 标注 `*` 的子任务为可选（属性测试类），可为更快的 MVP 跳过；阶段 J 端到端 mock 集成测试为**非可选**核心任务（把「必须人工」转自动化的关键价值）。
- 每个任务引用具体需求子条目并对应设计函数签名；新增能力以「默认关闭/旁路」引入，各任务验证点已写明零回归检查（Req 13）。
- 属性测试：Rust `proptest`（≥100 次 + `// Feature: pro-differentiation-and-hardening, Property N`），前端 `vitest` + `fast-check`（≥100 次）。`proptest` 为既有 dev-dependency，无需重复引入。
- 依赖关系：阶段 A 是全部后端/前端的前置；改 `engine.rs` 的任务（1.1/1.2/2.1/2.4/2.6/4.1/5.1/7.3/8.1）彼此串行以避免同文件冲突；新模块 `subscription.rs`（3.1）、`proxyguardian.rs`（6.1）、前端任务（1.3/3.3/4.3/5.3/9.x）可与 engine 任务并行；端到端测试（阶段 J）依赖 2.6/5.1/7.3。

### 人工实机 / 集成冒烟验证清单（不写自动化测试，由人工验证）

- 真实公网 socks5/http 上游的健康探测熔断/恢复实机观察（Req 1）；多上游按质量优选的实际选路（Req 2）。
- 真实多物理网卡并行叠加带宽（Req 边界）；每网卡独立 DNS/DoH 经真实网卡出口解析（Req 7）。
- Proxy_Guardian：真实强杀/崩溃后下次启动补偿还原系统代理（可半自动：预置残留快照文件后启动验证）；运行期真实死网关触发还原（Req 5）。
- 高效转发的真实 CPU 下降对比（Req 6）。
- 订阅导入真实机场订阅/分享链接解析 + 真实测速排序 + 并入（Req 4.6/4.7）。
- 前端 GUI：各新增分区渲染/持久化/交互、决策模拟器展示、`aria-label` 存在性、128 上限提示、中英切换无残留（Req 12）。
- panic 隔离：注入型故障下单连接异常不打垮引擎的实机确认（Req 8）。
- 零回归实机确认：全部新增能力关闭/旁路时既有直连聚合/上游链/bypass/规则/调度/限速/fake-ip/IPv4·IPv6·TCP·DNS·UDP·诊断·测速·TUN·HUD·托盘等与升级前一致（Req 13）。

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["1.2", "1.3", "3.1", "3.3", "6.1"] },
    { "id": 2, "tasks": ["2.1", "4.3", "5.3", "9.1", "11.2"] },
    { "id": 3, "tasks": ["2.4", "4.1", "5.1"] },
    { "id": 4, "tasks": ["2.6", "7.1"] },
    { "id": 5, "tasks": ["7.3", "8.1", "6.3"] },
    { "id": 6, "tasks": ["10.1"] },
    { "id": 7, "tasks": ["10.2", "10.3"] },
    { "id": 8, "tasks": ["9.2", "9.3"] },
    { "id": 9, "tasks": ["11.1"] },
    { "id": 10, "tasks": ["12"] }
  ]
}
```
