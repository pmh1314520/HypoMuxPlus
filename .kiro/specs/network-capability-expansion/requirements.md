# Requirements Document

## Introduction

本文档定义 HypoMuxPlus（Windows 多网卡带宽聚合工具，Tauri + Rust(tokio) 后端 + React/TS 前端）的一次能力扩展与工程补强，覆盖用户明确选择的 6 项工作：

1. IPv6 支持（分流、按网卡出口绑定、AAAA 解析、网卡地址扫描与展示、双栈选择与回退）
2. UDP / QUIC 分流（TUN 模式下按网卡出口绑定的 UDP 中继，可选 SOCKS5 UDP ASSOCIATE）
3. 按进程分流（按可执行文件名把流量钉到指定网卡 / 直连 / 聚合）
4. 自动化测试（Rust 纯函数单元测试 + 前端纯逻辑/工具测试）
5. 本地日志文件（关键事件与错误滚动落地、脱敏、路径可打开）
6. 诊断增强（抖动、丢包率探测，诊断历史曲线，纳入报告 / PNG 导出）

**背景与既有实现（作为兼容性基线，不在本次推翻）：**
- 引擎 `engine.rs` 现为纯 IPv4：SOCKS5 仅处理 `ATYP=0x01(IPv4)`/`0x03(域名)`，收到 `0x04(IPv6)` 拒绝；出口绑定用 `IP_UNICAST_IF` + 绑定 IPv4 源地址；DNS 仅 A 记录。
- TUN 模式 `tunmode.rs`：ipstack 用户态栈仅处理 V4 流，V6 直接 `return` 丢弃；非 53 端口 UDP（QUIC/HTTP3）丢弃；DNS 采用 fake-ip（198.18.0.0/15）。
- 网卡扫描 `netadapter.rs`：`GetAdaptersAddresses(AF_INET)`，仅取首个 IPv4 单播地址。
- 分流规则现按域名/端口，`action` 为 `direct` / `aggregate` / `nic:<ifindex>`。
- 诊断 `DiagnosticsPage.tsx` + 后端 `test_latency`/`speed_test`：仅测 TCP 握手 RTT 与下载吞吐；历史仅存"上次评级"于 localStorage。
- 错误仅 `console.error` 与 `emit("hmx-log")` 到前端日志面板，退出后无留存。
- 前端完整中英双语（`i18n.ts` 双字典，键须严格对齐）；无障碍已成体系（aria-label、`role=dialog`、`aria-live`）。
- 仅 Windows 10/11；代理仅监听 `127.0.0.1`；TUN 依赖 `wintun.dll` 与管理员/服务权限。

## Glossary

- **HypoMuxPlus**：本产品整体。以下需求主体按子系统命名。
- **Splitting_Engine**：`engine.rs` 中的 SOCKS5/HTTP 分流引擎（含 DNS、限速、调度、遥测）。
- **TUN_Stack**：`tunmode.rs` 中基于 wintun + ipstack 的用户态 TCP/IP 栈（含 fake-ip DNS）。
- **Adapter_Scanner**：`netadapter.rs` 中的网卡发现模块。
- **Route_Resolver**：分流规则解析与网卡选择逻辑（`pick_nic`、`pattern_match`、规则解析）。
- **Process_Resolver**：将连接关联到发起进程（PID → 可执行文件名）的模块。
- **Logger**：新增的本地日志落地子系统（滚动文件、脱敏、路径管理）。
- **Diagnostics_Engine**：诊断探测后端（RTT、吞吐、抖动、丢包）。
- **Diagnostics_UI**：`DiagnosticsPage.tsx` 诊断页面。
- **Settings_UI**：`SettingsPage.tsx` 设置页面。
- **Test_Suite**：Rust 单元测试（内置 `#[test]`）与前端 vitest 测试集合。
- **IfIndex**：Windows 接口索引，网卡绑定的权威标识。
- **IP_UNICAST_IF / IPV6_UNICAST_IF**：Windows 套接字选项，用于强制指定出口网卡（分别对应 IPv4 / IPv6）。
- **Fake_IP**：TUN 模式下对域名分配的占位 IP（现为 198.18.0.0/15），供后续连接反查域名。
- **Dual_Stack_Policy**：目标同时具备 IPv4 与 IPv6 地址时的选择与回退策略。
- **Egress_Binding**：把出站 socket 物理钉死在指定网卡（`*_UNICAST_IF` + 绑定该网卡源地址）的动作。

## Requirements

### Requirement 1: IPv6 目标分流与按网卡出口绑定

**User Story:** 作为多网卡用户，我希望访问 IPv6 目标的流量也能被分流并钉到指定网卡出口，以便在纯 IPv6 或双栈环境下同样获得多网卡聚合与正确的出口选择。

#### Acceptance Criteria

