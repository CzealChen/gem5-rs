# gem5-rs

用 Python 装配 Rust 仿真模型，观察同一份工作负载在不同内存配置下的结果与耗时。

```sh
devenv shell -- just check-mvp
```

从[双请求源实验](tests/e2e/traffic.py)开始：先预测两个请求的完成时刻，再比较单服务槽与双 bank 内存。[裸机工作负载](workloads/README.md)给出了 RV64I 汇编和 C 实验的输入、预期结果与 Spike 对照方法。
