# P2：在已完成的 benchmark 平台上建立可组合时序模型

> 状态：计划，尚未实施 System、多跳互连、独立 L1 或 FiveStageCore。
>
> 起点：当前阻塞 CPU 已完成 AM MicroBench 完整十项 test、自检、ROI、性能输出和 Spike 流式差分；单核 yield-os/thread-os 与可选时钟定时器也已接入。
>
> 目标：迁移并保住这些能力，让同一 ELF 可以在两种 CPU × 两种内存路径上运行，验证模型之间的时间、事务和生命周期契约。
>
> 顺序：M0 冻结契约与基线 → M1 迁移现有模型 → M2 多跳互连 → M3 独立 L1 → M4 五级核 → M5 实验矩阵与公开接口收尾。每阶段独立验收、提交。

## 1. 当前起点与交付边界

长期目标仍是 [PLAN.md](PLAN.md) 所述的 Rust 体系结构仿真框架。Python 负责有类型的配置与装配，Rust 负责事件、事务和模型行为。

### 1.1 已完成，P2 复用并回归验证

| 能力 | 当前实现与验证边界 |
| --- | --- |
| 阻塞 CPU | Python `Rv64Core`、Rust `src/cpu.rs::Cpu`；全核最多一个未完成请求，无 cache |
| 事件与内存 | 确定性 `(tick, phase, sequence)` 队列；Traffic、FixedRam、DualBankRam、Router 和 Exit；当前 Machine 集中管理事务，board 限制单 router |
| AM 构建 | 上游 Makefile、RV64I/LP64、具名物理平台、链接脚本和堆栈校验；正式源码在 `third_party`，`target` 存构建产物 |
| MicroBench | 完整十项 test，顺序为 qsort、queen、bf、fib、sieve、15pz、dinic、lzip、ssort、md5；保留上游算法、输入和自检 |
| 输入规模 | `--input test/train/ref/huge` 已开放；test 完整验收，较大输入已有构建与限额启动覆盖，不声称完成全部较大输入 |
| 运行时服务 | SimpleUart 输出子集；共享 m5 ABI、模拟时间读取、成功/失败退出、work_begin/end；服务在提交边界执行 |
| 输出与统计 | `AM_RESULT`、ROI tick/commits、端到端 JSON；恢复上游 Scored/Total time，非 test 有 min time，ref/huge 有 Marks |
| 差分 | 独立 Spike 进程，普通指令流式比较，已完成的白名单平台操作显式同步；故障和协议失败不能自动跳过 |
| 单核 OS | yield-os 协作切换；thread-os 五线程定时抢占；M 态 CSR、异常、mret、机器定时中断和单核临界区原子交换 |
| 定时器 | 独立 MachineTimer；simulated 随仿真 tick，wall 随主机单调经过时间；频率和重装间隔可配置 |
| 运行限制 | AM CLI 默认三类预算均无限，显式参数才限制；自动测试用有限预算，无限循环 OS 用显式指令预算验收进展 |

当前可执行入口与细节以 [AM 运行说明](../workloads/am/README.md)、[AM kernels 目录](../third_party/am-kernels/README.md)和 [MicroBench README](../third_party/am-kernels/benchmarks/microbench/README.md)为准。

这些能力已经在当前 Machine 上交付，不再作为 P2 的前置开发任务。P2 不重做 AM 移植、克隆源码、Makefile 或 Spike adapter。改动模型装配时，复用现有 guest、ABI 和结果校验。

### 1.2 本阶段新增

- `System` 组件运行容器、模型私有事件和生命周期协调。
- 有方向的逐跳请求/响应协议，多实例互连与显式背压。
- board 级可独立装配的阻塞式 I/D L1。
- 现有 `Rv64Core` 迁移为 `BlockingCore`，新增 RV64I `FiveStageCore`。
- 分层性能统计、ROI 统计快照、停止/排空及最终一致数据观察。
- 在同一套 MicroBench 和定向探针上验证四种组合。

允许 Rust/Python 公开接口破坏性变更，不保留长期公开兼容层；迁移期可以保留仅供测试使用的旧实现。原有 guest ABI、功能结果与指定配置下的时间基线必须可追溯。

### 1.3 不在本阶段扩张的范围

不新增 L2、非阻塞 cache、MSHR 合并、store/writeback buffer、预取、多核一致性、A 扩展、DMA、详细 DRAM、图形设备、MMU、Linux、O3、checkpoint 或独立 warmup。

