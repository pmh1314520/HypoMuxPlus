# Requirements Document

## Introduction

本文档定义 HypoMuxPlus（Windows 多网卡带宽聚合工具，Tauri 2 + Rust(tokio) 后端 + React 19/TS 前端，衍生自原版 HypoMux）的一组**专业化差异化与稳定性加固**能力扩展。目标是在既有「每网卡上游代理链 / 多节点聚合」（见 `nic-upstream-proxy-chain` spec）之上，补齐面向专业用户的上游智能优选、分流可视化、订阅导入、防泄漏、高效转发、每网卡 DNS/DoH，以及运行时稳定性与可测性加固，使 HypoMuxPlus 相对原版 HypoMux 形成清晰的专业度与可靠性差异。

**动机与场景：**
用户在多网卡聚合 + 多上游节点场景下，面临三类痛点：① 上游节点质量参差且会临时抖动/失效，静态映射无法自动避开劣质节点；② 分流规则复杂（进程/域名/bypass/网卡/上游多维叠加），用户难以预判某个目标究竟走哪条路径；③ 上游节点常以 Clash 订阅或分享链接形式分发，手工逐条录入成本高。与此同时，作为常驻网络工具，一旦主进程异常退出而未还原系统代理会导致用户「断网」，单连接异常不应拖垮整个引擎，转发路径的 CPU 开销也直接影响聚合上限。

**范围（本次要做）：**
1. 上游节点智能健康探测与自动优选（后台探测连通性/延迟、故障熔断 + 自动恢复、按质量对一网卡多上游动态加权优选，与既有回退协同）。
2. 分流决策可视化模拟器（输入域名/进程名 + 可选端口，实时显示命中的路径：走哪张网卡 / 直连 / 哪个上游 / 是否命中 bypass / 命中哪条规则）。
3. 订阅式上游导入（解析 Clash / base64 订阅或节点分享链接为 socks5/http 上游列表，忽略其他协议并提示，可一键测速排序）。
4. 系统代理防泄漏看门狗（主进程异常退出/被杀/崩溃时可靠还原系统代理与死网关检测）。
5. 性能与专业度增强（高效中继转发以降低 CPU；每网卡可配独立 DNS / DoH）。
6. 运行时稳定性加固（单连接 panic 隔离兜底、活跃连接数 / 后台任务数上限保护、结构化崩溃 / 错误日志）。
7. 可测性与测试自动化增强（本地 mock SOCKS5/HTTP 上游 + 本地 echo 目标，端到端自动化测试上游握手 + 转发 + 认证 + 失败回退，把原「必须人工」的验收项尽量转为 `cargo test` 可跑，并补齐既有可选属性测试）。

**范围外（本次不做）：**
- 不实现完整的 Clash 规则引擎（`rules` / `rule-providers` / `proxy-groups` 的策略路由与选路语义）；订阅仅解析出 `socks5` / `http` 节点入口用于 Upstream_Proxy 列表。
- 不实现除 `socks5` / `http` 之外上游协议（shadowsocks / vmess / trojan / hysteria 等）的实际转发；此类节点在导入时被识别为「不支持」并计数提示，不写入可用上游。
- 不改变既有上游选择「完全由承载网卡决定」的架构（不新增「按目标选上游」维度）；本次的优选发生在「同一网卡绑定的多个上游之间」。
- 不实现真实公网节点的自动化联网测试；端到端自动化仅针对本地 mock 上游与本地 echo 目标。

**兼容性基线（本次不推翻，且必须零回归）：**
- 引擎 `engine.rs`（Splitting_Engine）既有能力：多网卡直连聚合、SOCKS5/HTTP 入站、双栈 IPv6、SOCKS5 UDP ASSOCIATE、按进程/域名分流、三种调度策略（RR / 最少连接 / 加权）、限速、bypass 直连白名单、fake-ip、DoH、诊断（延迟/抖动/丢包/趋势）、聚合测速；以及每网卡上游代理链（`connect_via_upstream`、`establish_target`、`decide_egress`、`next_fallback`、`pick_nic`、`connect_via_nic`、`decide_rule_action`、`UpstreamProxy` / `UpstreamBinding`）。
- TUN 全局接管、HUD、托盘、开机自启、热键、情景模式、应用兼容性、配置备份、应用内更新等行为保持不变。
- 前端完整中英双语（`i18n.ts` 双字典，键须严格对齐、英文文案无中文残留）；无障碍已成体系（`aria-label`、`role=dialog`、`aria-live`）。
- 仅 Windows 10/11；代理仅监听 `127.0.0.1`。
- 测试基建：Rust `proptest`（dev-dependency，属性测试带 `// Feature: pro-differentiation-and-hardening, Property N` 注释、≥100 次迭代）；前端 `vitest` + `fast-check`（每属性 ≥100 次）；纯逻辑析出为纯函数；端到端自动化用本地 mock 不依赖真实公网 / 网卡。

