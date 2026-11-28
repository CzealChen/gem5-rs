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

以下程序均已导入；`yield-os` 和单核 `thread-os` 已接入，其余尚未接入通用运行入口。
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
| [thread-os](kernels/thread-os/) | 已接入单核定时器抢占；五线程进展验收，可选仿真时钟或主机墙钟 |
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

- OS 的单核 CTE、中断控制与 `atomic_xchg` 现已接入；多核版本仍需额外的
  hart 与共享内存原子性支持。
- 图形、音频、输入程序的 `ioe_read/ioe_write` 设备功能与资源打包。
- 部分程序的 `malloc/free/rand/memcmp/memmove/sprintf` 等 klib 功能。
  RV64I 下的乘除法会调用软件辅助函数，不意味着必须添加 M 扩展。

直接运行也可以发现问题：CPU 遇到不支持的指令会返回 `IllegalInstruction`、
故障 PC 和编码，再用 objdump 定位。但需要先链接成功，并且每次只覆盖实际
执行路径，遇到第一个错误就停止。本次另将上游 `kernels/hello` 与现有平台链接
并实际运行，成功退出（748 条指令）；这是一项独立冒烟验证，尚未增加通用 kernel
运行入口。静态检查不代表其他 kernel 已可运行，也不覆盖尚未链接的 AM/klib
实现、资源汇编或未来更换编译 ISA 后的结果。

## OS 程序：单核 yield-os 与 thread-os

在仓库根目录、devenv 环境中执行（首次或更新原生代码后先安装）：

```sh
maturin develop --release
python -m tools.am_benchmark --workload yield-os --instruction-budget 3000000 --display --output target/am-results/yield-os.json
```

程序使用上游未修改的 `kernels/yield-os/yield-os.c`。上述示例显式限制为 300 万条退休
指令；已验证输出 `ABABAB`。所有预算默认无限，程序是无限循环，报告
`instruction_limit` 和 `bounded-progress` 表示在预算内观察到了多次切换，
不是 guest 正常退出。少于四个字符、非交替输出、异常或其他停止原因均不通过。
可用预算参数调整运行长度；未指定的预算不限制运行。

CLI 为此 workload 自动开启 `Rv64Core(machine_mode=True)`。该选项默认关闭。
当前实现单核、仅机器态、同步异常与 Zicsr：直接模式 `mtvec`、`mepc/mcause/mtval`、
`mstatus/mscratch`、固定 `misa/mhartid`、六种 CSR 指令和 `mret`。
异常指令不退休；AM 保存/恢复全部 GPR、任务栈和返回 PC，`yield` 通过 `ecall`
进入调度程序。Context 与汇编布局通过 C 静态断言验证。

trap 汇编参考固定版本 AbstractMachine 的 MIT 实现，保留版权与许可来源，
并修正上下文布局、恢复原 SP、按处理程序返回值切换任务栈。`kcontext` 与 CTE
是完整的平台实现，没有链接上游空模板。详见 [平台说明](../../workloads/am/README.md)。

`thread-os` 已使用原版程序接入单核抢占式调度。`cpu_count()` 返回 1，
`cpu_current()` 返回 0，`mpe_init()` 在当前核启动入口，`atomic_xchg()` 保存并
关闭中断、交换值后恢复原状态。它不提供跨核原子性，也没有引入 A 扩展。

### Timer 参数与两种时钟

```sh
# 跟随仿真时间；默认 1 MHz 计数频率、1 ms 调度周期
python -m tools.am_benchmark --workload thread-os --instruction-budget 6000000 --timer-clock simulated --display --output target/am-results/thread-simulated.json

# 跟随主机墙钟；这里选择已验证的 10 ms 调度周期
python -m tools.am_benchmark --workload thread-os --instruction-budget 6000000 --timer-clock wall --timer-frequency-hz 1000000 --timer-interval-us 10000 --display --output target/am-results/thread-wall.json
```

| 参数 | 含义 | 默认值 |
| --- | --- | --- |
| `--timer-clock` | `simulated` 跟随仿真 tick；`wall` 跟随主机单调时钟经过的时间 | `simulated` |
| `--timer-frequency-hz` | `mtime` 每秒增加多少个计数，决定计时分辨率 | `1000000`（1 MHz） |
| `--timer-interval-us` | guest 每次处理定时器中断时重设的间隔，决定调度周期 | `1000`（1 ms） |

1 MHz 表示每 1 μs 增加一次计数；不是每微秒触发一次中断。
例如 1 MHz + 10 ms 周期，对应 `mtimecmp = mtime + 10000`。
仿真模式把频率换算成整数 tick 周期，不能精确表示的频率会被拒绝，不会偷偷舍入。
墙钟模式按主机单调时间计算，不受日历时间或 NTP 调整影响，也不强制仿真速度与
主机速度相等。计时从每次仿真运行开始，编译和 ELF 装载不计入 timer 时间。

墙钟模式下，主机负载、日志和追踪开销会影响中断出现在哪条指令之间，不能要求
重复运行或开关 trace 后结果完全相同。回归使用仿真模式。两种模式默认周期均为
1 ms；墙钟模式中周期过短可能让固定指令预算主要消耗在抢占上，可像示例一样
增加周期，或调整预算。不会因此放宽五线程进展的验收条件。

`thread-os` 默认不限制运行；上述示例显式限制为 600 万条退休指令。验收要求
无故障地达到指令上限、至少五次 timer 中断、A–E 每个线程至少输出两条完整的
`Thread-X on CPU #0`。允许预算恰好截断末尾一条合法输出前缀；其他错误行会失败。
它仍是无限循环，结果为 `bounded-progress`，不是正常退出或完整 OS 支持。
已验证仿真模式五个线程各输出三次；墙钟示例也通过五线程进展验收。

JSON 的 `configuration.timer_clock/timer_frequency_hz/timer_interval_us` 记录参数，
`statistics.timer_interrupts` 记录实际递送次数，`console` 保存输出。
仅切换时钟来源可以复用同一个 ELF；改变计数频率或 guest 重设间隔可能重新编译。

Timer 使用两个 64 位 MMIO 寄存器：基址 `0x02000000` 的 `mtime`（只读）与
`+8` 的 `mtimecmp`（读写）。这不是完整 CLINT 地址布局。MTIP 是
`mtime >= mtimecmp` 的电平；中断屏蔽不会丢弃 pending，重设比较值才会清除它。
CPU 在当前访存完成后的指令边界，检查 `mstatus.MIE`、`mie.MTIE` 和 `mip.MTIP`，
进入 cause 为 `(1 << 63) | 7` 的 trap。中断本身不退休，返回 PC 不加 4。
时间按需计算，不为每个计数创建事件；目前不支持 WFI/休眠核心唤醒。

对照 gem5：其固定版本的 [CLINT](https://github.com/gem5/gem5/blob/9d368896ce119c17e1289dcbb18f33624724b843/src/dev/riscv/clint.cc)
接收 RTC 信号增加 `mtime` 并与 `mtimecmp` 比较。这里参考同样的比较语义，
另提供可选主机墙钟源；没有复制 gem5 的设备实现。

现有 Spike difftest 尚未覆盖 CSR 与 guest trap-handler 执行，两个 OS workload 的 `--difftest`
及 `machine_mode=True` 与 difftest 的组合会在运行前明确拒绝，不会跳过比较后报通过。
MicroBench 的默认 RV64I 运行和 difftest 验收保持不变。