已实现的 M 态和定时器必须随 BlockingCore 迁移，不能因上述范围描述被删掉。FiveStageCore 首期以 RV64I + m5 的 benchmark 路径验收；将 OS、CSR 和异步中断扩展到五级核不作为 P2 完成条件，也不能静默接受不支持的配置。OS difftest 仍明确拒绝。

## 2. 验证原则

模型替换应只改变装配与模型参数，不要求 CPU 理解 cache 的内部状态，也不要求事件内核新增每个模型的专用分支。

三层证据分别负责不同问题：

1. **功能正确**：工作负载自检、架构提交和 Spike 差分。
2. **时序与协议正确**：小型定向实验、独立计算的完成 tick、所有权与资源约束断言。
3. **性能可解释**：同一 ELF、明确配置与测量区间下，记录 cycles、IPC、miss、stall 等指标。

不能用“MicroBench 跑完”代替协议验收，也不能把第一次实测的总 tick 直接当作正确性的证明。Marks 保留为上游展示结果，不作为框架正确性或真实硬件校准依据。

## 3. 时间、事件与 System

### 3.1 时间契约

- 全局时间用 `Tick`，地址、CPU 周期数和时钟配置用不同领域类型；换算检查正值和溢出。
- CPU 只在自身时钟边沿推进；M0 固定异步响应到边沿的对齐算法，以及恰好同 tick 到达时能否被消费。
- RAM、互连和 cache 延迟使用绝对 Tick；改变 CPU 时钟不能隐式改变其他模型。
- 明确请求、响应、retry、时钟推进、停止的事件相位。同 tick 的因果事件不能依赖偶然的注册顺序；禁止时间倒流和无限零时间循环。
- 保留单宿主线程。具名拓扑与仲裁优先序不变时，交换组件注册顺序不改变模拟结果。

现有 `src/sim.rs` 的通用事件队列是迁移基础。M0 需说明旧 BlockingCore 的响应处理与新时钟规则如何兼容；若某旧场景无法时间等价，必须记录具体差异及规则依据，不能悄悄重录期望值。

### 3.2 System 与模型的责任

`System` 持有组件、稳定 ID、连接和事件队列，负责调度、交付、运行限制和生命周期协调。缓存行、CPU 流水线、队列、仲裁与服务状态由对应模型拥有。

私有事件由模型解释。新增 Rust 模型允许增加注册、构造和 Python 类型，但不应修改内核的模型专用派发逻辑。用一个自带定时事件的测试组件验证这一点。

System 不提供隐藏的无限请求队列，不替 cache 关联上游需求与 fill/writeback，也不把 CPU、RAM 和设备状态揉进一个运行时状态机。

### 3.3 既有定时器迁移

保持 MachineTimer 的 MMIO 与 IRQ 接线分离、16 字节寄存器窗口及完成时读取语义。simulated 模式按仿真时间计数；wall 模式以开始执行时重置的主机单调时钟计数，不包含构建和加载时间，不负责把仿真节拍限制为实时速度。

确定性回放、trace 开关不变性与性能比较使用 simulated 模式。wall 模式只验证中断和线程进展，不要求固定中断数、固定提交点或逐次相同输出。显示、差分等宿主开销可能改变 wall 模式中断位置，不能承诺这类不变性。

## 4. 逐跳事务与互连

### 4.1 身份与所有权

区分架构访存、连接传输和目标内部服务。CPU load、cache fill、dirty writeback 是不同事务；内部传输不冒充额外的已提交指令。

请求包含类型化 ID、地址、操作和必要属性；写请求拥有数据。响应按 ID 匹配数据、写确认或故障，不依赖返回顺序。互连保留逻辑事务身份和返回上下文；cache 自己关联下游事务与上游请求。

```text
发送方持有请求
  ├─ Accepted：接收方保管状态，承担一次终结响应的责任
  └─ Rejected：完整请求返回，无访存副作用
                  ↓ 相邻接收方通知 retry
                再次尝试，仍可能拒收
```

接收判定可以同步返回，终结响应必须经事件交付，不能递归完成调用者请求。已接受的请求不取消；无法保管就拒收。下游拒收时，中间组件保留已接收的上游请求并等待 retry，上游不能重复发送。

