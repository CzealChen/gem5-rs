/* Adapted from AbstractMachine riscv/npc/trap.S, commit
 * 068dc8cc150c63388fc5f457ff7b7dd70c5dd0f5.
 * Copyright (c) 2018-2021 Yanyan Jiang and Zihao Yu. MIT licensed;
 * full notice: ../../third_party/abstract-machine/LICENSE.
 * RV64 frame layout and stack switching corrected for gem5-rs Context. */
.section .text.gem5rs_trap
.balign 4
.global gem5rs_trap
gem5rs_trap:
  addi sp, sp, -288
  sd zero, 16(sp)
  sd ra, 24(sp)
  sd gp, 40(sp)
  sd tp, 48(sp)
  sd t0, 56(sp)
  sd t1, 64(sp)
  sd t2, 72(sp)
  sd s0, 80(sp)
  sd s1, 88(sp)
  sd a0, 96(sp)
  sd a1, 104(sp)
  sd a2, 112(sp)
  sd a3, 120(sp)
  sd a4, 128(sp)
  sd a5, 136(sp)
  sd a6, 144(sp)
  sd a7, 152(sp)
  sd s2, 160(sp)
  sd s3, 168(sp)
  sd s4, 176(sp)
  sd s5, 184(sp)
  sd s6, 192(sp)
  sd s7, 200(sp)
  sd s8, 208(sp)
  sd s9, 216(sp)
  sd s10, 224(sp)
  sd s11, 232(sp)
  sd t3, 240(sp)
  sd t4, 248(sp)
  sd t5, 256(sp)
  sd t6, 264(sp)
  addi t0, sp, 288
  sd t0, 32(sp)
  csrr t0, mepc
  sd t0, 0(sp)
  csrr t0, mcause
  sd t0, 8(sp)
  csrr t0, mstatus
  sd t0, 272(sp)
  sd zero, 280(sp)
  mv a0, sp
  call gem5rs_irq_handle
  mv sp, a0
  ld t0, 0(sp)
  csrw mepc, t0
  ld t0, 272(sp)
  csrw mstatus, t0
  ld ra, 24(sp)
  ld gp, 40(sp)
  ld tp, 48(sp)
  ld t0, 56(sp)
  ld t1, 64(sp)
  ld t2, 72(sp)
  ld s0, 80(sp)
  ld s1, 88(sp)
  ld a0, 96(sp)
  ld a1, 104(sp)
  ld a2, 112(sp)
  ld a3, 120(sp)
  ld a4, 128(sp)
  ld a5, 136(sp)
  ld a6, 144(sp)
  ld a7, 152(sp)
  ld s2, 160(sp)
  ld s3, 168(sp)
  ld s4, 176(sp)
  ld s5, 184(sp)
  ld s6, 192(sp)
  ld s7, 200(sp)
  ld s8, 208(sp)
  ld s9, 216(sp)
  ld s10, 224(sp)
  ld s11, 232(sp)
  ld t3, 240(sp)
  ld t4, 248(sp)
  ld t5, 256(sp)
  ld t6, 264(sp)
  ld sp, 32(sp)
  mret