1. WHEN 一个 SOCKS5 请求携带 `ATYP=0x04`（IPv6 目标地址），THE Splitting_Engine SHALL 解析该 IPv6 目标并按当前调度策略选择网卡建立出站连接。
2. WHEN 为一条 IPv6 出站连接选定网卡，THE Splitting_Engine SHALL 对出站 socket 设置 `IPV6_UNICAST_IF`（接口索引为该网卡 IfIndex）并绑定该网卡的 IPv6 源地址。
3. IF 选定网卡不具备可用的全局 IPv6 源地址，THEN THE Splitting_Engine SHALL 记录一条可读的日志并按 Dual_Stack_Policy 回退到 IPv4 出口（当目标同时具备 IPv4 地址时）。
4. WHEN 目标为域名且需经指定网卡解析，THE Splitting_Engine SHALL 支持查询 AAAA 记录并返回 IPv6 结果地址。
5. WHERE 用户在设置中选择 IP 版本偏好为「IPv4 优先」「IPv6 优先」或「仅 IPv4」，THE Splitting_Engine SHALL 依据该偏好在双栈目标上选择首选地址族。
6. WHEN 首选地址族的连接在设定超时内未建立成功，THE Splitting_Engine SHALL 回退尝试另一地址族（当目标具备该地址族地址时）。
7. THE Splitting_Engine SHALL 保持既有 IPv4 分流路径（`ATYP=0x01`/`0x03` 与 `IP_UNICAST_IF` 绑定）行为不变。

**边界（不做什么）：**
- 不实现 IPv6 转 IPv4 的协议翻译（如 NAT64/DNS64）。
- 不实现 IPv6 前缀级路由策略，仅按网卡出口绑定与地址族选择。

### Requirement 2: 网卡 IPv6 地址扫描与展示

**User Story:** 作为用户，我希望在网卡列表看到每张网卡的 IPv6 地址，以便判断哪些网卡具备 IPv6 出口能力。

#### Acceptance Criteria

1. WHEN 执行网卡扫描，THE Adapter_Scanner SHALL 枚举每张 Up 网卡的 IPv4 与 IPv6 单播地址。
2. WHERE 一张网卡存在多个 IPv6 单播地址，THE Adapter_Scanner SHALL 优先返回一个全局单播 IPv6 地址而非链路本地地址（`fe80::/10`）。
3. WHEN 网卡不存在任何 IPv6 单播地址，THE Adapter_Scanner SHALL 将该网卡的 IPv6 字段返回为空字符串。
4. THE Adapter_Scanner SHALL 在返回结构中同时保留既有 `ipv4` 字段且其取值行为不变。
5. WHEN 前端展示网卡列表，THE HypoMuxPlus SHALL 在网卡条目上显示该网卡的 IPv6 地址（存在时）且文案中英同步。

**边界（不做什么）：**
- 不展示网卡的全部 IPv6 地址列表，仅展示一个代表性全局地址。
- 不做 IPv6 地址有效性的主动探测（仅读取系统枚举结果）。

### Requirement 3: TUN 模式 UDP / QUIC 分流

**User Story:** 作为用户，我希望 TUN 模式下的 UDP 流量（尤其是 QUIC/HTTP3）也能经指定网卡出口中继，以便使用 HTTP3 的站点与应用也能获得多网卡出口而非被丢弃回落。

#### Acceptance Criteria

1. WHEN TUN_Stack 截获一条目标端口非 53 的 UDP 流，THE TUN_Stack SHALL 为该流建立经所选网卡 Egress_Binding 的 UDP 中继，而非丢弃该流。
2. WHEN 一条被中继的 UDP 流的目标为 Fake_IP，THE TUN_Stack SHALL 将其反查为真实域名并经所选网卡解析真实目标地址后再中继。
3. WHILE 一条 UDP 中继会话在设定空闲超时内无数据往返，THE TUN_Stack SHALL 释放该会话占用的 socket 与映射资源。
4. WHEN 为一条 UDP 流选择出口网卡，THE Route_Resolver SHALL 复用现有 TCP 的网卡选择逻辑（bypass、按网卡规则、调度策略）。
5. THE TUN_Stack SHALL 保持既有 53 端口 DNS 的 fake-ip 应答行为不变。
6. THE TUN_Stack SHALL 保持既有 TCP 流处理路径行为不变。

**边界（不做什么）：**
- 不对 UDP 载荷做深度包解析或协议改写（QUIC 保持端到端加密，仅做地址级中继）。
- 不实现 UDP 层的限速（下行令牌桶限速仅覆盖 TCP 中继）。

### Requirement 4: SOCKS5 UDP ASSOCIATE（可选能力）

**User Story:** 作为代理模式用户，我希望 SOCKS5 支持 UDP ASSOCIATE，以便通过代理模式（非 TUN）使用的应用也能转发 UDP 流量。