retry 是唤醒，不保证下一次接收。等待登记、容量释放和撤回未接收请求必须避免丢唤醒、重复完成及迟到 retry 重新发出已撤回请求。

公共协议允许同一端口多个 outstanding 和乱序响应。BlockingCore 全核单请求、FiveStageCore IF/MEM 容量及 cache 单槽位均是模型约束，不能写成框架限制。

### 4.2 响应与故障

响应经过实际连接逐跳返回。禁止仅凭最初 source ID 直达 CPU，绕过中间互连的响应延迟。

P2 保证已接收事务的响应可靠交付，不实现有限响应队列和响应背压；接收方必须预留完成责任。互连有显式响应延迟，默认可为零；响应带宽竞争留待后续协议扩展。

拒收表达资源不足；非法地址、权限或操作以故障终结。二者均不允许部分写入。已接收事务恰好一次终结，包括故障路径。

### 4.3 地址属性与拓扑

平台声明区域、目标、cacheable 属性和合法操作。cache 覆盖范围是允许缓存的地址集合，必须属于平台可缓存 RAM，不能将 MMIO 提升为可缓存。

build 校验端口方向、缺失/重复连接、跨图句柄、请求拓扑环、各跳窗口与可达目标；串联路由中重复出现同一地址范围合法。完整 cache line 必须落在属性一致、完整可读写的同一 RAM 目标内。

Interconnect 支持多输入、多输出、串联及汇聚的请求 DAG。配置明确请求容量、转发/响应延迟和仲裁。继承旧基线的显式端口优先序及持续竞争轮转规则，不依赖组件注册次序。

M0 定义无缓冲兼容配置和有界缓冲配置的接收责任。P2 统计传输字节数，但不据此宣称已实现链路序列化、DRAM burst 或完整总线带宽模型。

## 5. 独立 I/D L1 与数据可见性

### 5.1 第一版 cache

- I/D 分离；默认每个 cache 为 32B line、32 sets、direct-mapped，共 1 KiB。
- 单槽位阻塞式：hit、miss、bypass 均占用唯一上游 outstanding 槽位。
- D-cache 为 write-back + write-allocate；I-cache 不接收写操作。
- lookup 延迟为正 Tick；fill 后安装、响应及 bypass 延迟在配置中显式定义。
- 无 hit-under-miss、写回缓冲、MSHR 合并；不支持自修改代码，不宣称已实现 FENCE.I。

| 访问 | 必须行为 |
| --- | --- |
| load/store hit | 查找完成时读取或修改 line；store 置 dirty 后确认 |
| clean miss | fill 成功后安装，完成原访问 |
| dirty miss | 先 writeback 并等确认，再 fill、安装和完成原访问 |
| store miss | fill 后合并 store、置 dirty、返回确认 |
| uncached/MMIO | 不命中、不分配 line，按配置延迟旁路并等待响应 |
| fill fault | 不安装无效 line，向上游返回需求访问故障并释放资源 |
| writeback fault | 已提交 store 的延迟写回失败属于不可恢复模型错误，保留行地址和事务上下文 |

槽位释放与 retry 的相位由 M0 固定。hit 延迟与接收间隔分别测量，不能把“1 tick hit”直接解释成每 tick 可接收一次访问。

### 5.2 写确认与观察

Accepted 只转移责任。RAM WriteAck 表示 RAM 已更新；cache WriteAck 表示最新数据已在该 cache 可见，不保证 backing RAM 更新。FENCE 等待本核前序访问完成，不等于 flush cache。

P2 限定单 CPU、无 DMA，无绕过 D-cache 的并发数据访问者；拒绝形成不一致缓存/旁路别名的装配。I/D 共享代码映射依赖不自修改代码约束。

程序退出不自动 flush。排空在途事务与写回全部脏行是两件事，脏而空闲的 cache 可以已排空。

成功停止并排空后，提供沿显式数据路径的只读一致观察：最新脏行优先于 RAM，不分配 line、不推进时间、不计入统计。用它检查最终数据，不把 RAM 旧值当架构结果；这不是完整的运行时 functional 模式。

## 6. CPU 迁移与第二种时序模型

### 6.1 BlockingCore

`Rv64Core` 迁移命名为 `BlockingCore`；暴露独立的 `ifetch`、`daccess` 请求端口，但仍全核至多一个未完成请求，分端口不引入执行重叠。

