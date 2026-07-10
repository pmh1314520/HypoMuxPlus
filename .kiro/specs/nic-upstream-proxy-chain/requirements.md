# Requirements Document

## Introduction

本文档定义 HypoMuxPlus（Windows 多网卡带宽聚合工具，Tauri 2 + Rust(tokio) 后端 + React/TS 前端）的一项新能力扩展：**每网卡上游代理链 / 多节点聚合**。

**动机与场景：**
用户希望在使用 Clash 等翻墙代理下载国外资源时，仍能借助多张物理网卡把本地上行带宽叠加起来。现有 HypoMuxPlus 通过把每条出站 TCP 连接用 `IP_UNICAST_IF` / `IPV6_UNICAST_IF` + 绑定源地址钉死到指定物理网卡，将多条连接分散到多张相互独立的物理上行网卡以叠加带宽（Egress_Binding）。

**朴素方案为何失效（必须体现在需求边界与假设中）：**
若把 HypoMuxPlus 的上游简单设为「Clash（`127.0.0.1`）」，则 HypoMuxPlus 到 Clash 的连接是本机回环，不经过任何物理网卡，网卡钉绑失效，所有流量汇聚到 Clash 的单一出口，无法叠加。真正能叠加的架构是：HypoMuxPlus 作为「多上游聚合代理」，为每张参与聚合的物理网卡各自绑定一个上游代理节点（如某机场/节点暴露的 SOCKS5 或 HTTP 入口，或远端节点入口）；当一条下载连接被调度到某张网卡时，HypoMuxPlus 以该网卡的物理出口去连接「该网卡绑定的上游代理节点」，再由上游把流量送往翻墙目标。由此「网卡A→上游节点1」「网卡B→上游节点2」并行，既叠加本地上行、又用多个节点分摊，突破本地最后一公里与单节点两个瓶颈。

**叠加真正生效的物理前提（作为使用假设，不由本功能保证）：**
① 存在 ≥2 条相互独立的物理上行线路；② 下载为多线程 / 多连接；③ 瓶颈在本地上行或单节点带宽，而非源站；④ 各上游节点相互独立、不共享同一上游瓶颈。

**兼容性基线（不在本次推翻）：**
- 引擎 `engine.rs`（Splitting_Engine）已有 `connect_via_nic`（按 `SocketAddr` 分派 v4/v6 + `IP_UNICAST_IF`/`IPV6_UNICAST_IF`）、`pick_nic`、`resolve_host_dual`、`decide_rule_action`、`RouteRuleDef{pattern, action, kind}`，以及 SOCKS5/HTTP 入站处理 `handle_socks`/`handle_http`。
- 既有直连聚合、bypass 直连白名单、按网卡规则、按进程规则、调度策略（RR / 最少连接 / 加权）、限速、fake-ip、IPv4/TCP/DNS 路径均须保持行为不变。
- 前端完整中英双语（`i18n.ts` 双字典，键须严格对齐）；无障碍已成体系（`aria-label`、`role=dialog`、`aria-live`）。
- 仅 Windows 10/11；代理仅监听 `127.0.0.1`。
- 测试基建：Rust `proptest`（dev-dependency，属性测试带 `// Feature: <name>, Property N` 注释、≥100 次迭代）；前端 `vitest` + `fast-check`（每属性 ≥100 次）。

**范围（本次要做）：** 上游代理条目定义与管理、网卡↔上游映射、作为 SOCKS5/HTTP 客户端对上游发起 CONNECT（含可选认证且到上游的 socket 仍被钉死在指定网卡）、与既有直连聚合并存与零回归、分流协同、健康与回退、前端配置 UI、纯函数可测性。

**范围外（本次不做）：** 不实现 Clash 的订阅 / 规则引擎（节点信息由用户手工填写或简单导入）；首版仅覆盖 TCP CONNECT，不实现 UDP over 上游的复杂封装（标记为后续可选）；不实现上游节点的自动测速与自动选路（后续可选）。

## Glossary