**全局硬约束（贯穿全部需求）：**
- 仅支持 Windows 10 / 11。
- 所有代理监听端点仅绑定 `127.0.0.1`。
- 全部新增能力对既有全部能力零回归：新增能力默认关闭或默认旁路，未启用时既有代码路径字节级行为不变。
- 前端新增文案在中英双字典严格对齐，英文界面无中文残留；新增交互元素延续既有无障碍规范。
- 真实多物理网卡叠加、真实公网节点握手、GUI 渲染等归人工实机验证，不声称自动化 100% 覆盖。

## Glossary

- **HypoMuxPlus**：本产品整体。以下需求主体按子系统命名。
- **Splitting_Engine**：`engine.rs` 中的 SOCKS5/HTTP 分流引擎（含 DNS、限速、调度、遥测、Egress_Binding、上游代理链）。
- **Route_Resolver**：分流规则解析与网卡选择逻辑（`pick_nic`、`decide_rule_action`、`decide_egress`、`pattern_match`）。
- **Settings_UI**：`SettingsPage.tsx` 设置页面。
- **IfIndex**：Windows 接口索引，网卡绑定的权威标识。
- **Egress_Binding**：把出站 socket 物理钉死在指定网卡（`IP_UNICAST_IF` / `IPV6_UNICAST_IF` + 绑定该网卡源地址）的动作。
- **Upstream_Proxy**：一条上游代理节点条目（`socks5` / `http`，含 host、port、可选认证、label、稳定唯一的 Upstream_Id），沿用 `nic-upstream-proxy-chain` 定义。
- **Upstream_Id**：上游代理条目稳定唯一、不复用的标识。
- **Upstream_Binding**：一条「参与聚合的物理网卡 ↔ 一个或多个 Upstream_Proxy」的映射关系。
- **Upstream_Route**：一条「走上游」的连接路径：经所选网卡物理出口连接该网卡绑定的 Upstream_Proxy，再经该上游 CONNECT 到真实目标。
- **Direct_Aggregate**：既有直连聚合路径（不经上游，经网卡物理出口直连真实目标）。
- **Health_Prober**：新增的上游健康探测子系统，在后台周期性对上游节点探测连通性与延迟并维护其健康状态。
- **Upstream_Health**：单个 Upstream_Proxy 的健康状态与质量度量，至少包含：可用性状态（Healthy / Circuit_Open）、最近一次探测的延迟样本、连续失败计数、进入熔断的时间戳。
- **Circuit_Open**：一个上游因连续探测/连接失败达到阈值而被熔断（暂时排除出优选候选）的状态。
- **Circuit_Recovery**：熔断上游经冷却期后被重新纳入探测、探测成功后恢复为 Healthy 的过程。
- **Upstream_Selector**：新增的上游质量加权优选逻辑，在一张网卡绑定的多个上游之间依据 Upstream_Health 与既有调度序动态选出承载上游。
- **Route_Simulator**：新增的分流决策可视化模拟器，对用户给定的目标（域名/进程名 + 可选端口）以纯函数计算并展示其将命中的完整分流路径。
- **Route_Decision**：Route_Simulator 的输出，描述一条模拟连接的判定路径：命中的规则、承载网卡、是否命中 bypass、走直连或走上游、以及走上游时选中的上游。
- **Subscription_Importer**：新增的订阅导入解析子系统，把 Clash 订阅 / base64 订阅 / 节点分享链接解析为 Upstream_Proxy 候选列表。
- **Import_Source**：一次导入的输入文本，可能是 Clash YAML、base64 编码的订阅正文，或以换行分隔的节点分享链接集合。
- **Proxy_Guardian**：新增的系统代理防泄漏看门狗子系统，负责在主进程异常终止时可靠还原系统代理并检测死网关。
- **System_Proxy**：Windows 系统级代理配置（WinINET / 注册表 `ProxyEnable` / `ProxyServer` 等）与其原始快照。
- **Dead_Gateway**：系统代理指向一个已不再监听的本地代理端口，导致应用无法联网的「死网关」状态。
- **Relay_Engine**：`engine.rs` 中承担已建立连接双向数据转发的中继逻辑（既有 `relay`），本次做高效转发增强。
- **Per_NIC_DNS**：每张网卡可配置的独立 DNS 解析配置（明文 DNS 服务器或 DoH 端点），用于经该网卡出口解析目标域名。
- **DoH**：DNS over HTTPS，既有 DNS 解析能力之一。
- **Stability_Guard**：新增的运行时稳定性加固逻辑，含单连接 panic 隔离与活跃连接数 / 后台任务数上限保护。
- **Connection_Cap**：允许同时存在的活跃中继连接数上限。
- **Task_Cap**：允许同时存在的后台任务（探测、测速等）数上限。
- **Crash_Logger**：新增的结构化崩溃 / 错误日志子系统，以结构化字段记录崩溃与错误事件。
- **Mock_Upstream**：测试基建中在 `127.0.0.1` 上运行的本地 SOCKS5 / HTTP 上游代理模拟器，用于端到端自动化测试上游握手、认证与转发。
- **Echo_Target**：测试基建中在 `127.0.0.1` 上运行的本地回显目标服务，用于验证经上游隧道的双向数据转发。
- **Test_Suite**：Rust `proptest` 属性测试与前端 `vitest` + `fast-check` 测试集合，以及本次新增的端到端本地集成测试。

