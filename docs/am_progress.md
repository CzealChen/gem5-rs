# AM 实施进度

按 `am_benchmark_plan.md` 的依赖顺序交付；“完成一项”不代表所在阶段已验收。

## A0 已完成的独立准备

- `workloads/am/sources.json` 固定四个上游源码版本。尚未导入 AM 算法和 klib，
  也未声称完成逐文件许可审计、移植补丁或 guest 构建缓存。
- 真实 Spike 能力原型及逐条 Rust ISA 对比：见 `tools/spike/README.md`。
- Python `BareMetalPlatform` 和 `am_test_platform()`：RAM、console、栈、时间
  单位、CPU/PIO 延迟及三类预算集中定义；校验地址重叠、溢出、栈堆和时间换算。
  `linker_script()` 从同一个对象派生布局；P2 装配与参考初始化还没有消费者。
- Rust `m5::Operation` 单一 ABI 表及 guest stub 生成函数；普通 RV64I 默认仍
  拒绝所有五种 m5 指令。未将这些操作接入旧 CPU 或旧 Machine。

## 固定的 m5 ABI

依据 gem5 `9d368896ce119c17e1289dcbb18f33624724b843` 的
`include/gem5/asm/generic/m5ops.h`、`util/m5/src/abi/riscv/m5op.S`、
`src/arch/riscv/isa/formats/m5ops.isa`。RV64 指令为
`0x7b | (operation << 25)`；参数按 a0/a1 传递，结果总是写回 a0。

| 操作 | 编号 | 指令 | 参数 | 结果/限制 |
| --- | --- | --- | --- | --- |
| rpns | 0x07 | 0x0e00007b | 无 | 模拟纳秒，向下取整 |
| exit | 0x21 | 0x4200007b | a0=delay | 仅 delay=0，a0 写零 |
| fail | 0x22 | 0x4400007b | a0=delay, a1=code | 仅 delay=0，保留 64 位 code，a0 写零 |
| work_begin | 0x5a | 0xb400007b | a0=id, a1=threadid | threadid=0，不嵌套，a0 写零 |
| work_end | 0x5b | 0xb600007b | a0=id, a1=threadid | 配对 id，threadid=0，a0 写零 |

以上表格冻结目标语义，当前仅实现 ABI 定义。实验控制器、参数验证、不可推测
服务、ROI 快照与 console 副作用需要 A1/A2 的独立测试后才算实现。

## 尚未通过的阶段门槛

A0 仍依赖 P2 M0 契约定案；生产参考协议及统一初始化由 A3 收敛，原型不能替代。
A1 依赖 P2 M1 的 System/组件/提交接口；A2 还依赖 M2 的逐跳端口与互连。
当前仓库仅有旧 Machine，不在其上添加临时 AM runtime。
A4 的四个算法和 A5 的四种模型组合均未实现或验收。

复现现有检查：

```
devenv shell -- just check-spike-probe
devenv shell -- just check-mvp
```

第二条包含安装 wheel 后的配置测试和 `mypy --strict`。完整 AM 的 CI、guest
编译、流式差分和组合报告需在后续阶段加入，当前检查不可称为 AM benchmark 通过。

## 实施顺序调整：当前阻塞核优先

用户后续明确选择先在当前 Blocking CPU 接入 benchmark，guest 使用 Makefile，
不再以 P2 M0–M4 为本次前置门槛。原计划的长期模块方向保留；本次实现复用当前
Source/MemoryTarget 接口，不新增 CPU 或事件内核副本。

当前已实现共享 m5 ABI、独立实验控制器、提交时退出与 ROI、正常事务路径上的
输出设备及类型化 Board 配置/结果。设备新增通用 accepted 通知，以区分字符
产生与响应完成；既有 RAM 完成时读写语义不变。指令、tick 和 event 预算分别
报告，m5_fail 保留完整失败码，普通 EBREAK/ECALL 不变。
