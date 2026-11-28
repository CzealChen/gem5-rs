# AM 实施进度

## 当前交付与顺序调整

用户明确选择：先在当前阻塞 CPU 实现 benchmark，guest 使用 Makefile，
不以 P2 重构作为本次前置条件。长期模块边界仍保留，后续迁移复用这些能力。

已完成：

- A0 独立 Spike 能力探针：真实单步、完整 PC/GPR 同步、访存记录、EBREAK
  原因/PC 获取。见 `tools/spike/README.md`；它不是 AM 的生产差分驱动。
- 独立实验控制器及共享 m5 ABI，当前阻塞核在不可推测提交边界执行服务。
  扩展默认关闭；未知编码、非法参数、嵌套/不配对 ROI 明确失败。
- SimpleUart 输出子集，通过正常 Target 接口接受写入，接受时记录字节，延迟
  后响应；拒收不输出，EOT 不退出，非法宽度/读取/取指拒绝。
- Python 具名平台、ELF 栈堆布局校验、类型化 console/ROI 结果及三类预算。
  失败退出与正常退出区分，退出指令先提交再停止，普通 EBREAK/ECALL 语义不变。
- 保留上游 AM Makefile 的 guest 构建适配。固定来源、逐文件许可说明、RV64I
  软件乘除、启动/IOE/klib 子集、构建缓存和独立实验报告。
- hello/time/ROI 小程序及 queen/qsort/sieve/md5 的原始 test 输入与自检。
  实际运行四项全部通过；执行顺序明确为 queen、qsort、sieve、md5。
- `just check-am`：release wheel 安装后 strict mypy、正反配置场景、预算、
  轨迹开关不变性、不同 RAM 延迟下同一 ELF 的自检；CI 加入统一检查。

## m5 ABI

固定依据为 gem5 `9d368896ce119c17e1289dcbb18f33624724b843` 的 m5ops.h、
RISC-V m5op.S 和 m5ops.isa。指令为 `0x7b | (operation << 25)`；a0/a1 传参，
所有成功操作均写回 a0。`src/m5.rs` 是操作表权威，guest 汇编通过生成及测试核对。

| 操作 | 编号 | 指令 | 约束 |
| --- | --- | --- | --- |
| rpns | 0x07 | 0x0e00007b | tick 的物理时间换算为 ns，向下取整 |
| exit | 0x21 | 0x4200007b | delay=0，a0 写零 |
| fail | 0x22 | 0x4400007b | delay=0，完整 64 位失败码，a0 写零 |
| work_begin | 0x5a | 0xb400007b | threadid=0，不嵌套 |
| work_end | 0x5b | 0xb600007b | threadid=0，id 配对 |

ROI commits 不包含两条标记；标记之间的 call/ret 等普通指令计入。
当前 ROI 提供 tick 和 commits；事务统计是端到端口径。

## 验证边界和后续阶段

本次交付是“当前阻塞核 + Makefile + 四项自检”，不是原计划 A0–A5 全部完成。
A3 的正式流式 Spike 协议、故障注入及平台操作同步尚未接入 AM；报告明确写
`difftest: false`。A5 的 FiveStage/L1 四种组合仍依赖 P2。完整 suite/train、
ROI 事务计数快照、最终一致数据观察也未交付。

当前 trace=False 只省略返回给 Python 的轨迹，旧 Machine 仍在运行期间保留
事务记录。流式观察与有界日志需要后续实现，不能据此宣称支持 train 规模。

复现：

```
devenv shell -- just check-mvp
devenv shell -- just check-am
devenv shell -- just check-spike-probe
```

产物和用法见 `workloads/am/README.md`。CI 配置已加入；本地验证不代表已运行
远端 CI。原有 4 KiB/Exit 基线仍独立保留。