## Requirements

### Requirement 1: 上游节点健康探测与故障熔断/自动恢复

**User Story:** 作为专业用户，我希望软件在后台自动探测每个上游节点的连通性与延迟，并在节点失效时熔断、恢复后自动纳回，以便个别节点抖动不影响整体聚合质量。

#### Acceptance Criteria

1. WHILE Upstream_Chain_Mode 已启用且 Health_Prober 已启用，THE Health_Prober SHALL 按可配置的探测间隔（缺省 30 秒）周期性对每个被引用的 Upstream_Proxy 经其所属网卡的 Egress_Binding 发起一次连通性探测并记录本次延迟样本与成功/失败结果。
2. WHEN 一次上游探测在可配置的探测超时（缺省不超过 5 秒）内完成 Upstream_Handshake，THE Health_Prober SHALL 将该 Upstream_Proxy 的 Upstream_Health 记录为 Healthy 并更新其最近延迟样本与连续失败计数为 0。
3. IF 一个 Upstream_Proxy 的连续探测失败次数达到可配置的熔断阈值（缺省 3 次），THEN THE Health_Prober SHALL 将该 Upstream_Proxy 置为 Circuit_Open 状态并记录进入熔断的时间戳。
4. WHILE 一个 Upstream_Proxy 处于 Circuit_Open 状态且未超过可配置的冷却期（缺省 60 秒），THE Upstream_Selector SHALL 将该 Upstream_Proxy 排除出承载上游的优选候选。
5. WHEN 一个处于 Circuit_Open 状态的 Upstream_Proxy 超过冷却期，THE Health_Prober SHALL 对该 Upstream_Proxy 发起一次探测，并当该探测成功时（Circuit_Recovery）将其恢复为 Healthy 状态、重新纳入优选候选。
6. WHERE Health_Prober 未启用，THE Splitting_Engine SHALL 将全部被引用的 Upstream_Proxy 视为 Healthy 并按既有回退与调度逻辑处理连接。
7. THE HypoMuxPlus SHALL 将 Health_Prober 的默认启用状态设为未启用。
8. WHEN 一个 Upstream_Proxy 的 Upstream_Health 状态发生变化（Healthy ↔ Circuit_Open），THE Health_Prober SHALL 记录一条包含上游标签、新状态与失败原因的可读日志。

**边界（不做什么）：**
- 不实现基于长期历史成功率的机器学习式质量建模，仅按连续失败计数、冷却期与最近延迟样本进行状态机式判定。
- 不对未被任何 Upstream_Binding 引用的 Upstream_Proxy 发起后台探测。
- 探测经网卡物理出口发起，真实网卡与真实节点的探测行为由人工实机验证。

### Requirement 2: 上游质量加权动态优选

**User Story:** 作为专业用户，我希望一张网卡绑定多个上游时软件按节点质量动态选择更优的上游，以便在保留既有回退能力的同时优先使用低延迟可用节点。

#### Acceptance Criteria

