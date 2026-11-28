/* Finite acceptance guest for single-hart AM atomics and asynchronous return. */
#include <am.h>
#include <klib.h>
#include <klib-macros.h>

static volatile unsigned interrupts;
static int cell;

static Context *interrupt(Event event, Context *ctx) {
  assert(event.event == EVENT_IRQ_TIMER);
  assert(!ienabled());
  assert((ctx->mstatus & 8) == 0 && (ctx->mstatus & 128) != 0);
  interrupts++;
  return ctx;
}

static void entry(void) {
  iset(true);
  register uintptr_t saved __asm__("s2") = UINT64_C(0xfedcba9876543210);
  register uintptr_t temporary __asm__("t3") = UINT64_C(0x123456789abcdef0);
  while (interrupts < 3) {
    __asm__ volatile("" : "+r"(saved), "+r"(temporary));
  }
  assert(saved == UINT64_C(0xfedcba9876543210));
  assert(temporary == UINT64_C(0x123456789abcdef0));
  assert(ienabled());
  iset(false);
  halt(0);
}

int main(const char *args) {
  (void)args;
  assert(cpu_count() == 1 && cpu_current() == 0);
  iset(false);
  assert(atomic_xchg(&cell, 7) == 0 && !ienabled());
  iset(true);
  assert(atomic_xchg(&cell, 9) == 7 && ienabled());
  iset(false);
  assert(atomic_xchg(&cell, 0) == 9 && !ienabled());
  cte_init(interrupt);
  mpe_init(entry);
  return 1;
}