#### Acceptance Criteria

1. WHERE SOCKS5 UDP ASSOCIATE 能力已启用，WHEN 客户端发送 `CMD=0x03`（UDP ASSOCIATE）请求，THE Splitting_Engine SHALL 在 `127.0.0.1` 上分配一个 UDP 中继端口并在应答中返回该绑定地址与端口。
2. WHERE SOCKS5 UDP ASSOCIATE 能力已启用，WHEN 客户端向该中继端口发送带 SOCKS5 UDP 请求头的数据报，THE Splitting_Engine SHALL 按请求头中的目标地址经所选网卡 Egress_Binding 转发该数据报。
3. IF SOCKS5 UDP ASSOCIATE 能力未启用，THEN THE Splitting_Engine SHALL 以标准 SOCKS5 应答码拒绝 `CMD=0x03` 请求。
4. THE Splitting_Engine SHALL 保持既有 `CMD=0x01`（CONNECT）处理路径行为不变。

**边界（不做什么）：**
- 不支持 SOCKS5 BIND（`CMD=0x02`）。
- 该能力为可选实现项；若不实现，则 Requirement 4 的验收退化为 AC 3（拒绝 UDP ASSOCIATE）。

### Requirement 5: 按进程名分流规则

**User Story:** 作为用户，我希望把某个可执行程序的流量整体钉到指定网卡（或直连/聚合），以便按应用而非按域名做出口选择。

#### Acceptance Criteria

1. THE Route_Resolver SHALL 支持一类新的分流规则，其匹配对象为发起连接的进程可执行文件名（大小写不敏感，如 `steam.exe`）。
2. THE Route_Resolver SHALL 支持进程规则的动作取值 `direct`（直连）、`aggregate`（聚合）与 `nic:<ifindex>`（钉死到指定网卡）。
3. WHEN 一条新连接到达且存在命中的进程规则，THE Route_Resolver SHALL 依据该规则的动作选择出口，且进程规则优先级高于域名规则。
4. WHEN 运行于代理模式，THE Process_Resolver SHALL 通过连接的本地端点（本地地址与端口）经 `GetExtendedTcpTable`（owning PID）反查发起进程 PID 并解析为可执行文件名。
5. WHEN 运行于 TUN 模式，THE Process_Resolver SHALL 通过被截获连接的原始本地端点经系统连接表反查发起进程 PID 并解析为可执行文件名。
6. IF 无法在设定时间内确定某连接的发起进程，THEN THE Route_Resolver SHALL 跳过进程规则匹配并回退到域名规则与调度策略。
7. WHEN 用户在设置中新增、编辑或删除进程规则，THE Settings_UI SHALL 持久化该规则并使其中英文案同步。

**边界（不做什么）：**
- 不支持按进程完整路径或命令行参数匹配，仅按可执行文件名。
- 不支持通配符进程名匹配（进程名为精确匹配）。

### Requirement 6: Rust 后端纯函数单元测试

**User Story:** 作为维护者，我希望后端核心纯函数具备单元测试，以便重构与扩展时快速发现回归。

#### Acceptance Criteria

1. THE Test_Suite SHALL 使用 Rust 内置测试框架（`#[cfg(test)]` + `#[test]`）为纯函数编写单元测试。
2. THE Test_Suite SHALL 覆盖 Splitting_Engine 的以下纯函数：`pattern_match`、`build_dns_query`、`parse_dns_a`、`dns_skip_name`、`split_host_port`、`build_origin_header`、`find_header`、`Strategy::parse`。
3. THE Test_Suite SHALL 覆盖 `RateLimiter` 令牌桶的取用与补充逻辑、SWRR 加权轮询与最少连接调度的选择结果。
4. THE Test_Suite SHALL 覆盖 `version_gt` 版本比较逻辑。
5. THE Test_Suite SHALL 覆盖 TUN_Stack 的 `parse_dns_question`、`build_dns_response` 与 Fake_IP 分配（`allocate`/`lookup`）逻辑。
6. WHERE 存在解析与序列化配对逻辑（DNS 查询构造与应答解析、fake-ip 分配与反查），THE Test_Suite SHALL 包含往返（round-trip）测试，验证构造后再解析可还原等价结果。
7. WHEN 在开发机执行 `cargo test`，THE Test_Suite SHALL 可独立于 GUI 与网络环境运行并给出通过/失败结果。

**边界（不做什么）：**
- 不为需要真实网卡绑定、真实 socket 连接或 Windows 系统调用的函数编写自动化测试（这些由人工实机验证）。
- 不引入外部测试运行依赖（CI 集成为可选，不在本次强制范围）。

### Requirement 7: 前端纯逻辑与工具函数测试

**User Story:** 作为维护者，我希望前端纯逻辑与工具函数具备测试，以便保证多语言键对齐与工具行为稳定。

#### Acceptance Criteria

