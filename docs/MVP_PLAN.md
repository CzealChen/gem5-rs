# gem5-rs MVP：用同一套 Rust 模型装配两台机器

**执行目标：** 从全新克隆进入 `devenv shell`，用带静态类型的 Python 装配并运行两个实验：两个流量源争用一个内存端口；一个顺序 RV64I 核运行裸机 ELF。两个实验共用事件内核、路由、RAM 和结果采集。把 RAM 换成另一个 Rust 实现后，原 Python 工作负载和请求方继续可用，数据结果一致，完成时间随模型变化。

沿用 [PLAN.md](PLAN.md) 的 M0–M3 语义。实现顺序以第一个可运行的无 CPU 实验为分界：先交付 Python 到 Rust 的完整路径，再加指令执行。以下名称和命令是实施目标。

## 装配者看到什么

```python
from gem5rs import Address, Board, FixedRam, RunLimit, Tick, Traffic, Write

def experiment() -> tuple[int, ...]:
    board = Board()
    src = board.add(Traffic([Write(at=Tick(0), addr=Address(0), data=b'A')]))
    ram = board.add(FixedRam(size_bytes=64, latency=Tick(10), capacity=1))
    board.connect(src.requests, ram.input)
    result = board.build().run(RunLimit(events=100))
    return tuple(int(tx.completed_at) for tx in result.transactions)
```

这里预期得到 `(10,)`。`Board.add` 按描述类型返回带具体端口的句柄；`connect` 接受请求输出和目标输入。`Address` 和 `Tick` 是不同的名义类型。示例用直连说明最短路径；完整机器把源接到路由的独立入口，把每个地址区间映射到一个下游目标。`build` 固化拓扑；`run` 返回具名的停止原因、模拟时间、事务记录和统计。Python 只在装配、启动和取结果时进入 Rust，事件循环由 Rust 推进。

## 三个值得保留的接口

| 模块 | 调用者需要知道什么 | 可替换或可验证的行为 |
|---|---|---|
| 事件内核 | 向稳定目标 ID 预约事件，按预算运行，得到停止原因。时间是整数 tick，同 tick 以阶段优先级、插入序号排序。 | 定时器与请求源使用同一个接口。模型自行解释事件内容；新增模型的私有事件留在模型内。测试倒流、加法溢出和零时间循环耗尽预算。 |
| 事务传输 | 请求带 ID、来源、地址和 Fetch、Read 或拥有数据的 Write。接收转移所有权；拒收归还原请求，等待 retry；终结响应匹配 ID。 | CPU 和流量源是同一接缝上的两个请求方适配器。RAM 和退出 MMIO 是两个目标适配器。请求方只看接收、唤醒和响应，目标的排队方式属于其实现。 |
| 机器装配 | Python 声明模型和接线，Rust 校验后构造可运行机器。事件内核接受已装配的目标，不解析 CPU 指令。 | 第一版的插拔是编译期加入 Rust 模型、运行前由 Python 选择实例。新 RAM 实现事务接口，添加类型化构造入口及绑定声明；内核、CPU 和已有调用者保持原样。 |

先用单 Cargo package 内的 `sim`、`memory`、`models`、`platform` 模块；Python 原生绑定放在可选 feature 下。核心的 Rust 测试独立于 Python 解释器运行。对扩展性的证据采用一个实际替换：第二个 RAM 适配器按地址区分两个 bank，独立空闲 bank 可并行完成。两源访问不同 bank 时，与单服务槽 RAM 相比读取值相同、完成时间不同。接缝的测试使用公开事务和运行结果，而不是窥视队列长度。

## 实施顺序