保留现有 ISA、ELF 初态、提交记录、m5 服务、机器态、UART、定时器和中断接线能力。中断在架构指令边界交付，在途数据访问先完成提交；m5 和有副作用的 MMIO 仍受不可推测提交约束。

用无缓存、无新增延迟的互连配置保留旧行为基线。旧端口和事务 ID 的结构变化采用明确归一化；不能只比较最终 PASS 而放弃时间与提交检查。

### 6.2 FiveStageCore

RV64I、单核、单发射、顺序提交，IF/ID/EX/MEM/WB；共享 ISA 功能语义、平台服务和架构提交结构，不复用 BlockingCore 的等待状态机。

- EX/MEM、MEM/WB 到 EX 转发，store 数据有明确的就绪和转发规则。
- load-use 根据实际数据就绪停顿，不假定恒定一周期。
- 分支在 EX 解决，默认预测不跳转；按取指代际冲刷错误路径。
- IF、MEM 各最多一个已接受请求，允许二者并存。
- D 等待阻塞 MEM 及上游，较老 WB 可以排空；最多保留一个已接受取指响应，不无限预取。
- I 等待只阻止新取指，较老指令仍可推进。
- 精确故障、m5、Exit 与 MMIO 副作用不能越过较老未决分支或故障；Exit 确认后的提交产生退出原因。

已接受的错误路径 fetch 必须完成传输并释放资源；CPU 丢弃结果及对应访问故障，cache 的查找、fill、替换和带宽占用仍保留并计数。事务 ID 匹配传输，代际判定路径，不能混用。未接收的错误路径请求可以撤回。

这种策略允许错误路径 cache 污染；它是明确的模型选择，不允许用于撤销 store/MMIO 副作用，也不能掩盖事务错配或资源泄漏。平台禁止从 MMIO 取指。

### 6.3 必须完成的组合

| CPU | 无 cache | 独立 I/D L1 |
| --- | --- | --- |
| BlockingCore | 迁移基线与 MicroBench 十项 test | cache 功能、数据可见性与相同十项 test |
| FiveStageCore | 流水线探针与相同十项 test | 流水线/cache 交互与相同十项 test |

四种组合使用同一份 benchmark ELF、平台布局和输入，不因换模型重新裁剪算法。共享配置类型与提交结果，替换内存路径不改 CPU。

BlockingCore 的 yield-os/thread-os 在无 cache 和 L1 路径保留进展验收；使用 simulated 定时器做确定性检查，wall 做有限预算下的独立进展测试。OS 不加入 Spike 矩阵，不要求两种内存路径产生相同调度点。

## 7. 运行控制、统计与现有实验入口

### 7.1 停止和排空

- AM CLI 继续默认不设指令、事件、tick 人为上限，未指定预算记录为 `null`；内部仍受 u64 表示范围约束。
- 自动验收显式配置有限预算；MicroBench 因预算耗尽停止不能视为通过，OS 的 bounded-progress 不伪装成正常退出。
- tick/event 上限是硬停止边界，不为排空偷偷越界；保留并标记未完成事务。P2 明确暂停状态的保管责任，不把现有一次性 run 声称为已支持续跑或 checkpoint。
- 正常退出/架构故障记录测量结束 tick，停止新工作；已承担事务及其必要子事务以独立预算排空。
- 分别报告运行结束 tick、排空结束 tick、排空是否成功；排空时间不计入程序区间。
- 只在成功排空后提供最终一致观察；排空期间的 writeback 错误不能被先前成功退出掩盖。

### 7.2 统计口径

按稳定实例路径提供有类型的结果，框架统计与模型专属统计分开。

| 层次 | 最小指标 |
| --- | --- |
| CPU | 测量 tick、周期、commits、CPI/IPC、fetch/data/load-use/control stall、分支与冲刷、丢弃取指响应 |
| Cache | 已接受需求访问、hit/miss、write allocation、writeback、fill/writeback bytes、占用时间和拒收 |
| 互连/RAM | 接收、拒收、retry、服务/转发次数与请求/响应延迟 |
| 平台 | console、ROI、timer_interrupts、退出/故障、运行预算、主机耗时 |

retry 不增加需求访问数，fill/writeback 不增加 CPU 指令数；被冲刷的已接受取指仍计入实际 cache 访问。M0 定义 stall 类别的重叠规则、零分母表示和同 tick 快照顺序。

