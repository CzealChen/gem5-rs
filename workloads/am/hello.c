#include <am.h>
#include <klib.h>
extern uint64_t m5_rpns(void);
extern void m5_work_begin(uint64_t, uint64_t);
extern void m5_work_end(uint64_t, uint64_t);
extern uint64_t __udivdi3(uint64_t, uint64_t);
extern uint64_t __umoddi3(uint64_t, uint64_t);
extern int64_t __divdi3(int64_t, int64_t);
extern int64_t __moddi3(int64_t, int64_t);
int main(const char *args) {
  assert(strcmp(args, "test") == 0);
  volatile uint64_t a = 0xfedcba9876543210ULL, b = 0x12345;
  assert(a * b == 0x999999999999ae50ULL);
  assert(a / b == 0xe0004fa01c4dULL);
  assert(a % b == 0x10a4f);
  assert(__udivdi3(UINT64_MAX, 0) == UINT64_MAX);
  assert(__umoddi3(UINT64_MAX, 0) == UINT64_MAX);
  assert(__udivdi3(UINT64_MAX, (uint64_t)1 << 63) == 1);
  assert(__umoddi3(UINT64_MAX, (uint64_t)1 << 63) == INT64_MAX);
  assert(__divdi3(INT64_MIN, -1) == INT64_MIN);
  assert(__divdi3(-100, 7) == -14);
  assert(__moddi3(-100, 7) == -2);
  uint64_t start = m5_rpns();
  m5_work_begin(1, 0);
  putch('H'); putch('i'); putch('\n');
  m5_work_end(1, 0);
  assert(m5_rpns() > start);
  printf("time=%llu\n", (unsigned long long)io_read(AM_TIMER_UPTIME).us);
  return 0;
}