1. WHERE 一张网卡绑定多个 Upstream_Proxy 且 Health_Prober 已启用，WHEN Splitting_Engine 为该网卡承载的新连接选择承载上游，THE Upstream_Selector SHALL 在该网卡绑定的 Healthy 上游中依据最近延迟样本进行加权优选，延迟更低的上游被选中的权重更高。
2. WHERE 一张网卡绑定的全部上游均处于 Circuit_Open 状态，THE Splitting_Engine SHALL 按既有回退策略（`next_fallback`）在该网卡上游集合内轮试或按 Upstream_Fallback 回退直连 / 失败。
3. WHEN Upstream_Selector 选出的上游在实际建连或 Upstream_Handshake 中失败，THE Splitting_Engine SHALL 按既有 `next_fallback` 在该网卡剩余未尝试上游中继续尝试，且不改变既有回退的终止语义。
4. WHERE Health_Prober 未启用，THE Splitting_Engine SHALL 使用既有 `pick_upstream_for_nic` 的调度序选择承载上游，行为与 `nic-upstream-proxy-chain` 现状一致。
5. THE Upstream_Selector SHALL 保持既有一对一映射、共享映射与调度策略（RR / 最少连接 / 加权）的语义不变。
6. FOR ALL 输入的上游健康度量与调度序，Upstream_Selector 的选择结果 SHALL 恒为该网卡当前候选上游集合中的一员（不得选出未绑定或已被熔断排除的上游）。

**边界（不做什么）：**
- 优选仅发生在同一网卡绑定的上游集合内，不跨网卡借用其他网卡的上游。
- 不新增独立于承载网卡的「按目标选上游」维度。

### Requirement 3: 分流决策可视化模拟器

**User Story:** 作为用户，我希望在设置页输入一个目标域名或进程名（可带端口）就能看到它将走哪条分流路径，以便在不实际发起连接的情况下验证我的规则配置。

#### Acceptance Criteria

1. WHEN 用户在 Route_Simulator 输入一个目标（域名或进程名，可选端口）并触发模拟，THE Route_Simulator SHALL 以纯函数依据当前 bypass、按网卡规则、按进程规则、调度策略、上游映射计算出一条 Route_Decision 并展示。
2. WHEN 一个模拟目标命中 bypass 直连白名单，THE Route_Simulator SHALL 在 Route_Decision 中标明「命中 bypass、走直连」且不展示任何承载上游。
3. WHEN 一个模拟目标未命中 bypass，THE Route_Simulator SHALL 在 Route_Decision 中标明命中的规则（进程规则 / 域名规则 / 无规则回退调度策略）、承载网卡的 IfIndex，以及该连接走 Direct_Aggregate 还是 Upstream_Route。
4. WHERE 一个模拟目标被判定为走 Upstream_Route，THE Route_Simulator SHALL 在 Route_Decision 中展示承载网卡当前将选中的 Upstream_Proxy 标签。
5. IF 用户输入的目标为空或端口不在 1 至 65535 范围内，THEN THE Route_Simulator SHALL 拒绝模拟并显示可读的输入校验错误提示。
6. THE Route_Simulator SHALL 保持与 Route_Resolver 相同的优先级语义（bypass 最高，进程规则优先于域名规则，再按调度策略），且模拟结果不发起任何真实网络连接、不改变引擎运行状态。

**边界（不做什么）：**
- 模拟结果为「基于当前配置的静态判定」，不反映运行期上游熔断的实时抖动（除非模拟器可读取当前健康快照，作为可选增强）。
- 不模拟 TUN 模式下 UDP / QUIC 的会话级行为，仅模拟 TCP 连接的分流路径判定。

### Requirement 4: 订阅式上游导入

**User Story:** 作为用户，我希望粘贴 Clash 订阅、base64 订阅或节点分享链接就能批量导入 socks5/http 上游，并一键测速排序，以便快速建立可用的上游节点列表。

#### Acceptance Criteria

