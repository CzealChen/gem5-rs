# Spike A0 能力探针

运行：`devenv shell -- just check-spike-probe`。

这是 AM 计划 A0 的接口可行性验证，不是 A3 的生产参考进程，不接受 workload
或任意 skip 请求。默认 Rust 构建无需 Spike/C++；具名检查入口才编译独立 C++
进程并从 Rust 对比它的输出。A3 完成后应将这里的测试迁入正式 adapter，删除
原型入口，避免维护两套协议。

## 固定来源

- Spike：`riscv-software-src/riscv-isa-sim@d140cbc0ae4ef4fc64ad323f3f175ff15f6dca56`。
- Nix 源码 NAR：`sha256-chyTl7v7pZUGcRczzrawLOYxSYbbk7eWohcbduU7VBU=`。
- 工具来自 `devenv.lock` 锁定的 nixpkgs；版本为
  `1.1.0-unstable-2026-10-20`。使用 recipe 默认 configure 参数；Darwin 的
  SoftFloat 编译诊断补丁见 `devenv.nix`，不改变指令语义。
- 探针编译：C++20，`-Wall -Wextra -Werror`，链接该环境的 `libriscv`。
  不从系统 PATH 混入另一套头文件或库。更新 lock 时必须重新核验版本和此探针。

## 已由真实库验证的接口

`processor_t("RV64I", "M", ...)` 配合最小 `simif_t` RAM 实现；直接设置
`get_state()->pc` 和 `XPR.write()` 后调用 `step(1)`，没有执行 reset ROM。
RAM 先清零，指令按小端写入，所有 GPR 明确初始化，x0 保持零。

测试覆盖 addi、sd、ld、add；Rust 用现有 ISA 语义逐步比较 next PC 和全部
32 个 GPR，第二次创建进程验证可重复性。中间替换全部 GPR/PC 后继续独立执行。
`enable_log_commits()` 开启实际的 `log_mem_read/write`：检查地址、宽度和 store
值；load 日志的 value 不被假设为返回数据，实际加载结果从架构 GPR 获取。

`EBREAK` 的 `mcause=3` 与 `mepc` 可读取。此结果只证明故障信息可获取，尚未
证明生产协议能可靠区分所有故障：A3 必须识别当次 trap（包括连续相同原因），
禁止只比较 mcause 新旧值。该探针的日志写到 stderr，stdout 仅供测试读取。

## 下一阶段协议约束（尚未实现）

生产协议使用版本化具名消息，显式握手版本、Spike commit、ISA 和状态范围。
初始化传入经过校验的 RAM 布局、ELF 段及 BSS、入口和全部 GPR；参考端不得
隐式执行启动 ROM。单步返回执行 PC/指令、post PC/GPR、访存效果或 fault。
同步只替换 PC/GPR，验证 x0 和 PC 对齐，不修改 RAM。结束必须回收子进程。

驱动在发送 step 前决定已成功提交的平台操作是否属于白名单；不得在 Spike
执行失败后再同步补救。通信应有消息大小与时间上限，未知版本、ISA、无效状态、
提前 EOF、超时、崩溃均单独报错。本探针固定输入，不具备上述生产防护，也不
构成 AM ELF、m5 ABI、周期、缓存或四种 CPU 组合通过的证据。
