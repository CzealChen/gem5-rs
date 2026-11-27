    .option norvc
    .option norelax
    .text
    .globl workload_main
    .type workload_main, @function
workload_main:
    li      a0, 1
    addi    zero, zero, 7
    bne     zero, t0, fail

    lla     t0, scratch
    ld      t1, 0(t0)
    bnez    t1, fail
    li      t1, -128
    sd      t1, 0(t0)
    lb      t2, 0(t0)
    bne     t1, t2, fail
    lh      t2, 0(t0)
    bne     t1, t2, fail
    lw      t2, 0(t0)
    bne     t1, t2, fail
    lbu     t2, 0(t0)
    li      t3, 128
    bne     t2, t3, fail
    lhu     t2, 0(t0)
    lwu     t3, 0(t0)
    slli    t3, t3, 48
    srli    t3, t3, 48
    bne     t2, t3, fail
    lwu     t3, 0(t0)
    srli    t3, t3, 31
    li      t2, 1
    bne     t2, t3, fail

    # Construct 0x0000000004030201 with three store widths.
    sw      zero, 0(t0)
    sw      zero, 4(t0)
    sb      t2, 0(t0)
    li      t2, 2
    sb      t2, 1(t0)
    li      t2, 0x0403
    sh      t2, 2(t0)
    ld      t2, 0(t0)
    li      t3, 0x04030201
    bne     t2, t3, fail

    li      t2, 1
    slliw   t2, t2, 31
    lui     t3, 0x80000
    bne     t2, t3, fail
    addiw   t2, t2, -1
    srliw   t3, t3, 1
    addw    t3, t3, t3
    addiw   t3, t3, -1
    bne     t2, t3, fail
    li      t2, 0
    li      t3, 4
1:
    add     t2, t2, t3
    addi    t3, t3, -1
    bnez    t3, 1b
    li      t3, 10
    bne     t2, t3, fail
    li      a0, 0
fail:
    ret
    .size workload_main, . - workload_main

    .bss
    .balign 8
scratch:
    .zero 8