1. WHEN 用户提交一段 Import_Source，THE Subscription_Importer SHALL 以纯函数将其解析为一组 Upstream_Proxy 候选，仅保留类型为 `socks5` 与 `http` 的节点。
2. WHERE Import_Source 为 base64 编码正文，THE Subscription_Importer SHALL 先对其做 Base64 解码再解析其中的节点分享链接。
3. WHERE Import_Source 为 Clash 订阅（YAML），THE Subscription_Importer SHALL 从其 `proxies` 列表中提取 `type` 为 `socks5` 或 `http` 的节点并映射为 Upstream_Proxy 候选（含 host、port、可选认证、label）。
4. WHEN Import_Source 中存在类型不属于 `socks5` / `http` 的节点，THE Subscription_Importer SHALL 忽略该节点、不将其写入候选，并在导入结果中统计被忽略的不支持节点数量以供提示。
5. IF Import_Source 无法解析出任何受支持节点，THEN THE Subscription_Importer SHALL 返回空候选列表并提示未发现受支持的上游节点。
6. WHEN 用户对导入的候选触发一键测速，THE HypoMuxPlus SHALL 对每个候选测量其连通延迟并按延迟从低到高对候选列表排序，且对测速失败的候选排在末尾并标记不可用。
7. WHEN 用户确认导入，THE HypoMuxPlus SHALL 将选中的候选并入 Upstream_Proxy 列表并遵守既有 128 条上限与字段校验（沿用 `nic-upstream-proxy-chain` Req 1）。
8. THE Subscription_Importer SHALL 在解析过程中对任意格式非法或畸形的 Import_Source 均不 panic，畸形节点被跳过并计入被忽略数量。

**边界（不做什么）：**
- 不实现 Clash 的规则引擎、`proxy-groups`、`rule-providers` 与策略选路语义，仅提取节点入口。
- 不实现 shadowsocks / vmess / trojan / hysteria 等协议的实际转发，仅识别并计数为不支持。
- 一键测速为连通性延迟测量，不做真实公网吞吐评估；真实公网节点测速由人工验证。

### Requirement 5: 系统代理防泄漏看门狗

**User Story:** 作为用户，我希望即使 HypoMuxPlus 被强杀或崩溃，系统代理也能被可靠还原，以便我不会在程序异常退出后陷入无法联网的状态。

#### Acceptance Criteria

1. WHEN HypoMuxPlus 设置 System_Proxy 指向本地代理端口，THE Proxy_Guardian SHALL 在修改前持久化系统代理的原始快照（`ProxyEnable`、`ProxyServer`、覆盖例外等）。
2. WHEN HypoMuxPlus 正常停止聚合或退出，THE Proxy_Guardian SHALL 依据持久化的原始快照还原 System_Proxy。
3. WHEN HypoMuxPlus 下一次启动时检测到存在上一次未被还原的 System_Proxy 快照，THE Proxy_Guardian SHALL 依据该快照还原 System_Proxy 后再继续启动流程。
4. WHILE 聚合运行中 System_Proxy 指向的本地代理端口不再监听，THE Proxy_Guardian SHALL 判定为 Dead_Gateway 并还原 System_Proxy 以避免用户断网。
5. IF System_Proxy 的还原操作失败，THEN THE Proxy_Guardian SHALL 记录一条包含失败原因的可读日志并重试还原至可配置的最大次数。
6. THE Proxy_Guardian SHALL 保持既有系统代理设置 / 清除功能在正常路径上的行为不变。
7. THE HypoMuxPlus SHALL 保证 System_Proxy 相关操作仅指向 `127.0.0.1` 的本地代理端点。

**边界（不做什么）：**
- 不监控或还原由第三方软件设置的、非本程序写入的系统代理配置。
- 死网关检测针对本程序所设代理端点的存活性，不做全局网络连通性巡检。
- 主进程被操作系统强制终止后的还原依赖启动期快照补偿（AC 3），不声称在任意强杀瞬间的原子还原。

### Requirement 6: 高效中继转发（降低 CPU）

**User Story:** 作为用户，我希望大流量下载时软件的 CPU 占用尽量低，以便转发开销不成为聚合带宽的瓶颈。

#### Acceptance Criteria

1. THE Relay_Engine SHALL 对每条已建立的中继连接以可复用缓冲区进行双向数据转发，避免每次转发都分配新缓冲区。
2. WHEN 一条中继连接的任一方向读到 0 字节（对端关闭），THE Relay_Engine SHALL 半关闭对应方向并在两个方向均结束后释放该连接的转发资源。
3. THE Relay_Engine SHALL 在启用高效转发后保持与既有转发路径逐字节等价的数据完整性（转发内容不被改写、截断或重排）。
4. WHERE 既有下行限速（令牌桶）对某连接生效，THE Relay_Engine SHALL 在高效转发路径下保持既有限速语义不变。
5. THE HypoMuxPlus SHALL 保持既有遥测（吞吐、活跃连接、每网卡分布）在高效转发路径下的统计口径不变。