- **HypoMuxPlus**：本产品整体。以下需求主体按子系统命名。
- **Splitting_Engine**：`engine.rs` 中的 SOCKS5/HTTP 分流引擎（含 DNS、限速、调度、遥测、Egress_Binding）。
- **Route_Resolver**：分流规则解析与网卡选择逻辑（`pick_nic`、`decide_rule_action`、`pattern_match`、规则解析）。
- **Settings_UI**：`SettingsPage.tsx` 设置页面。
- **IfIndex**：Windows 接口索引，网卡绑定的权威标识。
- **Egress_Binding**：把出站 socket 物理钉死在指定网卡（`IP_UNICAST_IF`/`IPV6_UNICAST_IF` + 绑定该网卡源地址）的动作。
- **Upstream_Proxy**：一条上游代理节点条目。至少包含：唯一标识（Upstream_Id）、类型（`socks5` / `http`）、主机地址（Upstream_Host，域名或 IP）、端口、可选用户名与密码认证、备注名（Upstream_Label）。
- **Upstream_Id**：上游代理条目的稳定唯一标识，用于网卡映射引用。
- **Upstream_Client**：Splitting_Engine 内新增的「对上游发起连接」的客户端逻辑，实现 SOCKS5 CONNECT 与 HTTP CONNECT 两种上游握手。
- **Upstream_Binding**：一条「参与聚合的物理网卡 ↔ 一个或多个 Upstream_Proxy」的映射关系。
- **Upstream_Mapping**：全部 Upstream_Binding 的集合，是网卡到上游节点的映射表。
- **Upstream_Chain_Mode**：上游代理链能力的总开关状态（启用 / 未启用）。
- **Upstream_Route**：一条被判定为「走上游」的连接所经过的完整路径：经所选网卡的物理出口连接该网卡绑定的 Upstream_Proxy，再由上游 CONNECT 到真实目标。
- **Direct_Aggregate**：既有的直连聚合路径（不经上游，经网卡物理出口直连真实目标）。
- **Upstream_Handshake**：Upstream_Client 与 Upstream_Proxy 之间建立到真实目标隧道的协议交互（SOCKS5 版本协商 + CONNECT 请求/应答；或 HTTP `CONNECT` 请求行 + 状态行解析）。
- **Upstream_Auth**：上游代理的认证信息（SOCKS5 用户名/密码认证子协商；HTTP `Proxy-Authorization: Basic` 头）。
- **Test_Suite**：Rust `proptest` 属性测试与前端 `vitest` + `fast-check` 测试集合。

## Requirements

### Requirement 1: 上游代理条目定义与管理

**User Story:** 作为用户，我希望定义并管理一组上游代理节点，以便把不同网卡的流量分别经不同上游节点转发。

#### Acceptance Criteria

1. THE HypoMuxPlus SHALL 支持持久化保存一组至多 128 条 Upstream_Proxy 条目，每条包含 Upstream_Id、类型、Upstream_Host（长度 ≤ 253 字符）、端口、可选 Upstream_Auth 与 Upstream_Label（长度 ≤ 64 字符）。
2. THE HypoMuxPlus SHALL 支持 Upstream_Proxy 的类型取值 `socks5` 与 `http`。
3. WHEN 用户新增、编辑或删除一条 Upstream_Proxy，THE Settings_UI SHALL 持久化该变更。
4. WHEN 一条 Upstream_Proxy 的增删改完成，THE Settings_UI SHALL 将界面条目列表刷新为持久化后的最新结果。
5. WHERE 一条 Upstream_Proxy 配置了 Upstream_Auth，THE HypoMuxPlus SHALL 随该条目一并持久化其用户名（长度 1 至 255 字符）与密码（长度 1 至 255 字符）。
6. IF 用户提交的 Upstream_Proxy 缺少 Upstream_Host、Upstream_Host 长度超过 253 字符、端口不在 1 至 65535 范围内、或类型不属于 `socks5`/`http`，THEN THE Settings_UI SHALL 拒绝保存该条目、保留用户已输入内容并显示指明失败字段的可读校验错误提示。
7. IF 当前条目数已达 128 且用户尝试新增，THEN THE Settings_UI SHALL 拒绝新增并提示已达条目数上限。
8. THE HypoMuxPlus SHALL 为每条 Upstream_Proxy 分配一个在同组内互不相同、且在其生命周期内保持稳定且不被复用的 Upstream_Id，供 Upstream_Binding 引用。