1. THE Test_Suite SHALL 使用 vitest 为前端纯逻辑与工具函数编写测试。
2. THE Test_Suite SHALL 包含 i18n 键对齐测试，验证中文字典与英文字典的键集合完全一致。
3. THE Test_Suite SHALL 覆盖 `clipboard` 复制的回退逻辑、`useModal` 的行为与 `AreaChart` 的 `niceCeil` 计算。
4. THE Test_Suite SHALL 覆盖 `version` 工具的比较逻辑。
5. WHEN 在开发机执行 vitest 单次运行（`--run`），THE Test_Suite SHALL 独立于运行中的开发服务器给出通过/失败结果。

**边界（不做什么）：**
- 不为依赖 Tauri `invoke` 的运行时集成路径编写自动化测试（由人工实机验证）。
- 不编写端到端 UI 渲染快照测试。

### Requirement 8: 本地日志文件落地

**User Story:** 作为用户，我希望关键事件与错误被记录到本地日志文件，以便退出程序后仍能查阅并用于反馈排查。

#### Acceptance Criteria

1. WHEN HypoMuxPlus 产生关键事件或错误，THE Logger SHALL 将该记录写入本地日志文件，每条记录包含时间戳与级别。
2. WHILE 单个日志文件达到设定大小上限，THE Logger SHALL 滚动到新文件并保留不超过设定数量的历史日志文件。
3. WHEN 写入日志记录，THE Logger SHALL 对敏感信息（如本机完整 IP 地址、可标识用户的路径）做脱敏处理。
4. WHEN 用户在设置或关于页点击「打开日志文件夹」，THE HypoMuxPlus SHALL 在系统文件管理器中打开日志所在目录，且该入口文案中英同步并带无障碍标签。
5. IF 日志文件写入失败，THEN THE Logger SHALL 不阻断主流程并降级为仅前端日志面板输出。
6. THE Logger SHALL 继续保持既有 `emit("hmx-log")` 前端日志面板行为不变。

**边界（不做什么）：**
- 不上传任何日志到远程服务器。
- 不记录完整数据报或连接负载内容，仅记录事件与错误元信息。

### Requirement 9: 诊断抖动与丢包探测

**User Story:** 作为用户，我希望诊断能测量每张网卡的抖动与丢包率，以便更全面地评估链路质量。

#### Acceptance Criteria

1. WHEN 用户运行诊断，THE Diagnostics_Engine SHALL 对每张参与网卡经该网卡出口进行多次 RTT 采样并计算抖动（多次 RTT 采样的离散度）。
2. WHEN 用户运行诊断，THE Diagnostics_Engine SHALL 对每张参与网卡经该网卡出口进行多次探测并计算丢包率（未成功探测次数 / 总探测次数）。
3. WHEN 某网卡全部探测均失败，THE Diagnostics_Engine SHALL 将该网卡丢包率报告为 100% 且抖动报告为不可用。
4. WHEN 诊断完成，THE Diagnostics_UI SHALL 在网卡卡片上展示 RTT、抖动、丢包率与吞吐，且新增指标文案中英同步。
5. THE Diagnostics_Engine SHALL 保持既有 RTT 与吞吐探测的取值行为不变。

**边界（不做什么）：**
- 不使用需要管理员权限的原始 ICMP 套接字实现丢包探测（采用 TCP 握手成功率或等价用户态方法）。
- 不做长时间持续后台链路监测，仅在用户触发诊断时探测。

### Requirement 10: 诊断历史曲线与报告导出

**User Story:** 作为用户，我希望保留多次诊断结果并以曲线展示趋势，同时纳入体检报告与 PNG 导出，以便观察链路随时间的变化。

#### Acceptance Criteria

1. WHEN 一次诊断完成，THE Diagnostics_UI SHALL 将本次每张网卡的 RTT、抖动、丢包率与吞吐连同时间戳追加到诊断历史记录并持久化。
2. WHILE 诊断历史记录数量超过设定上限，THE Diagnostics_UI SHALL 仅保留最近的记录并丢弃最旧的记录。
3. WHEN 用户查看诊断页，THE Diagnostics_UI SHALL 以趋势曲线展示所选指标的历史变化。
4. WHEN 用户复制体检报告，THE Diagnostics_UI SHALL 在纯文本报告中包含 RTT、抖动、丢包率与吞吐，且文案中英同步。
5. WHEN 用户导出 PNG，THE Diagnostics_UI SHALL 在导出图中包含 RTT、抖动、丢包率与吞吐。
6. THE Diagnostics_UI SHALL 保持既有「上次评级」历史与「应用健康网卡」行为不变。

**边界（不做什么）：**
- 不将诊断历史导出为独立文件格式（CSV/JSON）；历史仅用于页内展示与既有报告/PNG。
- 不跨设备同步诊断历史。