**边界（不做什么）：**
- 不承诺具体的 CPU 下降百分比数值；性能改善由人工实机对比验证。
- 不对 UDP 中继与 TUN 栈的转发路径做本次改造，仅覆盖 TCP 中继。
- 是否采用平台特有的零拷贝系统调用由设计与实现阶段确定，需求层仅约束「可复用缓冲、语义等价、限速不变」。

### Requirement 7: 每网卡独立 DNS / DoH

**User Story:** 作为多网卡用户，我希望为每张网卡单独指定 DNS 或 DoH 解析服务，以便不同上行线路使用各自合适的解析路径。

#### Acceptance Criteria

1. THE HypoMuxPlus SHALL 支持为每张参与聚合的网卡持久化配置一项 Per_NIC_DNS，取值为明文 DNS 服务器地址或 DoH 端点 URL。
2. WHERE 一张网卡配置了 Per_NIC_DNS，WHEN Splitting_Engine 需经该网卡解析一个目标域名，THE Splitting_Engine SHALL 使用该网卡的 Per_NIC_DNS 经该网卡 Egress_Binding 进行解析。
3. WHERE 一张网卡未配置 Per_NIC_DNS，THE Splitting_Engine SHALL 使用既有全局 DNS / DoH 解析路径解析目标域名。
4. IF 一张网卡配置的 Per_NIC_DNS 解析在设定超时内失败，THEN THE Splitting_Engine SHALL 记录一条可读日志并回退到既有全局 DNS / DoH 解析路径。
5. THE Settings_UI SHALL 提供每网卡 Per_NIC_DNS 的配置交互，并对非法的 DNS 地址或 DoH URL 拒绝保存并提示。
6. THE Splitting_Engine SHALL 保持既有全局 DNS、DoH、fake-ip 与 AAAA / A 解析路径的行为不变。

**边界（不做什么）：**
- Per_NIC_DNS 仅作用于经该网卡出口的域名解析，不改变系统级 DNS 配置。
- 不实现 DNS 解析结果的跨网卡缓存共享策略，缓存语义沿用既有实现。

### Requirement 8: 运行时稳定性加固（panic 隔离与上限保护）

**User Story:** 作为用户，我希望单条连接出现异常时不会拖垮整个引擎，且高并发下软件不会因资源耗尽而崩溃，以便长时间稳定运行。

#### Acceptance Criteria

1. IF 处理某一条连接的任务发生 panic，THEN THE Stability_Guard SHALL 捕获该 panic 使其仅影响该连接、释放该连接资源，且不终止 Splitting_Engine 或其他连接。
2. WHEN 活跃中继连接数达到 Connection_Cap（可配置上限），THE Splitting_Engine SHALL 拒绝或排队新入站连接以避免资源耗尽，且不影响既有活跃连接。
3. WHEN 后台任务（探测、测速等）数达到 Task_Cap（可配置上限），THE HypoMuxPlus SHALL 限制新后台任务的并发派发直至已有任务完成。
4. WHEN 一条连接因 panic 被隔离，THE Stability_Guard SHALL 记录一条包含连接标识与失败位置的结构化日志。
5. THE Stability_Guard SHALL 保证 panic 隔离与上限保护在未触发（连接数 / 任务数低于上限且无 panic）时对既有连接处理路径行为无影响。
6. THE HypoMuxPlus SHALL 为 Connection_Cap 与 Task_Cap 设置合理的默认上限值。

**边界（不做什么）：**
- 不对 Rust 中因 `panic = "abort"` 编译配置导致的进程级中止提供进程内恢复；panic 隔离以 catch-unwind 语义为前提。
- 上限保护针对本程序内部的连接与任务计数，不做操作系统级资源配额管理。

### Requirement 9: 结构化崩溃与错误日志

**User Story:** 作为用户与维护者，我希望崩溃与错误以结构化字段被记录，以便退出后仍能定位问题并用于反馈排查。

#### Acceptance Criteria

1. WHEN HypoMuxPlus 主进程发生未捕获的 panic，THE Crash_Logger SHALL 在进程终止前将一条包含时间戳、panic 位置、错误消息与调用栈摘要的结构化崩溃记录写入本地崩溃日志文件。
2. WHEN Splitting_Engine 或其子系统产生一条错误事件，THE Crash_Logger SHALL 以包含时间戳、级别、子系统名与错误消息的结构化字段记录该事件。
3. WHEN 写入崩溃 / 错误日志记录，THE Crash_Logger SHALL 对敏感信息（如本机完整 IP 地址、可标识用户的路径、认证凭据）做脱敏处理。
4. WHILE 崩溃 / 错误日志文件达到设定大小上限，THE Crash_Logger SHALL 滚动到新文件并保留不超过设定数量的历史文件。
5. IF 崩溃 / 错误日志写入失败，THEN THE Crash_Logger SHALL 不阻断主流程并降级为既有前端日志面板输出。
6. THE Crash_Logger SHALL 保持既有 `emit("hmx-log")` 前端日志面板行为不变。

