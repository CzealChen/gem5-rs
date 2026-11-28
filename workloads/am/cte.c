/* gem5-rs M-only cooperative CTE. See README.md for scope and provenance. */
#include <am.h>
#include <klib.h>
#include <klib-macros.h>

/* Check the upstream Context declaration against trap.S at compilation. */
_Static_assert(offsetof(Context, mepc) == 0, "mepc layout");
_Static_assert(offsetof(Context, mcause) == 8, "mcause layout");
_Static_assert(offsetof(Context, gpr) == 16, "GPR layout");
_Static_assert(offsetof(Context, mstatus) == 272, "mstatus layout");
_Static_assert(offsetof(Context, pdir) == 280 && sizeof(Context) == 288, "frame layout");

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
  assert(ctx->mcause == 11 && ctx->gpr[17] == (uintptr_t)-1);
  ctx->mepc += 4;
  Context *next = handler((Event){.event = EVENT_YIELD, .cause = ctx->mcause}, ctx);
  assert(next != NULL && ((uintptr_t)next & 15) == 0);
  return next;
}

void yield(void) {
  __asm__ volatile("li a7, -1; ecall" : : : "a7", "memory");
}
