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

以下程序均已导入，尚未接入 gem5-rs 运行入口。所列依赖说明后续适配方向，
不表示当前模拟器已提供这些功能。

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
| [yield-os](kernels/yield-os/) | 协作式任务切换示例；需 CTE 上下文与 `yield` 支持 |
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