沿用 work_begin/end 增加 ROI 分层快照，并明确边界外发起、边界内完成等跨区间事务的计数归属。现有 ROI commits 不含标记自身；周围 call/ret 等 wrapper 指令仍按真实执行计入。统计重置不能重置 cache 或流水线。

整个系统从固定初态和冷 cache 启动，但算法 ROI 前的 prepare、打印及前一子项会留下状态。保持 suite 顺序，记录测量前缀；不能把每项重新冷启动与上游顺序运行混为同一实验。

默认关闭详细 trace 时，已终结事务不应因调试用途无限保留；保留聚合计数及有界故障上下文。详细记录显式启用，长期运行的记录成本与模型必要状态分开说明。

### 7.3 复用 benchmark 平台

继续使用 `tools.am_benchmark`、Makefile、现有 m5 ABI 与 Spike adapter。新增 CPU/cache 装配选择及结果字段，不重复移植 guest。

保留 `--input`、`--display`、`--difftest`、三类可选预算、timer 参数和 `--output`。模型选择参数名称在接口阶段确定，未实现之前不在 README 中提供伪可执行命令。

`--display` 继续只控制仿真结束后的宿主打印，guest 格式化与 UART 访问始终执行；JSON 保留 console。benchmark uptime 始终是仿真时间，不能混入主机耗时或 thread-os 的 wall 定时器来源。

报告保留代码/工具链/Spike 版本、ELF hash、解析后配置、时间单位、初态、输入顺序、预算、ROI、退出原因和预期结果；增加时序模型与统计口径版本。为比较多个配置使用独立报告文件，避免默认 latest.json 被覆盖后失去依据。

## 8. 分阶段实施与验收

### M0：契约定案与现状冻结

形成可评审的设计记录：时间与相位、逐跳所有权与响应路径、私有事件、地址属性、写可见性、停止/排空/观察、cache/CPU 时序、统计与 ROI 边界。每项记录选择、简化、验证方法和重新评估条件。

冻结现有 Traffic/RAM、裸机 Exit、MicroBench test、平台服务、机器态和 timer 测试。保存 ELF/hash、配置、提交、tick、ROI 和 console；不将早期四项或性能输出恢复前的数字继续当当前基线。

当前检查入口：

```sh
devenv shell -- just check-mvp
devenv shell -- just check-am
```

参考执行器协议测试按现有 Spike 检查说明执行。保存实际命令、结果及跳过范围；已有 wall 模式不纳入固定 tick 基线。

**完成门槛**：现有能力列表与代码一致；每条框架保证有对应测试位置或待实现的具体场景；公开类型草案包含接线正反例。完成 M0 后再实施内核替换。

### M1：System 与现有模型迁移

迁移 Traffic、两种 RAM、Exit、SimpleUart、MachineTimer、实验控制器、BlockingCore 及其机器态。Board 构造与结果收集不能继续依赖单一具体 CPU 的专用流程。

保留旧实现为临时测试参考，逐项比对事务、提交和 tick；添加私有事件组件，验证扩展模型无需修改内核派发。迁移时落实运行预算与停止责任，避免引入默认限制。

**完成门槛**：旧无 cache 行为按约定归一化后一致；完整 MicroBench test/Spike、yield-os/thread-os 和平台错误路径通过；注册次序不改变 simulated 模式结果。

### M2：多跳互连与背压

先用 Traffic 验证串联、汇聚、有限缓冲、重试与逐跳响应，再接 CPU。

必测：两源经两级互连争用容量 1 RAM；中间已接收而下游拒收；retry 后再次拒收；无丢唤醒、无重复副作用、恰好一次完成。

同一请求端口至少同时持有两个已接收事务，分别路由到不同延迟目标，使后发先回；按 ID 检查结果。给返回路径增加已知延迟，完成时间必须增加相应数值。换 DualBankRam 验证资源并行。

**完成门槛**：多 outstanding 和乱序响应是实际验证过的公共能力；没有隐式无限队列或响应直达捷径；迁移基线仍通过。

### M3：独立 L1 与最终数据观察

先用 Traffic 覆盖状态机，再接 BlockingCore。必测 hit 延迟/接收间隔、clean/dirty miss、写回先于 fill、store miss、重试、fill fault、writeback fault、MMIO bypass 与 line 属性校验。