**边界（不做什么）：**
- 不上传任何日志到远程服务器。
- 不记录完整数据报或连接负载内容，仅记录事件与错误元信息。
- 调用栈摘要的完整性依赖运行时可获取的回溯信息，不声称在所有编译配置下均含完整符号。

### Requirement 10: 端到端自动化测试基建（本地 mock 上游 + echo 目标）

**User Story:** 作为维护者，我希望用本地 mock 上游与本地 echo 目标端到端地自动化测试上游握手、认证、转发与失败回退，以便把原本必须人工的验收项尽量转为 `cargo test` 可跑。

#### Acceptance Criteria

1. THE Test_Suite SHALL 提供一个运行于 `127.0.0.1` 的 Mock_Upstream，可分别以 `socks5` 与 `http` 模式接受 CONNECT 隧道请求，且可配置为要求或不要求认证。
2. THE Test_Suite SHALL 提供一个运行于 `127.0.0.1` 的 Echo_Target，可将经隧道收到的字节原样回写。
3. WHEN 自动化测试经 `connect_via_upstream` 向 Mock_Upstream 建立到 Echo_Target 的隧道并写入随机字节，THE Test_Suite SHALL 断言读回的字节与写入的字节逐字节相等（握手 + 转发正确）。
4. WHEN Mock_Upstream 配置为要求认证且测试提供正确凭据，THE Test_Suite SHALL 断言 Upstream_Handshake 成功；当测试提供错误凭据时，THE Test_Suite SHALL 断言 Upstream_Handshake 失败；WHERE 客户端未提供任何凭据，THE Mock_Upstream SHALL 将其视为一次有效的认证尝试并允许 Upstream_Handshake 成功。
5. WHEN 一个网卡绑定的首个上游对应一个不可达或拒绝连接的 Mock_Upstream 且存在可用的次选 Mock_Upstream，THE Test_Suite SHALL 经 `establish_target` 与 `next_fallback` 断言连接回退到次选上游后成功（失败回退正确）。
6. WHERE 一个网卡的全部 Mock_Upstream 均不可用且回退策略为「回退直连」，THE Test_Suite SHALL 断言 `establish_target` 直连本地 Echo_Target 成功；WHERE 回退策略为「失败」，THE Test_Suite SHALL 断言 `establish_target` 返回错误。
7. WHEN 在开发机执行 `cargo test`，THE Test_Suite SHALL 使上述端到端测试独立于真实公网、真实网卡绑定与 GUI 给出通过/失败结果。
8. IF 某端到端测试对真实网络资源产生任何依赖，THEN THE Test_Suite SHALL 使该测试判定为失败，无论其结果是否恰好正确。

**边界（不做什么）：**
- Mock_Upstream 与 Echo_Target 仅监听 `127.0.0.1`，不验证真实物理网卡 Egress_Binding 的系统调用效果（由人工实机验证）。
- 不用端到端测试验证真实公网节点握手与真实多网卡并行叠加带宽。

### Requirement 11: 纯函数可测性与属性测试补齐

**User Story:** 作为维护者，我希望本次新增逻辑的纯函数部分具备属性测试，并补齐既有上游代理链中标记为可选的属性测试，以便重构与扩展时快速发现回归。

#### Acceptance Criteria