**边界（不做什么）：**
- 不解析或导入 Clash 的订阅链接与规则引擎；节点信息由用户手工填写或从简单文本来源导入。
- 不支持除 `socks5` 与 `http` 之外的上游协议（如 shadowsocks、vmess、trojan）。

### Requirement 2: 网卡与上游代理的绑定映射

**User Story:** 作为用户，我希望把参与聚合的每张物理网卡与上游代理节点关联，以便一条连接被调度到某网卡时经其绑定的上游节点转发。

#### Acceptance Criteria

1. THE HypoMuxPlus SHALL 支持在参与聚合的物理网卡与 Upstream_Proxy 之间建立 Upstream_Binding，并将全部绑定持久化为 Upstream_Mapping。
2. THE HypoMuxPlus SHALL 支持一张网卡绑定一个 Upstream_Proxy 的一对一映射。
3. WHERE 一张网卡绑定多个 Upstream_Proxy，THE Splitting_Engine SHALL 在该网卡承载新连接时按既有调度策略在这些上游之间选择一个 Upstream_Proxy。
4. WHERE 多张网卡绑定同一个 Upstream_Proxy，THE Splitting_Engine SHALL 允许该共享映射并各自经自身网卡物理出口连接同一上游地址。
5. WHEN 用户在 Settings_UI 修改 Upstream_Mapping，THE HypoMuxPlus SHALL 持久化该映射并在下一次连接调度时生效。
6. IF 一条被引用的 Upstream_Proxy 已被删除，THEN THE HypoMuxPlus SHALL 移除引用该条目的 Upstream_Binding 并将对应网卡视为未绑定上游。

**边界（不做什么）：**
- 不实现基于目标地址前缀的「按目标选上游」策略，映射对象仅为网卡。
- 不实现上游节点的自动测速与自动优选。

### Requirement 3: 上游握手（SOCKS5 / HTTP CONNECT）

**User Story:** 作为用户，我希望 HypoMuxPlus 能作为 SOCKS5 与 HTTP 客户端向上游代理发起 CONNECT 隧道，以便把真实目标流量交给上游节点转发。

#### Acceptance Criteria

1. WHERE 目标 Upstream_Proxy 类型为 `socks5`，WHEN Upstream_Client 建立到真实目标的隧道，THE Upstream_Client SHALL 向该上游发送 SOCKS5 版本协商与 `CMD=0x01`（CONNECT）请求，并当且仅当上游应答的 `REP` 字段为 `0x00` 时判定隧道建立成功。
2. WHERE 目标 Upstream_Proxy 类型为 `http`，WHEN Upstream_Client 建立到真实目标的隧道，THE Upstream_Client SHALL 向该上游发送 HTTP `CONNECT <host>:<port>` 请求，并当且仅当响应状态行状态码为 2xx（含 200）时判定隧道建立成功。
3. WHERE 目标 Upstream_Proxy 配置了 Upstream_Auth 且类型为 `socks5`，THE Upstream_Client SHALL 在版本协商中声明用户名/密码认证方法并按 SOCKS5 用户名/密码子协商发送凭据。
4. WHERE 目标 Upstream_Proxy 配置了 Upstream_Auth 且类型为 `http`，THE Upstream_Client SHALL 在 `CONNECT` 请求中包含 `Proxy-Authorization: Basic` 头，其值为用户名与密码的 Base64 编码。
5. WHEN 真实目标以域名形式给出，THE Upstream_Client SHALL 将该域名作为目标地址交由上游解析（SOCKS5 使用 `ATYP=0x03` 域名地址类型；HTTP 使用域名 `host:port`）。
6. IF Upstream_Handshake 返回失败状态码或应答格式非法，THEN THE Upstream_Client SHALL 判定该上游隧道建立失败并触发 Requirement 6 定义的回退。

**边界（不做什么）：**
- 首版仅实现 CONNECT（TCP 隧道），不实现 SOCKS5 UDP ASSOCIATE 或 BIND over 上游。
- 不对上游隧道内的应用层载荷做解析或改写（保持端到端加密透传）。

### Requirement 4: 经网卡物理出口连接上游（Egress_Binding 复用）

**User Story:** 作为用户，我希望 HypoMuxPlus 到上游节点的连接本身也钉死在指定网卡的物理出口，以便多网卡到多上游的连接真正并行叠加而非汇聚到单一出口。

