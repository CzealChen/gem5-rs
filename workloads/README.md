# RV64I 裸机参考工作负载

在仓库根目录进入 `devenv shell`，然后重建 ELF 并运行 Spike：

```sh
./workloads/build.sh
python tools/spike_trace.py workloads/build/rv64i.elf workloads/build/loop.elf
GEM5RS_ELF_DIR=workloads/build python -B -m unittest workloads.test_spike_trace
mypy --strict -m tools.spike_trace -m workloads.test_spike_trace
```

参考工具链：`riscv64-none-elf-gcc` 16.2.0、GNU Binutils 2.46，Spike 包版本 `1.1.0-unstable-2026-10-20`（命令自报 `1.1.1-dev`）。构建使用 RV64I/LP64、medany 和 freestanding C；指令宽度固定为 32 位。`build.sh` 接受输出目录参数，`CC_RV64` 可指定交叉 GCC。

## 预期结果

| ELF | 程序自检 | Spike 在退出写前的提交数（上述工具链） |
| --- | --- | ---: |
| `rv64i.elf` | `x0` 恒为零，BSS 初值为零；有符号读取得 `-128`，无符号读取得 `0x80`、`0xff80`、`0xffffff80`；字节、半字、字和双字访存得到 `scratch = 0x04030201`；RV64 word 运算验证符号扩展与回绕；倒数循环求和得 `10`。 | 73 |
| `loop.elf` | BSS 中 `result` 初值为零，循环调用独立编译的 `term` 四次，累加值依次为 `2、9、21、38`。 | 122 |

两个 ELF 的入口均为 `0x80000000`，工作负载使用从该地址起的 4 KiB RAM，共用启动代码将栈设为 `0x80000ff0`。首个向 `0x100000` 发出的 32 位写报告运行结果，两程序均应写入 `0`。`riscv64-none-elf-readelf -l -A` 可查看装载段和 ISA 属性，`riscv64-none-elf-objdump -d workloads/build/loop.elf` 可看到循环中的 `jalr … <term>`。

## 提交轨迹

Spike 运行其 reset ROM（ROM 使用 Zicsr），启动代码随后初始化 ROM 修改的 `t0`、`a0` 和 `a1`，使 Spike 与从 ELF 入口直接启动的机器进入同一初始状态。参考轨迹从 `_start` 开始，按 PC、指令字和整数寄存器写回逐条比较，在首个退出写之前结束。Spike 用一页 RAM 捕获退出写，报告对应指令位置与写入值。

集成调用：`run_spike(Path(...))` 取得参考轨迹，`compare_trace(reference, actual)` 接收含退出写提交的模拟器轨迹，返回首个差异诊断或 `None`。最后一条退出写按 PC 和指令字检查；将平台报告的退出结果与参考轨迹的 `exit_code` 单独比较。

## AM MicroBench

AM 使用独立的 16 MiB 平台、上游 Makefile、console 与显式 m5 指令扩展。
`devenv shell -- just check-am` 验证四个 test 算法；详见 [AM 入口](am/README.md)。
此入口不使用本页的离线 Spike CLI，也不改变原有 4 KiB 裸机基线。