1. THE Test_Suite SHALL 使用 Rust `proptest` 为 Upstream_Selector 的加权优选纯函数编写属性测试，验证其选择结果恒属于当前候选上游集合且熔断上游不被选中。
2. THE Test_Suite SHALL 使用 Rust `proptest` 为 Health_Prober 的健康状态机纯函数（连续失败计数达阈值进入 Circuit_Open、冷却期后允许恢复）编写属性测试。
3. THE Test_Suite SHALL 为 Subscription_Importer 的解析纯函数编写属性测试，验证对任意字节输入不 panic，且对由受支持节点构造再序列化的输入解析可还原等价的 Upstream_Proxy 候选（round-trip）。
4. THE Test_Suite SHALL 为 Route_Simulator 的 Route_Decision 纯函数编写属性测试，验证其判定与 Route_Resolver 的优先级语义一致（bypass 最高、进程规则优先于域名规则、再按调度策略）。
5. THE Test_Suite SHALL 使用前端 `vitest` + `fast-check` 为订阅导入、每网卡 DNS 配置校验、分流模拟输入校验等前端纯逻辑，以及本次新增文案的 i18n 中英键对齐编写测试。
6. THE Test_Suite SHALL 使每条属性测试运行不少于 100 次迭代并带 `// Feature: pro-differentiation-and-hardening, Property N` 注释。
7. WHEN 在开发机执行 `cargo test` 与前端 `vitest` 单次运行（`--run`），THE Test_Suite SHALL 独立于 GUI 与真实公网环境给出通过/失败结果，且当且仅当 `cargo test` 与 `vitest` 两者均独立成功执行时视为满足本需求。
8. THE Test_Suite SHALL 保持对 GUI 与真实网络的独立性，不论其属性测试的迭代次数多少（迭代次数为零亦不得引入外部依赖）。

**边界（不做什么）：**
- 不为需要真实 socket 连接公网、真实网卡绑定或 Windows 系统调用的函数编写自动化属性测试（此类由 Requirement 10 的本地 mock 端到端测试或人工实机验证覆盖）。
- 不编写端到端 UI 渲染快照测试。

### Requirement 12: 前端配置界面、双语与无障碍

**User Story:** 作为用户，我希望本次新增的全部功能都有可用的设置界面且中英文一致、可无障碍访问，以便在中英两种语言下都能顺畅配置。

#### Acceptance Criteria

1. THE Settings_UI SHALL 为 Health_Prober 开关与探测参数、Upstream_Selector 优选、订阅导入、每网卡 DNS / DoH、稳定性上限与防泄漏看门狗相关的可配置项提供交互界面。
2. THE Settings_UI SHALL 为分流决策模拟器提供输入目标与展示 Route_Decision 的交互界面。
3. WHEN 用户修改本次任一新增配置项，THE Settings_UI SHALL 持久化该变更并在下一次聚合启动时生效。
4. THE Settings_UI SHALL 为本次全部新增文案在中文与英文字典中提供键集合完全一致的键值，且英文界面无中文残留。
5. THE Settings_UI SHALL 为本次全部新增交互元素提供无障碍标签（`aria-label`），并对模态与动态区域延续既有 `role=dialog` 与 `aria-live` 规范。

**边界（不做什么）：**
- 不重构既有设置页的信息架构，新增能力以叠加分区形式并入。
- GUI 实际渲染与交互效果由人工实机验证。

### Requirement 13: 零回归、默认关闭/旁路与平台约束

**User Story:** 作为现有用户，我希望升级到本版本后所有既有能力行为完全不变，以便新增能力不会破坏我现有的使用。

#### Acceptance Criteria

1. THE HypoMuxPlus SHALL 将本次新增的每项能力（Health_Prober、Upstream_Selector 加权优选、Per_NIC_DNS、Stability_Guard 上限保护、Proxy_Guardian、高效转发的可选开关）默认设为关闭或默认旁路。
2. WHILE 本次新增能力均未启用，THE Splitting_Engine SHALL 使既有直连聚合、上游代理链、bypass、按进程 / 域名规则、调度策略、限速、fake-ip、DoH、双栈 IPv6、UDP ASSOCIATE、诊断与聚合测速路径行为与升级前一致。
3. WHILE 本次任一新增能力已启用，THE HypoMuxPlus SHALL 依据该能力的既定语义改变相应连接的处理行为，此时不要求与该能力未启用时的既有行为逐字节一致（既有行为仅在全部新增能力均未启用时才保证不变）。
4. THE HypoMuxPlus SHALL 仅在 Windows 10 / 11 上运行本次全部新增能力。
5. THE HypoMuxPlus SHALL 保证本次新增的全部代理与本地服务监听端点仅绑定 `127.0.0.1`。
6. THE HypoMuxPlus SHALL 保持 TUN 全局接管、HUD、托盘、开机自启、热键、情景模式、应用兼容性、配置备份与应用内更新的既有行为不变。

**边界（不做什么）：**
- 不在非 Windows 平台提供本次能力。
- 真实多物理网卡叠加、真实公网节点握手与 GUI 渲染归人工实机验证，不声称自动化 100% 覆盖。