#### Acceptance Criteria

1. WHEN Upstream_Client 建立一条到 Upstream_Proxy 的连接，THE Splitting_Engine SHALL 对该连接的 socket 应用所选网卡的 Egress_Binding（`IP_UNICAST_IF`/`IPV6_UNICAST_IF` + 绑定该网卡源地址）。
2. WHEN 上游地址以域名给出，THE Splitting_Engine SHALL 经所选网卡出口解析该上游域名并按解析结果建立到上游的物理出口连接。
3. WHEN 一条连接被调度到某网卡且该网卡存在 Upstream_Binding，THE Splitting_Engine SHALL 经该网卡的物理出口连接其绑定的 Upstream_Proxy，再经 Upstream_Handshake 连接真实目标（构成 Upstream_Route）。
4. IF 所选网卡不具备连接上游地址所需地址族的可用源地址，THEN THE Splitting_Engine SHALL 记录一条可读日志并触发 Requirement 6 定义的回退。
5. THE Splitting_Engine SHALL 保证到上游的连接经物理网卡出口而非本机回环。

**边界（不做什么）：**
- 不改变既有 Egress_Binding 的底层实现，仅复用 `connect_via_nic` 的地址族分派与钉绑逻辑。
- 不处理上游地址为 `127.0.0.1`/回环时的叠加语义保证（此类配置无法叠加，属于用户配置责任，可在 UI 给出提示）。

### Requirement 5: 与既有直连聚合并存与零回归

**User Story:** 作为用户，我希望在未启用上游代理链时软件行为与现状完全一致，以便升级不会影响我原有的直连聚合使用。

#### Acceptance Criteria

1. WHILE Upstream_Chain_Mode 未启用，THE Splitting_Engine SHALL 使用既有 Direct_Aggregate 路径处理全部连接且行为与现状一致。
2. WHILE Upstream_Chain_Mode 已启用，WHEN 一条连接被判定为「走上游」，THE Splitting_Engine SHALL 改用 Upstream_Route 处理该连接。
3. WHILE Upstream_Chain_Mode 已启用，WHEN 一条连接被判定为「直连」，THE Splitting_Engine SHALL 使用既有 Direct_Aggregate 路径处理该连接。
4. THE Splitting_Engine SHALL 保持既有 IPv4/IPv6、TCP、DNS、限速、调度、fake-ip 与直连聚合路径的行为不变。
5. THE HypoMuxPlus SHALL 将 Upstream_Chain_Mode 的默认状态设为未启用。

**边界（不做什么）：**
- 不移除或替换任何既有直连聚合能力，上游链为叠加式新增能力。

### Requirement 6: 上游健康与回退策略

**User Story:** 作为用户，我希望某个上游不可用时软件能按既定策略回退，以便个别节点故障不会中断整体下载。

#### Acceptance Criteria

1. IF 到某 Upstream_Proxy 的物理建连与 Upstream_Handshake 在可配置的上游超时（缺省不超过 10 秒）内未完成成功，THEN THE Splitting_Engine SHALL 判定该上游本次不可用并按配置的回退策略处理该连接。
2. WHERE 所选网卡绑定多个 Upstream_Proxy 且其一不可用，THE Splitting_Engine SHALL 尝试该网卡绑定的其他 Upstream_Proxy。
3. WHERE 回退策略配置为「回退直连」且该网卡绑定的全部上游均不可用，THE Splitting_Engine SHALL 经该网卡的物理出口以 Direct_Aggregate 方式直连真实目标。
4. WHERE 回退策略配置为「失败」且该网卡绑定的全部上游均不可用，THE Splitting_Engine SHALL 以标准 SOCKS5/HTTP 错误应答向入站客户端返回连接失败。
5. WHEN 发生上游不可用或回退，THE Splitting_Engine SHALL 记录一条包含上游标签与失败原因的可读日志。

**边界（不做什么）：**
- 首版不实现基于历史成功率的主动健康探测与熔断，仅按单次连接的超时/失败即时判定。
- 不实现跨网卡借用其他网卡上游的回退（回退仅在同一网卡绑定的上游集合内或回退直连/失败）。

### Requirement 7: 分流协同与优先级