新增 `dcache_reuse`、`dcache_conflict`、`dirty_evict` 定向探针，核对明确访问序列的命中、替换、写回和数据。退出后同时检查 RAM 可能保留旧值，而一致观察返回新值。

**完成门槛**：BlockingCore 有/无 L1 均通过十项 test/Spike；OS 两条内存路径均有进展且设备不被缓存；cache 的错误和排空路径完整。

### M4：FiveStageCore 与四种组合

先无 cache，再接同一套 L1。新增短汇编探针：`alu_chain`/`alu_independent`、`load_use`、`branch_pattern`、`ifetch_squash`；避免编译器重排待测模式。

覆盖前递、store 数据依赖、数据尚未返回时的 load-use、分支遇 I miss、I/D 并存、D 等待时 WB 排空、错误路径取指故障、迟到 retry、退出/故障后的年轻写禁止。

专测错误路径污染：A 已在 I-cache，接受与 A 冲突的 B 取指后冲刷 B；B 的 fill 仍完成并替换 A，随后正确路径访问 A 发生 miss。核对 B 无提交、cache 计数和延迟保留、资源已释放。

**完成门槛**：四种组合运行同一 MicroBench test ELF，并通过提交差分。定向时序测试有独立依据，不要求两种 CPU tick 相同，也不假定单发射独立 ALU 流一定更快。

### M5：实验矩阵、接口切换与收尾

在现有入口增加 CPU/cache 选择和分层 ROI 报告。自动扫描 CPU 时钟、RAM 延迟、cache 延迟/容量，每次控制一个变量；只有满足明确前提的小场景才断言单调性能趋势。

清理旧公开 API 和临时测试适配器；更新 Python 导出、类型、原生 `.pyi`、安装后检查、装配示例、README 与迁移说明。保留原无 cache 时间基线，不用四种组合总耗时互相替代。

**完成门槛**：统一检查覆盖旧回归、多跳协议、cache、流水线和四种 benchmark 组合；每份报告可定位配置、ELF、版本、区间和结果。较大输入可按资源情况补充完整验收，但不替代十项 test，也不是重新完成 benchmark 接入。

## 9. 公共接口和类型要求

保留 Board 装配入口，build 后固化图。拟定迁移如下，具体签名在 M0 用类型正反例确定：

| 当前 | P2 方向 |
| --- | --- |
| Rv64Core | BlockingCore；新增 FiveStageCore |
| 单 requests 端口 | CPU 提供具名 ifetch/daccess；不是所有模型必须恰好两个端口 |
| Router | 多实例 Interconnect，显式窗口、容量和延迟 |
| 无独立 cache | Cache 提供 upstream TargetIn / downstream RequestOut |
| timer IRQ 接线 | 保留 InterruptOut/InterruptIn 与机器态能力校验 |
| memory::Machine | 逐阶段迁移到 System 与模型自有状态 |

Python 函数标注参数与返回值；每种新增接线有通过和应被拒绝的类型示例。安装 wheel 后执行 `mypy --strict`；修改原生绑定时同步 `.pyi`。跨图、属性和地址范围等实例约束由 build 校验。

构建、类型检查和测试复用 devenv 同步的 uv 环境。外部无类型输入从 `object` 校验收窄；类型豁免仅限具体行、错误码及理由。

## 10. P2 完成定义

1. System 与逐跳协议实际承载现有模型，多跳竞争验证所有权、重试和终结责任。
2. BlockingCore 保留当前 benchmark、机器态和 timer 能力；FiveStageCore 作为第二种独立时序模型交付。
3. CPU 与 I/D L1 独立装配，四种组合用同一 ELF 通过完整十项 test 和 Spike 差分。
4. 时钟、相位、cache、流水线规则经定向测试验证；固定时间断言有独立依据。
5. 写确认、脏数据、最终观察、故障与排空一致；预算语义不倒退。
6. 分层统计和 ROI 可解释，现有上游性能输出保留，实验记录能复现。
7. 类型与安装后检查通过，旧公开接口和迁移适配器清理完成，分阶段提交和文档齐全。

后续再按需求扩展缓存并发/L2、多核一致性、设备/DMA、MMU/全系统及更复杂 CPU。现有单核 M 态已是起点，不再重复规划为尚未实现；多核可见性、响应背压、运行时 functional 访问和 OS difftest 仍需各自的协议与验收。