1. **M0：先让一张时间表可执行。** 建 Cargo 工程、确定性事件队列、状态分派与运行预算。先写手算期望，再实现。两个定时事件逆序预约仍按时刻触发；同 tick 同优先级保持插入顺序。重复运行的事件轨迹一致。
2. **M1：从 Python 跑通双源争用。** 加流量源、地址路由、容量受限的 RAM 和退出 MMIO。固定案例：A 和 B 在 tick 0 同时到达；仲裁以配置中显式端口优先次序打破初次平局，持续竞争按轮转服务。容量 1、服务时间 10。A 写入一个字节，B 读取同一地址。A 在 0 被接收，B 的请求被退回；10 时先完成 A 的写入并释放容量，再通知 B 重试；B 在 10 重交获准、在 20 读到新字节。用交换实例注册次序、保留端口次序的变体复验。另测 retry 后再次拒收、非法区间写完整失败、MMIO 副作用一次和两个 bank 的替换案例。每个已接收请求恰有一个终结响应；可靠事件负责交付响应。容量、仲裁、读写可见时刻属于模型契约。
3. **在 M1 一起交付类型化装配。** 用 PyO3 暴露私有原生扩展，maturin 生成 wheel，公开 Python 包交付具名类型、原生 `.pyi` 和 `py.typed`。针对 wheel 中的包运行示例与 `mypy --strict`。类型反例把地址传给延迟，或把请求输出连到另一个输出，应当在指定行得到相应诊断；修正那一行后检查通过。运行时测试覆盖跨图句柄、端口重复绑定、必需端口缺失和地址区间重叠，错误指出实例和端口。公共类型与运行时图校验各有独立测试。
4. **M2：用同一接缝接入 RV64I。** 实现 little-endian 单核顺序模型与 ELF64 `PT_LOAD` 初始化、BSS 清零、入口和栈。ISA 模块给出执行结果或访存意图，CPU 模块等待取指、数据和写确认，再提交指令。实现基础整数、word 运算、跳转、分支、访存及 FENCE；错误按 `docs/PLAN.md` 记录 PC、原因和相关地址。交付链接脚本、手写汇编及带函数和循环的 freestanding C。固定 `-march=rv64i -mabi=lp64`，测试退出使用一个有地址与宽度定义的 MMIO 写；待写确认和该指令提交后报告退出。
5. **M3：把功能正确与时间正确分开核对。** 对同一裸机指令流，与锁定版本 Spike 对齐起始状态，按 PC、指令和寄存器写回寻找第一次差异；在平台专用退出操作之前截取可比较区间。另用手算时间表核对整段取指、执行和数据等待，用两组 RAM 延迟运行相同 ELF，核对架构结果一致与 tick 差值。用固定配置重跑并比较规范化轨迹。为非法指令、未对齐、越界、溢出和运行预算分别留回归用例。现有可选 gem5 环境供查阅上游行为，MVP 的具体延迟以公布的模型契约为准。

## 交付给下一位执行者的复现入口

在仓库根目录运行 `devenv shell -- just check-mvp`；已进入 `devenv shell` 时运行 `just check-mvp`。进入环境时，uv 根据 `uv.lock` 同步开发依赖。检查顺序如下：

- `cargo test --locked`：事件、事务和 ISA 的独立行为用例通过。
- `maturin build` 后用 `uv pip install --target` 将本次 wheel 安装到临时目录，在该目录用项目环境运行严格类型正例、指定错误反例与绑定运行测试。上面的单源示例、双源争用和适配器替换都经公开装配接口运行。
- 重建两个 ELF，运行 Python 装配的机器，输出退出原因、提交数、最终 tick、访存统计和首个轨迹差异（如有）。静态期望、手算依据和复现命令随测试一起交付。

流量实验中，两个源在 tick 0 写入各自 bank、tick 21 读取：单服务槽依次在 tick 10、20 完成写入，tick 31、41 完成读取；双 bank 在 tick 10 完成两次写入、tick 31 完成两次读取，两者均读回 `A`、`B`。

裸机实验的时间从 tick 0 开始。每条指令先等待 RAM 取指，再花 2 tick 执行。普通读写另等一次 RAM 服务，末尾退出写等待 1 tick，并在确认时提交。若提交数为 `N`、普通数据访问数为 `D`、RAM 延迟为 `L`，结束时间为 `N × (L + 2) + D × L + 1`。两次运行使用 3、7 tick 的 RAM 延迟，结束时间相差 `4 × (N + D)`。工作负载与 Spike 参考轨迹见 [裸机工作负载](../workloads/README.md)。

## 一手依据

- [gem5 v26 timing protocol](https://github.com/gem5/gem5/blob/v26.0.0.0/src/mem/protocol/timing.hh)：请求与响应分别有重试机制。MVP 的可靠响应事件是本项目明确选择的简化。
- [Python 类型系统规范](https://typing.python.org/en/latest/spec/type-system.html)、[mypy 严格模式](https://mypy.readthedocs.io/en/stable/command_line.html#cmdoption-mypy-strict)、[类型信息的分发](https://typing.python.org/en/latest/spec/distributing.html)：静态检查调用方式，运行时校验具体拓扑；从已安装包验证类型可见性。
- [PyO3：类型声明](https://pyo3.rs/main/python-typing-hints)、[maturin：项目布局](https://www.maturin.rs/project_layout.html)：Python 门面、类型声明与 Rust 扩展一起分发。
- [RISC-V 非特权 ISA 规范](https://docs.riscv.org/reference/isa/unpriv/unpriv-index.html)、[Spike](https://github.com/riscv-software-src/riscv-isa-sim)：分别作为指令语义依据和功能执行参照。
