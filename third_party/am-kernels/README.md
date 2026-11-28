# AM kernels 在 gem5-rs 中的源码与接入状态

这里保存完整的 [NJU-ProjectN/am-kernels](https://github.com/NJU-ProjectN/am-kernels)
源码快照，固定版本为 `9c433f7cf4b4e05d09543bf7e269b23502d1b2b6`。
`benchmarks/`、`kernels/`、`tests/` 均纳入本仓库版本管理；`target/` 仅用于构建和缓存。
上游根目录说明保留在 [README](README)，来源与许可说明见 [third_party README](../README.md)。

## Benchmarks

| 目录 | 用途 | gem5-rs 接入状态 |
| --- | --- | --- |
| [microbench](benchmarks/microbench/README.md) | 排序、搜索、压缩、MD5 等 10 项基准 | 已接入；完整 `test` 自检和 Spike difftest 通过，`train/ref/huge` 已开放选择并验证构建和限额启动，尚未完整验收 |
| [coremark](benchmarks/coremark/README.md) | CoreMark 整数处理器基准 | 已导入，尚未接入构建与验收 |
| [dhrystone](benchmarks/dhrystone/) | Dhrystone 整数程序基准 | 已导入，尚未接入构建与验收 |

## Kernels

以下程序均已导入；`yield-os` 已接入，其余尚未接入通用运行入口。
表中注明其用途与适配依赖。

| 目录 | 用途与适配依赖 |
| --- | --- |
| [hello](kernels/hello/) | 简单输出程序；需对接启动、控制台和退出 |
| [demo](kernels/demo/) | 蚂蚁、生命游戏、时钟等演示；按所选程序对接输出和计时等接口 |
| [blockchain](kernels/blockchain/) | C++ 区块链与哈希计算示例；需检查 C++ 运行时及库函数依赖 |
| [bad-apple](kernels/bad-apple/) | 视频播放演示；需图形输出和计时 |
| [litenes](kernels/litenes/) | NES 模拟器；需图形、输入等设备适配 |
| [slider](kernels/slider/) | 图片显示程序；需图形设备 |
| [snake](kernels/snake/) | 贪吃蛇游戏；需图形、输入和计时 |
| [typing-game](kernels/typing-game/) | 打字游戏；需图形、键盘和计时 |
| [yield-os](kernels/yield-os/) | 已接入单核机器态协作切换；限额验证 A/B 交替 |
| [thread-os](kernels/thread-os/) | 多线程调度示例；需上下文切换、多处理器及原子操作接口 |
| [nemu](kernels/nemu/) | 在 AM 上构建和运行 NEMU 的入口；上游要求 NEMU 平台及外部 `NEMU_HOME` |

## Tests

| 目录 | 用途 | gem5-rs 接入状态 |
| --- | --- | --- |
| [cpu-tests](tests/cpu-tests/) | 算术、控制流、内存及小程序正确性测试 | 已导入，尚未接入逐项构建和结果判定 |
| [alu-tests](tests/alu-tests/) | 主机生成算术测试 C 源码，再编译执行 | 已导入，尚未接入生成与验收流程 |
| [am-tests](tests/am-tests/) | AM 输出、时钟、图形、音频、中断、虚拟内存和多处理器接口测试 | 已导入；需按实际实现的 AM 接口逐项接入 |

## 当前运行入口

在 gem5-rs 仓库根目录执行：

```sh
devenv shell
maturin develop --release
python -m tools.am_benchmark --input test --display
```

默认运行 MicroBench，difftest 默认关闭，加 `--difftest` 开启。
输入规模、内存和预算参数详见 [MicroBench 说明](benchmarks/microbench/README.md)。
`just check-am` 执行当前 AM 验收流程，包括 Spike difftest。
该入口中的 `--workload hello` 是项目自有的
[平台冒烟程序](../../workloads/am/hello.c)，不是这里的 `kernels/hello`。

当前 AM 平台实现位于 [workloads/am](../../workloads/am/README.md)。
`third_party/abstract-machine` 保留所需上游头文件及 Makefile；`amdev.h` 声明
设备接口，不会自动创建设备。实际使用的 klib 函数由项目平台代码实现。

## 低成本检查 CPU 指令依赖

在仓库根目录执行：

```sh
devenv shell -- python -m tools.am_audit
```

此命令排除 NEMU，使用各 kernel 的上游 Makefile 和现有 RV64I/LP64、`-O2`
平台配置，只编译 C/C++ 目标文件，不链接、不仿真，也不生成图片或音视频资源。
检查直接复用 blocking CPU 的指令语义函数，避免维护另一份支持指令表。
结果写入 `target/am-audit/report.json`，各程序的编译日志也在同一目录。

当前锁定版本的检查结果：10 个 kernel、31 个目标文件、12,398 条静态指令，
全部编译成功，未发现不支持的指令。这里包括可能被链接器删除的代码，指令数
不是动态执行次数。`external_dependencies` 列出尚未链接的外部符号，既包括
已有平台函数，也包括待实现接口，不能直接当成缺失函数清单。

主要的后续接入工作包括：

- `yield-os` 的 `cte_init/kcontext/yield`；`thread-os` 还需要多处理器、
  中断控制与 `atomic_xchg`。这些接口的未来实现可能引入新的指令需求。
- 图形、音频、输入程序的 `ioe_read/ioe_write` 设备功能与资源打包。
- 部分程序的 `malloc/free/rand/memcmp/memmove/sprintf` 等 klib 功能。
  RV64I 下的乘除法会调用软件辅助函数，不意味着必须添加 M 扩展。

直接运行也可以发现问题：CPU 遇到不支持的指令会返回 `IllegalInstruction`、
故障 PC 和编码，再用 objdump 定位。但需要先链接成功，并且每次只覆盖实际
执行路径，遇到第一个错误就停止。本次另将上游 `kernels/hello` 与现有平台链接
并实际运行，成功退出（748 条指令）；这是一项独立冒烟验证，尚未增加通用 kernel
运行入口。静态检查不代表其他 kernel 已可运行，也不覆盖尚未链接的 AM/klib
实现、资源汇编或未来更换编译 ISA 后的结果。

## OS 程序：yield-os 已接入，thread-os 待接入

在仓库根目录、devenv 环境中执行（首次或更新原生代码后先安装）：

```sh
maturin develop --release
python -m tools.am_benchmark --workload yield-os --display --output target/am-results/yield-os.json
```

程序使用上游未修改的 `kernels/yield-os/yield-os.c`。默认预算为 300 万条退休
指令、2400 万事件、100 亿 ticks；已验证输出 `ABABAB`。程序是无限循环，报告
`instruction_limit` 和 `bounded-progress` 表示在预算内观察到了多次切换，
不是 guest 正常退出。少于四个字符、非交替输出、异常或其他停止原因均不通过。
可用现有预算参数调整运行长度；仅增大指令预算时也应注意 tick/event 上限。

CLI 为此 workload 自动开启 `Rv64Core(machine_mode=True)`。该选项默认关闭。
当前实现单核、仅机器态、同步异常与 Zicsr：直接模式 `mtvec`、`mepc/mcause/mtval`、
`mstatus/mscratch`、固定 `misa/mhartid`、六种 CSR 指令和 `mret`。
异常指令不退休；AM 保存/恢复全部 GPR、任务栈和返回 PC，`yield` 通过 `ecall`
进入调度程序。Context 与汇编布局通过 C 静态断言验证。

trap 汇编参考固定版本 AbstractMachine 的 MIT 实现，保留版权与许可来源，
并修正上下文布局、恢复原 SP、按处理程序返回值切换任务栈。`kcontext` 与 CTE
是完整的平台实现，没有链接上游空模板。详见 [平台说明](../../workloads/am/README.md)。

`thread-os` 仍未接入。它的 `func()` 不主动 `yield`，因此下一步需要定时器事件、
中断使能/挂起状态与递送、`iset`、CPU 身份和原子交换接口，才能验证五线程进展。
可以先做单核抢占；真正多核还需要共享内存原子事务及多核装配支持。
当前机器态不实现异步中断、U/S 态或分页，不能据此宣称支持完整操作系统。

现有 Spike difftest 尚未覆盖 CSR 与 guest trap-handler 执行，`yield-os --difftest`
及 `machine_mode=True` 与 difftest 的组合会在运行前明确拒绝，不会跳过比较后报通过。
MicroBench 的默认 RV64I 运行和 difftest 验收保持不变。
