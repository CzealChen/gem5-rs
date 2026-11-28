#include <am.h>
#include <klib.h>
extern char _heap_start[], _heap_end[], _console[];
Area heap;
extern uint64_t m5_rpns(void);
extern void m5_exit(uint64_t delay);
extern void m5_fail(uint64_t delay, uint64_t code);
extern int main(const char *args);
void putch(char ch) { *(volatile unsigned char *)_console = (unsigned char)ch; }
void halt(int code) {
  if (code == 0) m5_exit(0);
  else m5_fail(0, (uint64_t)(int64_t)code);
  for (;;) __asm__ volatile ("j .");
}
bool ioe_init(void) { return true; }
void ioe_read(int reg, void *buf) {
  if (reg != AM_TIMER_UPTIME) panic("unsupported IOE read");
  ((AM_TIMER_UPTIME_T *)buf)->us = m5_rpns() / 1000;
}
void ioe_write(int reg, void *buf) {
  (void)reg; (void)buf;
  panic("unsupported IOE write");
}
void _trm_init(void) {
  heap = (Area){_heap_start, _heap_end};
  halt(main("test"));
}
