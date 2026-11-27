    .option norvc
    .option norelax
    .section .text.start,"ax",@progbits
    .globl _start
    .type _start, @function
_start:
    # Match the direct ELF-entry machine after Spike's reset ROM hands off.
    li      t0, 0
    li      a0, 0
    li      a1, 0
    lla     sp, __stack_top
    call    workload_main
    lui     t0, 0x100
    sw      a0, 0(t0)
1:
    j       1b
    .size _start, . - _start
