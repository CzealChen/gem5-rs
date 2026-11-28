/* gem5-rs M-only CTE. See README.md for scope and provenance. */
#include <am.h>
#include <klib.h>
#include <klib-macros.h>

/* Check the upstream Context declaration against trap.S at compilation. */
_Static_assert(offsetof(Context, mepc) == 0, "mepc layout");
_Static_assert(offsetof(Context, mcause) == 8, "mcause layout");
_Static_assert(offsetof(Context, gpr) == 16, "GPR layout");
_Static_assert(offsetof(Context, mstatus) == 272, "mstatus layout");
_Static_assert(offsetof(Context, pdir) == 280 && sizeof(Context) == 288, "frame layout");

#ifdef GEM5RS_TIMER
#include "timer-config.h"
extern volatile uint64_t _timer[];
static void timer_rearm(void) {
  volatile uint64_t *timer = _timer;
  uint64_t now = timer[0];
  assert(now <= UINT64_MAX - GEM5RS_TIMER_RELOAD);
  timer[1] = now + GEM5RS_TIMER_RELOAD;
}
#endif

static Context *(*handler)(Event, Context *);
extern void gem5rs_trap(void);

bool cte_init(Context *(*callback)(Event, Context *)) {
  assert(callback != NULL);
  handler = callback;
  __asm__ volatile("csrw mtvec, %0" : : "r"(gem5rs_trap) : "memory");
  return true;
}

static void task_returned(void) { panic("kernel task returned"); }

Context *kcontext(Area stack, void (*entry)(void *), void *arg) {
  uintptr_t top = (uintptr_t)stack.end & ~(uintptr_t)15;
  assert(top >= (uintptr_t)stack.start + sizeof(Context));
  Context *ctx = (Context *)(top - sizeof(Context));
  memset(ctx, 0, sizeof(*ctx));
  ctx->mepc = (uintptr_t)entry;
  ctx->mstatus = (3 << 11) | (1 << 7);
  ctx->gpr[1] = (uintptr_t)task_returned;
  ctx->gpr[2] = top;
  ctx->gpr[10] = (uintptr_t)arg;
  return ctx;
}

Context *gem5rs_irq_handle(Context *ctx) {
  assert(handler != NULL);
  Event event = {.cause = ctx->mcause};
  if (ctx->mcause == 11 && ctx->gpr[17] == (uintptr_t)-1) {
    ctx->mepc += 4;
    event.event = EVENT_YIELD;
#ifdef GEM5RS_TIMER
  } else if (ctx->mcause == (UINT64_C(1) << 63 | 7)) {
    timer_rearm();
    event.event = EVENT_IRQ_TIMER;
#endif
  } else {
    panic("unexpected machine trap");
  }
  Context *next = handler(event, ctx);
  assert(next != NULL && ((uintptr_t)next & 15) == 0);
  return next;
}

void yield(void) {
  __asm__ volatile("li a7, -1; ecall" : : : "a7", "memory");
}

#ifdef GEM5RS_TIMER
bool ienabled(void) {
  uintptr_t status;
  __asm__ volatile("csrr %0, mstatus" : "=r"(status));
  return (status & 8) != 0;
}

void iset(bool enabled) {
  if (enabled) __asm__ volatile("csrsi mstatus, 8" : : : "memory");
  else __asm__ volatile("csrci mstatus, 8" : : : "memory");
}

int cpu_count(void) { return 1; }
int cpu_current(void) { return 0; }

int atomic_xchg(int *addr, int value) {
  uintptr_t saved;
  __asm__ volatile("csrrci %0, mstatus, 8" : "=r"(saved) : : "memory");
  int old = *(volatile int *)addr;
  *(volatile int *)addr = value;
  if (saved & 8) __asm__ volatile("csrsi mstatus, 8" : : : "memory");
  return old;
}

bool mpe_init(void (*entry)(void)) {
  timer_rearm();
  __asm__ volatile("csrs mie, %0" : : "r"(UINT64_C(1) << 7) : "memory");
  entry();
  panic("single-hart entry returned");
}
#endif