**User Story:** 作为用户，我希望上游代理链与既有 bypass、按网卡规则、按进程规则、调度策略协同工作，以便国内直连、国外走上游链并按明确优先级组合。

#### Acceptance Criteria

1. WHEN 一条连接命中 bypass 直连白名单，THE Route_Resolver SHALL 以 Direct_Aggregate 路径处理该连接且不经任何 Upstream_Proxy。
2. WHEN Route_Resolver 判定一条连接走聚合出口，THE Route_Resolver SHALL 先按既有优先级（进程规则优先于域名规则，再按调度策略）选定承载网卡，再依据该网卡是否存在 Upstream_Binding 决定走 Upstream_Route 还是 Direct_Aggregate。
3. WHERE 一条连接被调度到的网卡不存在 Upstream_Binding，THE Splitting_Engine SHALL 以 Direct_Aggregate 路径处理该连接。
4. WHERE 一条连接被调度到的网卡存在 Upstream_Binding 且未命中 bypass，THE Splitting_Engine SHALL 以 Upstream_Route 处理该连接。
5. THE Route_Resolver SHALL 保持既有 bypass、按网卡规则、按进程规则与调度策略（RR / 最少连接 / 加权）的既有语义不变。

**边界（不做什么）：**
- 不新增独立于既有规则体系的「按目标选上游」规则维度；上游选择完全由承载网卡的 Upstream_Binding 决定。

### Requirement 8: 上游代理链配置界面

**User Story:** 作为用户，我希望在设置页管理上游代理条目与网卡映射并切换总开关，以便直观配置多网卡多上游聚合。

#### Acceptance Criteria

1. THE Settings_UI SHALL 提供 Upstream_Proxy 条目的新增、编辑与删除交互。
2. THE Settings_UI SHALL 提供参与聚合网卡与 Upstream_Proxy 之间 Upstream_Binding 的编辑交互。
3. THE Settings_UI SHALL 提供 Upstream_Chain_Mode 的启用/关闭开关。
4. WHEN 用户切换 Upstream_Chain_Mode 或修改上游配置，THE Settings_UI SHALL 持久化该变更。
5. THE Settings_UI SHALL 为上游代理链相关的全部新增文案在中文与英文字典中提供对齐的键值。
6. THE Settings_UI SHALL 为上游代理链相关的交互元素提供无障碍标签（`aria-label`）。

**边界（不做什么）：**
- 不在 UI 内提供上游节点测速或延迟展示（后续可选）。

### Requirement 9: 纯函数可测性

**User Story:** 作为维护者，我希望上游握手报文构造/解析与网卡上游选择逻辑具备属性测试，以便重构与扩展时快速发现回归。

#### Acceptance Criteria

1. THE Test_Suite SHALL 使用 Rust `proptest` 为 SOCKS5 CONNECT 请求构造与应答解析、HTTP `CONNECT` 请求行构造与状态行解析等纯函数编写属性测试。
2. WHERE 存在报文构造与解析配对逻辑（SOCKS5 CONNECT 请求地址段的构造与解析、HTTP CONNECT 请求行的构造与解析），THE Test_Suite SHALL 包含往返（round-trip）测试，验证构造后再解析可还原等价结果。
3. THE Test_Suite SHALL 为网卡到 Upstream_Proxy 的选择逻辑（含一网卡多上游的调度选择与共享映射解析）编写属性测试。
4. THE Test_Suite SHALL 为回退决策的纯函数逻辑（给定上游可用性与回退策略产出目标动作）编写属性测试。
5. THE Test_Suite SHALL 使每条属性测试运行不少于 100 次迭代并带 `// Feature: nic-upstream-proxy-chain, Property N` 注释。
6. THE Test_Suite SHALL 使用前端 `vitest` + `fast-check` 为上游配置相关的前端纯逻辑与 i18n 键对齐编写测试。
7. WHEN 在开发机执行 `cargo test` 与前端 `vitest` 单次运行（`--run`），THE Test_Suite SHALL 独立于 GUI 与网络环境给出通过/失败结果。

**边界（不做什么）：**
- 不为需要真实 socket 连接上游、真实网卡绑定或 Windows 系统调用的函数编写自动化测试（由人工实机验证）。
- 不编写端到端 UI 渲染快照测试。
